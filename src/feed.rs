//! Reader for the "Media Export" JSON that Enhanced Voting election night
//! reporting sites publish (used by the Virginia and Georgia sites).
//! Only the fields we need are declared, so serde skips the rest (precinct
//! level detail) without building it in memory.
use crate::config::SourceCfg;
use crate::model::*;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Deserialize;

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
    })
}

pub async fn fetch_state(client: &reqwest::Client, cfg: &SourceCfg) -> FeedResult {
    let fail = |reason: String| FeedResult::Failed {
        state: cfg.state.clone(),
        source_label: cfg.label.clone(),
        human_url: cfg.human_url.clone(),
        reason,
    };
    let resp = match client.get(&cfg.export_url).send().await {
        Ok(r) => r,
        Err(e) => return fail(format!("gagal menghubungi sumber: {e}")),
    };
    if !resp.status().is_success() {
        return fail(format!("sumber membalas HTTP {}", resp.status().as_u16()));
    }
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return fail(format!("gagal membaca data: {e}")),
    };
    match parse_export(&bytes, cfg) {
        Ok(f) => FeedResult::Ok(f),
        Err(e) => fail(e),
    }
}
