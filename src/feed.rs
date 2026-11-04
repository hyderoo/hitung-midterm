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
        let bytes = resp.bytes().await.map_err(|e| format!("gagal membaca data: {e}"))?;
        let feed = parse_export(&bytes, cfg)?;
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
