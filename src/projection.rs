//! Media-derived projections, read from the English Wikipedia articles on the
//! 2026 Senate and House elections (CC BY-SA 4.0). Wikipedia editors compile
//! race calls from news organisations, so this layer is a *projection of
//! winners*, kept apart from the official vote counts. It is volunteer-edited
//! and can lag or be wrong; the page labels it that way and flags any state
//! where it disagrees with an official feed.
use crate::model::Party;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Instant;
use tokio::sync::Mutex;

pub const SENATE_TITLE: &str = "2026_United_States_Senate_elections";
pub const HOUSE_TITLE: &str = "2026_United_States_House_of_Representatives_elections";

#[derive(Clone, Debug, Default)]
pub struct ChamberTotals {
    pub dem: Option<u32>,
    pub rep: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Projection {
    /// state code -> projected Senate winner party. Absent = not called.
    pub senate: BTreeMap<String, Party>,
    /// state code -> (Democratic seats called, Republican seats called).
    /// Absent = Wikipedia has no seat counts for the state yet.
    pub house: BTreeMap<String, (u32, u32)>,
    /// Chamber composition after the election as stated in each infobox.
    pub senate_total: ChamberTotals,
    pub house_total: ChamberTotals,
    /// Newest revision timestamp of the two articles.
    pub revision_time: DateTime<Utc>,
    pub senate_rev: u64,
    pub house_rev: u64,
}

#[derive(Clone, Debug)]
pub enum ProjResult {
    Ok(Projection),
    Failed(String),
}

fn party_of(name: &str) -> Party {
    let n = name.to_ascii_lowercase();
    if n.starts_with("democratic") || n.contains("democratic") {
        Party::Dem
    } else if n.starts_with("republican") {
        Party::Rep
    } else {
        Party::Other
    }
}

/// `names` maps full state names (as in the articles) to codes.
pub fn parse_senate(wt: &str, names: &BTreeMap<String, String>) -> Result<BTreeMap<String, Party>, String> {
    let start = wt.find("== Race summary ==").ok_or("bagian Race summary tidak ditemukan")?;
    let end = wt[start..].find("\n== Alabama ==").map(|i| start + i).ok_or("bagian negara bagian tidak ditemukan")?;
    let sec = &wt[start..end];
    let head = Regex::new(r"! \[\[2026 United States Senate (?:special )?election in ([^|\]]+)\|").unwrap();
    let aye = Regex::new(r"Party stripe\|([^}]+)\}\}\{\{aye\}\}").unwrap();
    let mut out = BTreeMap::new();
    let mut rows = 0;
    for chunk in sec.split("\n|-") {
        let Some(h) = head.captures(chunk) else { continue };
        let Some(code) = names.get(h[1].trim()) else { continue };
        rows += 1;
        if let Some(a) = aye.captures(chunk) {
            out.insert(code.clone(), party_of(&a[1]));
        }
    }
    if rows < 30 {
        return Err(format!("hanya {rows} baris Senat terbaca; format artikel berubah?"));
    }
    Ok(out)
}

pub fn parse_house(wt: &str, names: &BTreeMap<String, String>) -> Result<BTreeMap<String, (u32, u32)>, String> {
    let start = wt.find("Per state ===").ok_or("tabel per negara bagian tidak ditemukan")?;
    let end = wt[start..].find("\n|}").map(|i| start + i).ok_or("akhir tabel tidak ditemukan")?;
    let sec = &wt[start..end];
    let name = Regex::new(r"!\[\[#([^|\]]+)\|").unwrap();
    let num = Regex::new(r"(\d+)").unwrap();
    let mut out = BTreeMap::new();
    let mut rows = 0;
    for chunk in sec.split("\n|-") {
        let Some(h) = name.captures(chunk) else { continue };
        let Some(code) = names.get(h[1].trim()) else { continue };
        rows += 1;
        let cells: Vec<&str> = chunk.lines().filter(|l| l.starts_with('|')).collect();
        // cells: total, dem seats, dem change, rep seats, rep change
        if cells.len() < 5 {
            continue;
        }
        let cell = |i: usize| -> Option<u32> {
            let c = cells[i].trim_start_matches('|');
            let c = c.rsplit('|').next().unwrap_or(c);
            num.captures(c).and_then(|m| m[1].parse().ok())
        };
        let (d, r) = (cell(1), cell(3));
        if d.is_some() || r.is_some() {
            out.insert(code.clone(), (d.unwrap_or(0), r.unwrap_or(0)));
        }
    }
    if rows < 45 {
        return Err(format!("hanya {rows} baris DPR terbaca; format artikel berubah?"));
    }
    Ok(out)
}

pub fn parse_totals(wt: &str) -> ChamberTotals {
    let re = |k: &str| Regex::new(&format!(r"\|\s*{k}\s*=\s*(?:'''|)(\d+)")).unwrap();
    let get = |k: &str| re(k).captures(wt).and_then(|c| c[1].parse().ok());
    ChamberTotals { dem: get("seats_after1"), rep: get("seats_after2") }
}

#[derive(Deserialize)]
struct Api {
    query: Q,
}
#[derive(Deserialize)]
struct Q {
    pages: Vec<P>,
}
#[derive(Deserialize)]
struct P {
    title: String,
    revisions: Option<Vec<R>>,
}
#[derive(Deserialize)]
struct R {
    revid: u64,
    timestamp: String,
    slots: Slots,
}
#[derive(Deserialize)]
struct Slots {
    main: Main,
}
#[derive(Deserialize)]
struct Main {
    content: String,
}

pub fn parse_api(
    body: &[u8],
    names: &BTreeMap<String, String>,
) -> Result<Projection, String> {
    let api: Api = serde_json::from_slice(body).map_err(|e| format!("respons Wikipedia tidak dikenali: {e}"))?;
    let mut sen = None;
    let mut hou = None;
    for p in api.query.pages {
        let rev = p.revisions.and_then(|mut r| r.pop()).ok_or("artikel tanpa revisi")?;
        let ts = DateTime::parse_from_rfc3339(&rev.timestamp).map_err(|e| e.to_string())?.with_timezone(&Utc);
        if p.title.contains("Senate") {
            sen = Some((rev, ts));
        } else if p.title.contains("House") {
            hou = Some((rev, ts));
        }
    }
    let (s, st) = sen.ok_or("artikel Senat tidak ada")?;
    let (h, ht) = hou.ok_or("artikel DPR tidak ada")?;
    Ok(Projection {
        senate: parse_senate(&s.slots.main.content, names)?,
        house: parse_house(&h.slots.main.content, names)?,
        senate_total: parse_totals(&s.slots.main.content),
        house_total: parse_totals(&h.slots.main.content),
        revision_time: st.max(ht),
        senate_rev: s.revid,
        house_rev: h.revid,
    })
}

struct Cache {
    at: Instant,
    value: ProjResult,
}
static CACHE: OnceLock<Mutex<Option<Cache>>> = OnceLock::new();
const MIN_REFRESH_SECS: u64 = 90;

pub async fn fetch(client: &reqwest::Client, names: &BTreeMap<String, String>) -> ProjResult {
    let slot = CACHE.get_or_init(|| Mutex::new(None));
    let mut g = slot.lock().await; // single flight
    if let Some(c) = g.as_ref() {
        if c.at.elapsed().as_secs() < MIN_REFRESH_SECS {
            return c.value.clone();
        }
    }
    let url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&prop=revisions&rvprop=ids|timestamp|content&rvslots=main&formatversion=2&format=json&titles={SENATE_TITLE}|{HOUSE_TITLE}"
    );
    let res = async {
        let r = client.get(&url).send().await.map_err(|e| format!("gagal menghubungi Wikipedia: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("Wikipedia membalas HTTP {}", r.status().as_u16()));
        }
        let b = r.bytes().await.map_err(|e| e.to_string())?;
        parse_api(&b, names)
    }
    .await;
    let value = match res {
        Ok(p) => ProjResult::Ok(p),
        Err(e) => match g.as_ref() {
            // keep a recent good copy for a short while, otherwise report the failure
            Some(Cache { at, value: ProjResult::Ok(p) }) if at.elapsed().as_secs() < 15 * 60 => ProjResult::Ok(p.clone()),
            _ => ProjResult::Failed(e),
        },
    };
    *g = Some(Cache { at: Instant::now(), value: value.clone() });
    value
}
