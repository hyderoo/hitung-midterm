//! Reader for the "Media Export" JSON that Enhanced Voting election night
//! reporting sites publish (used by the Virginia and Georgia sites).
//! Only the fields we need are declared, so serde skips the rest (precinct
//! level detail) without building it in memory.
use crate::config::SourceCfg;
use crate::model::*;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::sync::Mutex;

#[derive(Deserialize)]
struct Export {
    #[serde(rename = "createdAt")]
    created_at: String,
    results: Results,
    #[serde(rename = "localResults", default)]
    local: Vec<Local>,
}
#[derive(Deserialize)]
struct Results {
    #[serde(rename = "ballotItems", default)]
    items: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    id: String,
    name: String,
    #[serde(rename = "ballotOptions", default)]
    options: Vec<Opt>,
}
#[derive(Deserialize)]
struct Opt {
    name: String,
    #[serde(rename = "voteCount", default)]
    votes: u64,
    #[serde(rename = "politicalParty")]
    party: Option<String>,
}
#[derive(Deserialize)]
struct Local {
    #[serde(rename = "ballotItems", default)]
    items: Vec<LItem>,
}
#[derive(Deserialize)]
struct LItem {
    id: String,
    #[serde(rename = "precinctsReporting")]
    reporting: Option<u32>,
    #[serde(rename = "precinctsParticipating")]
    participating: Option<u32>,
}

/// Drops every "(...)" tag, e.g. "Jon Ossoff (I) (Dem)" -> "Jon Ossoff".
pub fn clean_name(raw: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for ch in raw.chars() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_contest(state: &str, item: &Item, district: Option<u32>, label: String, local: &[Local]) -> Contest {
    let mut candidates = Vec::new();
    let mut write_in = 0u64;
    for o in &item.options {
        if o.name.trim().eq_ignore_ascii_case("write-in") {
            write_in += o.votes;
        } else {
            candidates.push(Candidate {
                name: clean_name(&o.name),
                party: Party::classify(o.party.as_deref()),
                votes: o.votes,
            });
        }
    }
    candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
    let (mut r, mut p, mut seen) = (0u32, 0u32, false);
    for l in local {
        for li in l.items.iter().filter(|x| x.id == item.id) {
            if let Some(pp) = li.participating {
                seen = true;
                p += pp;
                r += li.reporting.unwrap_or(0);
            }
        }
    }
    Contest {
        state: state.to_string(),
        label,
        district,
        candidates,
        write_in,
        precincts: if seen && p > 0 { Some((r, p)) } else { None },
        polling_pct: None,
    }
}

pub fn parse_export(bytes: &[u8], cfg: &SourceCfg) -> Result<StateFeed, String> {
    let ex: Export = serde_json::from_slice(bytes).map_err(|e| format!("format file tidak dikenali: {e}"))?;
    let source_time: DateTime<Utc> = DateTime::parse_from_rfc3339(&ex.created_at)
        .map_err(|e| format!("createdAt tidak valid: {e}"))?
        .with_timezone(&Utc);
    let re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let mut senate = None;
    let mut house = Vec::new();
    for item in &ex.results.items {
        if cfg.senate_exact.iter().any(|n| n == &item.name) {
            senate = Some(build_contest(&cfg.state, item, None, "Senat AS".into(), &ex.local));
        } else if let Some(c) = re.captures(&item.name) {
            if let Ok(d) = c[1].parse::<u32>() {
                house.push(build_contest(&cfg.state, item, Some(d), format!("Distrik {d}"), &ex.local));
            }
        }
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed {
        state: cfg.state.clone(),
        source_label: cfg.label.clone(),
        human_url: cfg.human_url.clone(),
        export_url: cfg.export_url.clone(),
        source_time,
        senate,
        house,
        refresh_note: None,
    })
}

#[derive(Deserialize)]
struct NcRow {
    cnm: String,
    bnm: String,
    pty: String,
    vct: String,
    prt: String,
    ptl: String,
}

/// NC State Board of Elections statewide results file (results_0.txt): a JSON
/// array with one row per candidate per contest. The file carries no embedded
/// timestamp, so the HTTP Last-Modified header is used as the source time.
pub fn parse_ncsbe(bytes: &[u8], cfg: &SourceCfg, source_time: DateTime<Utc>) -> Result<StateFeed, String> {
    let rows: Vec<NcRow> = serde_json::from_slice(bytes).map_err(|e| format!("format file tidak dikenali: {e}"))?;
    let re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let mut order: Vec<String> = Vec::new();
    let mut by: HashMap<String, Vec<&NcRow>> = HashMap::new();
    for r in &rows {
        if !by.contains_key(&r.cnm) {
            order.push(r.cnm.clone());
        }
        by.entry(r.cnm.clone()).or_default().push(r);
    }
    let build = |name: &str, district: Option<u32>, label: String| -> Contest {
        let rs = &by[name];
        let mut candidates = Vec::new();
        let mut write_in = 0u64;
        for r in rs.iter() {
            let v: u64 = r.vct.parse().unwrap_or(0);
            if r.bnm.to_ascii_lowercase().starts_with("write-in") {
                write_in += v;
            } else {
                candidates.push(Candidate { name: clean_name(&r.bnm), party: Party::classify(Some(&r.pty)), votes: v });
            }
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        let precincts = match (rs[0].prt.parse::<u32>(), rs[0].ptl.parse::<u32>()) {
            (Ok(a), Ok(b)) if b > 0 => Some((a, b)),
            _ => None,
        };
        Contest { state: cfg.state.clone(), label, district, candidates, write_in, precincts, polling_pct: None }
    };
    let mut senate = None;
    let mut house = Vec::new();
    for name in &order {
        if cfg.senate_exact.iter().any(|n| n == name) {
            senate = Some(build(name, None, "Senat AS".into()));
        } else if let Some(c) = re.captures(name) {
            if let Ok(d) = c[1].parse::<u32>() {
                house.push(build(name, Some(d), format!("Distrik {d}")));
            }
        }
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed {
        state: cfg.state.clone(),
        source_label: cfg.label.clone(),
        human_url: cfg.human_url.clone(),
        export_url: cfg.export_url.clone(),
        source_time,
        senate,
        house,
        refresh_note: None,
    })
}

/// Texas Secretary of State results page (static HTML, one table per contest).
/// The page states its own "Updated" time in local time, but the HTTP
/// Last-Modified header is used as the source time (they agree).
pub fn parse_txsos(html: &str, cfg: &SourceCfg, source_time: DateTime<Utc>) -> Result<StateFeed, String> {
    let re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let row = Regex::new(r#"<td class="party">([^<]*)</td><td class="candidate">([^<]*)</td><td class="ballot">([^<]*)</td>"#).map_err(|e| e.to_string())?;
    let head = Regex::new(r#"<div class="election">([^<]*)</div>"#).map_err(|e| e.to_string())?;
    let heads: Vec<(usize, usize, String)> = head.captures_iter(html).map(|c| (c.get(0).unwrap().start(), c.get(0).unwrap().end(), c[1].trim().to_string())).collect();
    if heads.is_empty() {
        return Err("format halaman tidak dikenali: tidak ada kontes".into());
    }
    let mut senate = None;
    let mut house = Vec::new();
    for (i, (_, end, name)) in heads.iter().enumerate() {
        let stop = heads.get(i + 1).map(|h| h.0).unwrap_or(html.len());
        let seg = &html[*end..stop];
        let is_sen = cfg.senate_exact.iter().any(|n| n == name);
        let dist = re.captures(name).and_then(|c| c[1].parse::<u32>().ok());
        if !is_sen && dist.is_none() {
            continue;
        }
        let polling_pct = Regex::new(r"Polling Locations Reporting: (\d+)%").ok().and_then(|p| p.captures(seg)).and_then(|c| c[1].parse::<u8>().ok());
        let mut candidates = Vec::new();
        let mut write_in = 0u64;
        for r in row.captures_iter(seg) {
            let v: u64 = r[3].trim().replace(',', "").parse().map_err(|_| format!("angka suara tidak valid di {name}"))?;
            if r[1].trim().eq_ignore_ascii_case("write-in") {
                write_in += v;
            } else {
                candidates.push(Candidate { name: clean_name(&r[2]), party: Party::classify(Some(r[1].trim())), votes: v });
            }
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        let c = Contest { state: cfg.state.clone(), label: if is_sen { "Senat AS".into() } else { format!("Distrik {}", dist.unwrap()) }, district: dist, candidates, write_in, precincts: None, polling_pct };
        if is_sen {
            senate = Some(c);
        } else {
            house.push(c);
        }
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate, house, refresh_note: None })
}

/// Oregon SOS results pages (HTML). The page states "Results last updated:
/// M/D/YYYY | H:MM AM PT"; the election is after the November DST change, so
/// PT is PST (UTC-8). The page's own precinct counter is not used (it reads
/// 0 of N although votes are counted; Oregon votes by mail).
pub fn parse_orsos(html: &str, cfg: &SourceCfg) -> Result<StateFeed, String> {
    use chrono::{FixedOffset, NaiveDateTime, TimeZone};
    let re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let ts = Regex::new(r"Results last updated:\s*(?:<[^>]+>\s*)*(\d+/\d+/\d{4})\s*\|?\s*(\d+:\d{2} [AP]M)\s*(PT|MT|CT|ET)").map_err(|e| e.to_string())?;
    let c = ts.captures(html).ok_or("halaman tidak mencantumkan waktu pembaruan")?;
    let naive = NaiveDateTime::parse_from_str(&format!("{} {}", &c[1], &c[2]), "%m/%d/%Y %I:%M %p").map_err(|e| e.to_string())?;
    // US standard time offsets (valid after the Nov 1 2026 DST change).
    let hours = match &c[3] { "PT" => 8, "MT" => 7, "CT" => 6, _ => 5 };
    let source_time = FixedOffset::west_opt(hours * 3600).unwrap().from_local_datetime(&naive).single().ok_or("waktu tidak valid")?.with_timezone(&Utc);
    let head = Regex::new(r"<h1>((?:US|United States) [^<]*)</h1>").map_err(|e| e.to_string())?;
    let cand = Regex::new(r#"(?s)<h1>([^<]*)</h1>\s*<h2 class="[^"]*">([^<]*)</h2>.*?<div class="col display-results-box-f">\s*<(?:div|h1)>([\d,]+)</(?:div|h1)>"#).map_err(|e| e.to_string())?;
    let prec = Regex::new(r"Precincts Fully:\s*(\d+)\s*/\s*(\d+)").map_err(|e| e.to_string())?;
    let heads: Vec<(usize, usize, String)> = head.captures_iter(html).map(|c| (c.get(0).unwrap().start(), c.get(0).unwrap().end(), c[1].trim().to_string())).collect();
    if heads.is_empty() {
        return Err("format halaman tidak dikenali: tidak ada kontes".into());
    }
    let mut senate = None;
    let mut house = Vec::new();
    for (i, (_, end, name)) in heads.iter().enumerate() {
        let stop = heads.get(i + 1).map(|h| h.0).unwrap_or(html.len());
        let seg = &html[*end..stop];
        let is_sen = cfg.senate_exact.iter().any(|n| n == name);
        let dist = re.captures(name).and_then(|c| c[1].parse::<u32>().ok());
        if !is_sen && dist.is_none() {
            continue;
        }
        let precincts = prec.captures(seg).and_then(|c| Some((c[1].parse::<u32>().ok()?, c[2].parse::<u32>().ok()?)));
        let mut candidates = Vec::new();
        let mut write_in = 0u64;
        for r in cand.captures_iter(seg) {
            let v: u64 = r[3].replace(',', "").parse().map_err(|_| format!("angka suara tidak valid di {name}"))?;
            let party_txt = r[2].split(',').next().unwrap_or("").trim().to_string();
            if r[1].trim().eq_ignore_ascii_case("write-in") {
                write_in += v;
            } else {
                candidates.push(Candidate { name: clean_name(&r[1]), party: Party::classify(Some(&party_txt)), votes: v });
            }
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        let ct = Contest { state: cfg.state.clone(), label: if is_sen { "Senat AS".into() } else { format!("Distrik {}", dist.unwrap()) }, district: dist, candidates, write_in, precincts, polling_pct: None };
        if is_sen { senate = Some(ct); } else { house.push(ct); }
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate, house, refresh_note: None })
}

/// Maryland SBE results page (HTML). "Last refreshed: MM/DD/YYYY hh:mm:ss AM"
/// is Eastern time; on Nov 4 2026 that is EST (UTC-5). The page's precinct
/// note counts election-day precincts only, so it is not used as a
/// reporting fraction.
pub fn parse_mdsbe(html: &str, cfg: &SourceCfg) -> Result<StateFeed, String> {
    use chrono::{FixedOffset, NaiveDateTime, TimeZone};
    let ts = Regex::new(r"Last refreshed:\s*(\d+/\d+/\d{4} \d+:\d{2}:\d{2} [AP]M)").map_err(|e| e.to_string())?;
    let c = ts.captures(html).ok_or("halaman tidak mencantumkan waktu pembaruan")?;
    let naive = NaiveDateTime::parse_from_str(&c[1], "%m/%d/%Y %I:%M:%S %p").map_err(|e| e.to_string())?;
    let source_time = FixedOffset::west_opt(5 * 3600).unwrap().from_local_datetime(&naive).single().ok_or("waktu tidak valid")?.with_timezone(&Utc);
    let head = Regex::new(r"<h3>District (\d+)</h3>").map_err(|e| e.to_string())?;
    let row = Regex::new(r#"(?s)<tr>\s*<td><div>(.*?)</div></td>\s*<td>([^<]*)</td>.*?<td class="alignRight">\s*([\d,]+)</td>\s*<td class="alignRight">[\d.]+%</td>"#).map_err(|e| e.to_string())?;
    let heads: Vec<(usize, usize, u32)> = head.captures_iter(html).filter_map(|c| Some((c.get(0)?.start(), c.get(0)?.end(), c[1].parse().ok()?))).collect();
    if heads.is_empty() {
        return Err("format halaman tidak dikenali: tidak ada distrik".into());
    }
    let mut house = Vec::new();
    for (i, (_, end, dist)) in heads.iter().enumerate() {
        let stop = heads.get(i + 1).map(|h| h.0).unwrap_or(html.len());
        let seg = &html[*end..stop];
        let mut candidates = Vec::new();
        let mut write_in = 0u64;
        for r in row.captures_iter(seg) {
            let name = r[1].trim();
            if name.contains("<b>") {
                continue; // Totals row
            }
            let v: u64 = r[3].replace(',', "").parse().map_err(|_| format!("angka suara tidak valid di distrik {dist}"))?;
            if name.to_ascii_lowercase().contains("write-in") {
                write_in += v;
            } else {
                candidates.push(Candidate { name: clean_name(&name.replace("&quot;", "\"")), party: Party::classify(Some(r[2].trim())), votes: v });
            }
        }
        if candidates.is_empty() {
            return Err(format!("distrik {dist}: tidak ada kandidat terbaca"));
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        house.push(Contest { state: cfg.state.clone(), label: format!("Distrik {dist}"), district: Some(*dist), candidates, write_in, precincts: None, polling_pct: None });
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate: None, house, refresh_note: None })
}

/// Per-source cache kept in the warm function instance. The source files are
/// large (27-48 MB), so one lock per source gives single-flight refreshes, a
/// minimum interval between origin requests, and conditional requests
/// (If-None-Match) so an unchanged file costs a 304 and no download.
struct Entry {
    checked: Instant,
    etag: Option<String>,
    feed: Option<StateFeed>,
}

pub const MIN_REFRESH_SECS: u64 = 90;
/// A cached copy is served for at most this long after a failed refresh.
pub const MAX_STALE_SERVE_SECS: u64 = 15 * 60;

static CACHE: OnceLock<Mutex<HashMap<String, Arc<Mutex<Option<Entry>>>>>> = OnceLock::new();

async fn slot(state: &str) -> Arc<Mutex<Option<Entry>>> {
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock().await;
    g.entry(state.to_string()).or_insert_with(|| Arc::new(Mutex::new(None))).clone()
}

pub async fn fetch_state(client: &reqwest::Client, cfg: &SourceCfg) -> FeedResult {
    let slot = slot(&cfg.state).await;
    let mut guard = slot.lock().await; // single flight per source
    if let Some(e) = guard.as_ref() {
        if e.checked.elapsed().as_secs() < MIN_REFRESH_SECS {
            if let Some(f) = &e.feed {
                return FeedResult::Ok(f.clone());
            }
        }
    }
    let fail = |reason: String| FeedResult::Failed {
        state: cfg.state.clone(),
        source_label: cfg.label.clone(),
        human_url: cfg.human_url.clone(),
        reason,
    };
    if let Some(why) = &cfg.suspended {
        return fail(format!("pengambilan data dihentikan: {why}"));
    }
    let mut req = client.get(&cfg.export_url);
    if let Some(Entry { etag: Some(t), feed: Some(_), .. }) = guard.as_ref() {
        req = req.header("If-None-Match", t.clone());
    }
    let outcome: Result<Option<(StateFeed, Option<String>)>, String> = async {
        let resp = req.send().await.map_err(|e| format!("gagal menghubungi sumber: {e}"))?;
        if resp.status().as_u16() == 304 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("sumber membalas HTTP {}", resp.status().as_u16()));
        }
        let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        let lm = resp
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| DateTime::parse_from_rfc2822(s).ok())
            .map(|d| d.with_timezone(&Utc));
        let bytes = resp.bytes().await.map_err(|e| format!("gagal membaca data: {e}"))?;
        let feed = if cfg.format == "mdsbe" {
            parse_mdsbe(&String::from_utf8_lossy(&bytes), cfg)?
        } else if cfg.format == "orsos" {
            parse_orsos(&String::from_utf8_lossy(&bytes), cfg)?
        } else if cfg.format == "txsos" {
            let t = lm.ok_or_else(|| "sumber tidak mencantumkan Last-Modified".to_string())?;
            parse_txsos(&String::from_utf8_lossy(&bytes), cfg, t)?
        } else if cfg.format == "ncsbe" {
            let t = lm.ok_or_else(|| "sumber tidak mencantumkan Last-Modified".to_string())?;
            parse_ncsbe(&bytes, cfg, t)?
        } else {
            parse_export(&bytes, cfg)?
        };
        Ok(Some((feed, etag)))
    }
    .await;
    match outcome {
        Ok(Some((feed, etag))) => {
            *guard = Some(Entry { checked: Instant::now(), etag, feed: Some(feed.clone()) });
            FeedResult::Ok(feed)
        }
        Ok(None) => {
            // 304: the cached copy is still the current file.
            if let Some(e) = guard.as_mut() {
                e.checked = Instant::now();
                if let Some(f) = &e.feed {
                    return FeedResult::Ok(f.clone());
                }
            }
            fail("sumber membalas 304 tanpa salinan tersimpan".into())
        }
        Err(reason) => {
            if let Some(Entry { checked, feed: Some(f), .. }) = guard.as_ref() {
                if checked.elapsed().as_secs() < MAX_STALE_SERVE_SECS {
                    let mut f = f.clone();
                    f.refresh_note = Some(format!("pembaruan terakhir gagal ({reason}); menampilkan salinan terakhir yang berhasil dibaca"));
                    return FeedResult::Ok(f);
                }
            }
            fail(reason)
        }
    }
}
