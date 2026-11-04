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
        unresolved: 0,
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
        Contest { state: cfg.state.clone(), label, district, candidates, write_in, unresolved: 0, precincts, polling_pct: None }
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
        let dist = re.captures(name).and_then(|c| c.get(1)).and_then(|m| m.as_str().parse::<u32>().ok());
        if !is_sen && re.captures(name).is_none() {
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
        let c = Contest { state: cfg.state.clone(), label: if is_sen { "Senat AS".into() } else { format!("Distrik {}", dist.unwrap()) }, district: dist, candidates, write_in, unresolved: 0, precincts: None, polling_pct };
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
    // Every h1 is a contest boundary except the ones inside a candidate
    // block (candidate name/vote cells are h1s too). This keeps non-federal
    // contests (Governor etc.) from leaking into the last federal segment.
    let head = Regex::new(cfg.head_regex.as_deref().unwrap_or(r"<h1>([^<]*)</h1>")).map_err(|e| e.to_string())?;
    let cand = Regex::new(r#"(?s)<h1>\s*(?:<img\b[^>]*>\s*)?([^<]*)</h1>\s*<h2 class="[^"]*">([^<]*)</h2>.*?<div class="col display-results-box-f">\s*<(?:div|h1)>([\d,]+)</(?:div|h1)>"#).map_err(|e| e.to_string())?;
    let prec = Regex::new(r"Precincts Fully:\s*(\d+)\s*/\s*(\d+)").map_err(|e| e.to_string())?;
    let cand_spans: Vec<(usize, usize)> = cand.captures_iter(html).map(|c| (c.get(0).unwrap().start(), c.get(0).unwrap().end())).collect();
    let heads: Vec<(usize, usize, String)> = head
        .captures_iter(html)
        .filter(|c| {
            let p = c.get(0).unwrap().start();
            !cand_spans.iter().any(|&(a, b)| p >= a && p < b)
        })
        .map(|c| (c.get(0).unwrap().start(), c.get(0).unwrap().end(), c[1].trim().to_string()))
        .collect();
    if heads.is_empty() {
        return Err("format halaman tidak dikenali: tidak ada kontes".into());
    }
    let mut senate = None;
    let mut house = Vec::new();
    for (i, (_, end, name)) in heads.iter().enumerate() {
        let stop = heads.get(i + 1).map(|h| h.0).unwrap_or(html.len());
        let seg = &html[*end..stop];
        let is_sen = cfg.senate_exact.iter().any(|n| n == name);
        let dist = re.captures(name).and_then(|c| c.get(1)).and_then(|m| m.as_str().parse::<u32>().ok());
        if !is_sen && re.captures(name).is_none() {
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
        if candidates.is_empty() {
            return Err(format!("kontes {name}: tidak ada kandidat terbaca; tata letak sumber mungkin berubah"));
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        let label = if is_sen { "Senat AS".to_string() } else { dist.map(|d| format!("Distrik {d}")).unwrap_or_else(|| "Distrik at-large".to_string()) };
        let ct = Contest { state: cfg.state.clone(), label, district: dist, candidates, write_in, unresolved: 0, precincts, polling_pct: None };
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
        house.push(Contest { state: cfg.state.clone(), label: format!("Distrik {dist}"), district: Some(*dist), candidates, write_in, unresolved: 0, precincts: None, polling_pct: None });
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate: None, house, refresh_note: None })
}


/// Alabama election-night statewide results page (HTML). "Last Updated:
/// <span id="lblLastUpdated">M/D/YYYY H:MM:SS AM" is Central time (UTC-6
/// after the Nov 1 2026 DST change). The page publishes no precinct counts.
pub fn parse_alsos(html: &str, cfg: &SourceCfg) -> Result<StateFeed, String> {
    use chrono::{FixedOffset, NaiveDateTime, TimeZone};
    let re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let ts = Regex::new(r#"(?s)Last Updated:.*?lblLastUpdated">\s*(\d+/\d+/\d{4} \d+:\d{2}:\d{2} [AP]M)"#).map_err(|e| e.to_string())?;
    let c = ts.captures(html).ok_or("halaman tidak mencantumkan waktu pembaruan")?;
    let naive = NaiveDateTime::parse_from_str(&c[1], "%m/%d/%Y %I:%M:%S %p").map_err(|e| e.to_string())?;
    let source_time = FixedOffset::west_opt(6 * 3600).unwrap().from_local_datetime(&naive).single().ok_or("waktu tidak valid")?.with_timezone(&Utc);
    let head = Regex::new(r#"enrContestHeader">\s*([^<]+?)\s*</td>"#).map_err(|e| e.to_string())?;
    let cand = Regex::new(r#"(?s)enr(?:Alt)?CandidateListItemCol enrCandNameCol"[^>]*>([^<]+)</td>.*?enrCandVoteNumCol"><div class="numberValue">([\d,]+)</div>"#).map_err(|e| e.to_string())?;
    let party_tag = Regex::new(r"\(([A-Za-z]+)\)\s*$").map_err(|e| e.to_string())?;
    let cand_spans: Vec<(usize, usize)> = cand.captures_iter(html).map(|c| (c.get(0).unwrap().start(), c.get(0).unwrap().end())).collect();
    let heads: Vec<(usize, usize, String)> = head
        .captures_iter(html)
        .filter(|c| {
            let p = c.get(0).unwrap().start();
            !cand_spans.iter().any(|&(a, b)| p >= a && p < b)
        })
        .map(|c| (c.get(0).unwrap().start(), c.get(0).unwrap().end(), c[1].trim().to_string()))
        .collect();
    if heads.is_empty() {
        return Err("format halaman tidak dikenali: tidak ada kontes".into());
    }
    let mut senate = None;
    let mut house = Vec::new();
    for (i, (_, end, name)) in heads.iter().enumerate() {
        let stop = heads.get(i + 1).map(|h| h.0).unwrap_or(html.len());
        let seg = &html[*end..stop];
        let is_sen = cfg.senate_exact.iter().any(|n| n == name);
        let dist = re.captures(name).and_then(|c| c.get(1)).and_then(|m| m.as_str().parse::<u32>().ok());
        if !is_sen && re.captures(name).is_none() {
            continue;
        }
        let mut candidates = Vec::new();
        let mut write_in = 0u64;
        for r in cand.captures_iter(seg) {
            let v: u64 = r[2].replace(',', "").parse().map_err(|_| format!("angka suara tidak valid di {name}"))?;
            let raw = r[1].trim();
            if raw.to_ascii_lowercase().starts_with("write-in") {
                write_in += v;
                continue;
            }
            let pty = party_tag.captures(raw).map(|c| c[1].to_string());
            candidates.push(Candidate { name: clean_name(raw), party: Party::classify(pty.as_deref()), votes: v });
        }
        if candidates.is_empty() {
            return Err(format!("kontes {name}: tidak ada kandidat terbaca"));
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        let label = if is_sen { "Senat AS".to_string() } else { dist.map(|d| format!("Distrik {d}")).unwrap_or_else(|| "Distrik at-large".to_string()) };
        let ct = Contest { state: cfg.state.clone(), label, district: dist, candidates, write_in, unresolved: 0, precincts: None, polling_pct: None };
        if is_sen { senate = Some(ct); } else { house.push(ct); }
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate, house, refresh_note: None })
}

fn roman(s: &str) -> Option<u32> {
    let mut total = 0i64;
    let mut prev = 0i64;
    for ch in s.chars().rev() {
        let v = match ch { 'I' => 1, 'V' => 5, 'X' => 10, 'L' => 50, _ => return None };
        if v < prev { total -= v } else { total += v; prev = v }
    }
    if total > 0 { Some(total as u32) } else { None }
}

/// Split a TSV with RFC-4180-style quoting (fields may hold newlines).
fn tsv_rows(s: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_q = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') { chars.next(); field.push('"'); } else { in_q = false; }
            } else { field.push(c); }
        } else {
            match c {
                '"' => in_q = true,
                '\t' => { row.push(std::mem::take(&mut field)); }
                '\n' => { row.push(std::mem::take(&mut field)); rows.push(std::mem::take(&mut row)); }
                '\r' => {}
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() { row.push(field); rows.push(row); }
    rows
}

/// Hawaii Office of Elections statewide summary (tab-delimited text). Column
/// layout follows the "#Contest ID" header row. There is no page timestamp,
/// so the HTTP Last-Modified header is the source time.
pub fn parse_hisummary(text: &str, cfg: &SourceCfg, lm: DateTime<Utc>) -> Result<StateFeed, String> {
    let re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let rows = tsv_rows(text);
    let mut house: std::collections::BTreeMap<u32, Contest> = std::collections::BTreeMap::new();
    for row in &rows {
        if row.len() < 21 || row[0].starts_with('#') || row[0].is_empty() {
            continue;
        }
        let title = row[1].trim();
        let dist = re.captures(title).and_then(|c| c.get(1)).and_then(|m| roman(m.as_str()));
        let Some(dist) = dist else { continue };
        let votes: u64 = row[20].trim().replace(',', "").parse().map_err(|_| format!("angka suara tidak valid di {title}"))?;
        let raw_name = row[15].trim();
        let (party, name) = if raw_name.starts_with('(') {
            let end = raw_name.find(')').unwrap_or(3);
            let letter = raw_name[1..end].trim().to_ascii_lowercase();
            let party = if letter == "d" { Party::Dem } else if letter == "r" { Party::Rep } else { Party::Other };
            (party, raw_name[end + 1..].trim().to_string())
        } else {
            (Party::Other, raw_name.to_string())
        };
        let counted: u32 = row[13].trim().parse().unwrap_or(0);
        let total: u32 = row[12].trim().parse().unwrap_or(0);
        let ct = house.entry(dist).or_insert_with(|| Contest {
            state: cfg.state.clone(),
            label: format!("Distrik {dist}"),
            district: Some(dist),
            candidates: Vec::new(),
            write_in: 0,
            unresolved: 0,
            precincts: if total > 0 { Some((counted, total)) } else { None },
            polling_pct: None,
        });
        if name.to_ascii_lowercase().contains("write-in") || name.to_ascii_lowercase().contains("write in") {
            ct.write_in += votes;
        } else {
            ct.candidates.push(Candidate { name, party, votes });
        }
    }
    if house.is_empty() {
        return Err("format berkas tidak dikenali: tidak ada distrik DPR".into());
    }
    let mut house: Vec<Contest> = house.into_values().collect();
    for ct in &mut house {
        ct.candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
    }
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time: lm, senate: None, house, refresh_note: None })
}

/// Vermont election-night results (Angular SPA backed by static JSON).
/// export_url points at elections.json; the general-election entry's
/// federal.path names a timestamped per-tab file on the same host. The page's
/// "lastUpdatedDate" is Eastern time (UTC-5 after the Nov 1 2026 DST change).
pub async fn parse_vtenr(client: &reqwest::Client, elections: &[u8], cfg: &SourceCfg) -> Result<StateFeed, String> {
    use chrono::{FixedOffset, NaiveDateTime, TimeZone};
    let g: serde_json::Value = serde_json::from_slice(elections).map_err(|e| format!("respons tidak dikenali: {e}"))?;
    let arr = g.as_array().ok_or("daftar pemilihan tidak dikenali")?;
    let entry = arr.iter().find(|e| e["isGeneralElection"].as_bool() == Some(true) && e["isDefault"].as_bool() == Some(true))
        .or_else(|| arr.iter().find(|e| e["isGeneralElection"].as_bool() == Some(true)))
        .ok_or("tidak ada pemilihan umum dalam daftar")?;
    let guid = entry["electionGuid"].as_str().ok_or("GUID pemilihan tidak ada")?;
    let base = cfg.export_url.strip_suffix("elections/elections.json").ok_or("export_url tidak berakhiran elections.json")?;
    let meta_resp = client.get(format!("{base}elections/{guid}.json")).send().await.map_err(|e| format!("gagal menghubungi sumber: {e}"))?;
    if !meta_resp.status().is_success() {
        return Err(format!("sumber membalas HTTP {}", meta_resp.status().as_u16()));
    }
    let meta_bytes = meta_resp.bytes().await.map_err(|e| e.to_string())?;
    let meta: serde_json::Value = serde_json::from_slice(&meta_bytes).map_err(|e| format!("respons tidak dikenali: {e}"))?;
    let updated = meta["lastUpdatedDate"].as_str().ok_or("halaman tidak mencantumkan waktu pembaruan")?;
    let naive = NaiveDateTime::parse_from_str(updated, "%m/%d/%Y %I:%M %p").map_err(|e| e.to_string())?;
    let source_time = FixedOffset::west_opt(5 * 3600).unwrap().from_local_datetime(&naive).single().ok_or("waktu tidak valid")?.with_timezone(&Utc);
    let precincts = meta["townsReporting"].as_str().and_then(|s| {
        let (a, b) = s.split_once('/')?;
        Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?))
    });
    let path = meta["federal"]["path"].as_str().ok_or("berkas federal tidak tercantum")?.replace('\\', "/");
    let fed_resp = client.get(format!("{base}{path}")).send().await.map_err(|e| format!("gagal menghubungi sumber: {e}"))?;
    if !fed_resp.status().is_success() {
        return Err(format!("sumber membalas HTTP {}", fed_resp.status().as_u16()));
    }
    let fed: serde_json::Value = serde_json::from_slice(&fed_resp.bytes().await.map_err(|e| e.to_string())?).map_err(|e| format!("respons tidak dikenali: {e}"))?;
    let re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let mut house = Vec::new();
    let offices = fed["d"].as_array().and_then(|d| d.first()).and_then(|p| p["o"].as_array()).cloned().unwrap_or_default();
    for off in &offices {
        let name = off["on"].as_str().unwrap_or("").trim();
        if re.captures(name).is_none() {
            continue;
        }
        let mut totals: std::collections::BTreeMap<String, (String, Party, u64)> = std::collections::BTreeMap::new();
        let mut write_in = 0u64;
        if let Some(towns) = off["cs"].as_array() {
            for town in towns {
                if let Some(rows) = town["rc"].as_array() {
                    for r in rows {
                        let cid = r["cid"].as_i64().unwrap_or(0).to_string();
                        let cn = r["cn"].as_str().unwrap_or("").trim().to_string();
                        let pn = r["pn"].as_str().unwrap_or("").to_string();
                        let vc = r["vc"].as_u64().unwrap_or(0);
                        let e = totals.entry(cid).or_insert((cn, Party::classify(Some(&pn)), 0));
                        e.2 += vc;
                    }
                }
                if let Some(wrows) = town["wc"].as_array() {
                    for r in wrows {
                        write_in += r["vc"].as_u64().unwrap_or(0);
                    }
                }
            }
        }
        let mut candidates: Vec<Candidate> = totals.into_values().map(|(name, party, votes)| Candidate { name, party, votes }).collect();
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        if candidates.is_empty() {
            return Err(format!("kontes {name}: tidak ada kandidat terbaca"));
        }
        house.push(Contest { state: cfg.state.clone(), label: "Distrik at-large".into(), district: None, candidates, write_in, unresolved: 0, precincts, polling_pct: None });
    }
    if house.is_empty() {
        return Err("format berkas tidak dikenali: tidak ada kontes DPR".into());
    }
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate: None, house, refresh_note: None })
}


/// Indiana election-night results (Scytl "FirstTuesday" AngularJS SPA backed
/// by versioned static JSON). export_url points at data/settings.json; its
/// VersionType suffixes the data files (e.g. statewideElectionsC_A.json).
/// WriteTime values are Eastern time (UTC-5 after the Nov 1 2026 DST change).
pub async fn parse_inenr(client: &reqwest::Client, settings: &[u8], cfg: &SourceCfg) -> Result<StateFeed, String> {
    use chrono::{FixedOffset, NaiveDateTime, TimeZone};
    let st: serde_json::Value = serde_json::from_slice(settings).map_err(|e| format!("respons tidak dikenali: {e}"))?;
    let root = st.get("Root").unwrap_or(&st);
    let ver = root["VersionType"].as_str().ok_or("versi data tidak tercantum")?;
    let base = cfg.export_url.strip_suffix("settings.json").ok_or("export_url tidak berakhiran settings.json")?;
    let get = |path: String| {
        let client = client.clone();
        async move {
            let r = client.get(&path).send().await.map_err(|e| format!("gagal menghubungi sumber: {e}"))?;
            if !r.status().is_success() {
                return Err(format!("sumber membalas HTTP {}", r.status().as_u16()));
            }
            let b = r.bytes().await.map_err(|e| e.to_string())?;
            let v: serde_json::Value = serde_json::from_slice(&b).map_err(|e| format!("respons tidak dikenali: {e}"))?;
            Ok::<serde_json::Value, String>(v.get("Root").cloned().unwrap_or(v))
        }
    };
    let cats = get(format!("{base}statewideElectionsC_{ver}.json")).await?;
    let mut latest = root["WriteTime"].as_str().unwrap_or("").to_string();
    if cats["WriteTime"].as_str().unwrap_or("") > latest.as_str() {
        latest = cats["WriteTime"].as_str().unwrap_or("").to_string();
    }
    let empty = vec![];
    let headings = cats["List"].as_array().unwrap_or(&empty);
    let mut cat_id = None;
    for h in headings {
        let items = h["Items"]["Item"].as_array().unwrap_or(&empty);
        for it in items {
            let name = it["OFFICE_CATEGORY_NAME"].as_str().unwrap_or("");
            if cfg.senate_exact.iter().any(|n| n == name) || Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?.is_match(name) {
                cat_id = it["OFFICECATEGORYID"].as_str().map(|s| s.to_string());
            }
        }
    }
    let cat_id = cat_id.ok_or("kategori DPR tidak ditemukan")?;
    let house_re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let dist_re = Regex::new(r"\((\d+)\)").map_err(|e| e.to_string())?;
    let data = get(format!("{base}OffCatC_{cat_id}_{ver}.json")).await?;
    if data["WriteTime"].as_str().unwrap_or("") > latest.as_str() {
        latest = data["WriteTime"].as_str().unwrap_or("").to_string();
    }
    let naive = NaiveDateTime::parse_from_str(&latest, "%Y-%m-%dT%H:%M:%S%.f").map_err(|e| e.to_string())?;
    let source_time = FixedOffset::west_opt(5 * 3600).unwrap().from_local_datetime(&naive).single().ok_or("waktu tidak valid")?.with_timezone(&Utc);
    let races = data["StatewideSummary"]["Race"].as_array().ok_or("tidak ada daftar kontes")?;
    let mut house = Vec::new();
    for race in races {
        let title = race["OFFICE_TITLE"].as_str().unwrap_or("").trim();
        let sub = race["SubSortOrder"].as_str().unwrap_or("");
        if house_re.captures(title).is_none() && house_re.captures(sub).is_none() {
            continue;
        }
        let dist = dist_re.captures(sub).and_then(|c| c[1].parse::<u32>().ok());
        let cands = race["Candidates"]["Candidate"].as_array().cloned().unwrap_or_default();
        let mut candidates = Vec::new();
        let mut write_in = 0u64;
        for c in &cands {
            let votes = c["TOTAL"].as_u64().unwrap_or(0);
            let name = c["NAME_ON_BALLOT"].as_str().unwrap_or("").trim().to_string();
            if name.to_ascii_lowercase().contains("write-in") || name.to_ascii_lowercase().contains("write in") {
                write_in += votes;
                continue;
            }
            let letter = c["PARTY"].as_str().unwrap_or("").trim().to_ascii_lowercase();
            let party = if letter == "d" { Party::Dem } else if letter == "r" { Party::Rep } else { Party::Other };
            candidates.push(Candidate { name, party, votes });
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        if candidates.is_empty() {
            return Err(format!("kontes {title}: tidak ada kandidat terbaca"));
        }
        let label = dist.map(|d| format!("Distrik {d}")).unwrap_or_else(|| "Distrik at-large".to_string());
        house.push(Contest { state: cfg.state.clone(), label, district: dist, candidates, write_in, unresolved: 0, precincts: None, polling_pct: None });
    }
    if house.is_empty() {
        return Err("format berkas tidak dikenali: tidak ada kontes DPR".into());
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate: None, house, refresh_note: None })
}




/// South Carolina: scvotes.gov posts timestamped xlsx snapshots of statewide
/// results. export_url is the results page; the latest timestamped
/// *-General-Election-Results.xlsx link wins. Each contest sheet has the
/// contest title in row 1, candidate names (party-prefixed) in row 2, vote
/// columns in row 3 (7 per candidate, the last labeled "Total Votes"), one
/// row per county, and a "Total:" footer. The xlsx Last-Modified is the
/// source time.
pub fn scxlsx_feed(xlsx: &[u8], cfg: &SourceCfg, lm: DateTime<Utc>) -> Result<StateFeed, String> {
    use calamine::{Reader, Xlsx};
    use std::io::Cursor;
    let mut wb: Xlsx<_> = Xlsx::new(Cursor::new(xlsx)).map_err(|e| format!("berkas xlsx tidak terbaca: {e}"))?;
    let house_re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let prefix_re = Regex::new(r"^([A-Z]{2,5})\s+(.+)$").map_err(|e| e.to_string())?;
    let names = wb.sheet_names().to_vec();
    let mut senate = None;
    let mut house = Vec::new();
    for sheet in names {
        let range = match wb.worksheet_range(&sheet) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rows: Vec<Vec<String>> = range
            .rows()
            .map(|r| r.iter().map(|c| c.to_string()).collect())
            .collect();
        if rows.len() < 4 {
            continue;
        }
        let title = rows[0][0].trim().to_string();
        let is_sen = cfg.senate_exact.iter().any(|n| n == &title);
        let cap = house_re.captures(&title);
        if !is_sen && cap.is_none() {
            continue;
        }
        let cand_names: Vec<String> = rows[1].iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let total_cols: Vec<usize> = rows[2].iter().enumerate().filter(|(_, v)| v.trim() == "Total Votes").map(|(i, _)| i).collect();
        if cand_names.is_empty() || cand_names.len() != total_cols.len() {
            return Err(format!("kontes {title}: tata letak kolom tidak dikenali"));
        }
        let total_row = rows.iter().find(|r| r.first().map(|first| first.trim_matches(|ch: char| !ch.is_alphabetic())) == Some("Total"));
        let total_row = total_row.ok_or_else(|| format!("kontes {title}: baris Total tidak ditemukan"))?;
        let mut candidates = Vec::new();
        let mut write_in = 0u64;
        for (name, col) in cand_names.iter().zip(total_cols.iter()) {
            let votes = total_row.get(*col).and_then(|v| v.replace(',', "").parse::<u64>().ok()).unwrap_or(0);
            if name.eq_ignore_ascii_case("write-in") || name.eq_ignore_ascii_case("write in") {
                write_in += votes;
                continue;
            }
            let (party, clean) = match prefix_re.captures(name) {
                Some(c) => (Party::classify(Some(&c[1])), c[2].trim().to_string()),
                None => (Party::Other, name.clone()),
            };
            candidates.push(Candidate { name: clean, party, votes });
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        if candidates.is_empty() {
            return Err(format!("kontes {title}: tidak ada kandidat terbaca"));
        }
        if is_sen {
            senate = Some(Contest { state: cfg.state.clone(), label: title.clone(), district: None, candidates, write_in, unresolved: 0, precincts: None, polling_pct: None });
        } else {
            let dist = cap.and_then(|c| c.get(1)).and_then(|g| g.as_str().parse::<u32>().ok());
            let label = dist.map(|d| format!("Distrik {d}")).unwrap_or_else(|| "Distrik at-large".to_string());
            house.push(Contest { state: cfg.state.clone(), label, district: dist, candidates, write_in, unresolved: 0, precincts: None, polling_pct: None });
        }
    }
    if senate.is_none() && house.is_empty() {
        return Err("format berkas tidak dikenali: tidak ada kontes federal".into());
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time: lm, senate, house, refresh_note: None })
}

/// Pick the freshest timestamped results snapshot link from the SC results
/// page. Filenames carry nominal election-night publication times
/// ("2026-11-03-1130PM-...", "2026-11-03-12AM-..."); AM times are after
/// midnight, so they sort past the evening PM times. The file's own
/// Last-Modified (a few minutes around the label time) is the source time.
pub fn scxlsx_pick_link(html: &str) -> Option<String> {
    let link_re = Regex::new(r#"href="(https://scvotes\.gov/[^"]*?(\d{4})-(\d{2})-(\d{2})-([0-9-]+?)(AM|PM)-General-Election-Results\.xlsx)""#).ok()?;
    // Collect every snapshot: url, day key (yyyymmdd), minutes-of-day, am/pm.
    let mut entries: Vec<(String, u64, u64, bool)> = Vec::new();
    for c in link_re.captures_iter(html) {
        let url = c[1].to_string();
        let digits: String = c[5].chars().filter(|ch| ch.is_ascii_digit()).collect();
        let (hh, mm) = match digits.len() {
            1 | 2 => (digits.parse::<u64>().unwrap_or(0), 0),
            3 => (digits[..1].parse::<u64>().unwrap_or(0), digits[1..].parse::<u64>().unwrap_or(0)),
            _ => (digits[..2].parse::<u64>().unwrap_or(0), digits[2..].parse::<u64>().unwrap_or(0)),
        };
        let h24 = if &c[6] == "PM" && hh != 12 { hh + 12 } else if &c[6] == "AM" && hh == 12 { 0 } else { hh };
        let day: u64 = c[2].parse().unwrap_or(0) * 10000 + c[3].parse().unwrap_or(0) * 100 + c[4].parse().unwrap_or(0);
        entries.push((url, day, h24 * 60 + mm, &c[6] == "AM"));
    }
    // Election-night snapshots are dated for election day but AM labels belong
    // just past midnight. Roll an AM label to the next day only when its date
    // also carries late-evening (>= 6PM) snapshots, the midnight-rollover
    // pattern; a next-morning file dated the next day must not be bumped.
    let pm_late_days: std::collections::HashSet<u64> = entries
        .iter()
        .filter(|(_, _, m, am)| !am && *m >= 18 * 60)
        .map(|(_, d, _, _)| *d)
        .collect();
    let mut best: Option<(u64, String)> = None;
    for (url, day, minutes, am) in entries {
        let minutes = if am && pm_late_days.contains(&day) { minutes + 24 * 60 } else { minutes };
        let key = day * 100000 + minutes;
        if best.as_ref().map(|(k, _)| key > *k).unwrap_or(true) {
            best = Some((key, url));
        }
    }
    best.map(|(_, u)| u)
}

pub async fn parse_scxlsx(client: &reqwest::Client, html: &[u8], cfg: &SourceCfg) -> Result<StateFeed, String> {
    let text = String::from_utf8_lossy(html);
    let url = scxlsx_pick_link(&text).ok_or("tidak ada tautan hasil xlsx di halaman")?;
    let resp = client.get(&url).send().await.map_err(|e| format!("gagal menghubungi sumber: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("sumber membalas HTTP {}", resp.status().as_u16()));
    }
    let lm = resp
        .headers()
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| DateTime::parse_from_rfc2822(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .ok_or("sumber tidak mencantumkan Last-Modified")?;
    let bytes = resp.bytes().await.map_err(|e| format!("gagal membaca data: {e}"))?;
    scxlsx_feed(&bytes, cfg, lm)
}

/// Delaware official results via the state open-data portal (Socrata). The
/// ENR page elections.delaware.gov/results/enr/GE2026.html links this exact
/// query. One row per candidate with statewide totals; data_as_of is Eastern
/// time (UTC-5 after the Nov 1 2026 DST change).
pub fn parse_desocrata(bytes: &[u8], cfg: &SourceCfg) -> Result<StateFeed, String> {
    use chrono::{FixedOffset, NaiveDateTime, TimeZone};
    let rows: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| format!("respons tidak dikenali: {e}"))?;
    let rows = rows.as_array().ok_or("respons tidak dikenali: bukan daftar")?;
    let house_re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let mut latest = "";
    for r in rows {
        let t = r["data_as_of"].as_str().unwrap_or("");
        if t > latest {
            latest = t;
        }
    }
    if latest.is_empty() {
        return Err("waktu sumber tidak tercantum".into());
    }
    let naive = NaiveDateTime::parse_from_str(latest, "%Y-%m-%dT%H:%M:%S%.f").map_err(|e| e.to_string())?;
    let source_time = FixedOffset::west_opt(5 * 3600).unwrap().from_local_datetime(&naive).single().ok_or("waktu tidak valid")?.with_timezone(&Utc);
    let mut senate = None;
    let mut house = Vec::new();
    for r in rows {
        let office = r["office"].as_str().unwrap_or("").trim();
        let is_sen = cfg.senate_exact.iter().any(|n| n == office);
        let cap = house_re.captures(office);
        if !is_sen && cap.is_none() {
            continue;
        }
        let name = r["candidate_name"].as_str().unwrap_or("").trim().to_string();
        let votes = r["total_votes"].as_str().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let party = Party::classify(r["party"].as_str());
        let dist = cap.and_then(|c| c.get(1)).and_then(|g| g.as_str().parse::<u32>().ok());
        let label = dist.map(|d| format!("Distrik {d}")).unwrap_or_else(|| "Distrik at-large".to_string());
        let cand = Candidate { name, party, votes };
        if is_sen {
            senate.get_or_insert_with(|| Contest { state: cfg.state.clone(), label: office.to_string(), district: None, candidates: vec![], write_in: 0, unresolved: 0, precincts: None, polling_pct: None }).candidates.push(cand);
        } else if let Some(ct) = house.iter_mut().find(|c: &&mut Contest| c.district == dist) {
            ct.candidates.push(cand);
        } else {
            house.push(Contest { state: cfg.state.clone(), label, district: dist, candidates: vec![cand], write_in: 0, unresolved: 0, precincts: None, polling_pct: None });
        }
    }
    for ct in house.iter_mut().chain(senate.iter_mut()) {
        ct.candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        if ct.candidates.is_empty() {
            return Err("kontes tanpa kandidat".into());
        }
    }
    if senate.is_none() && house.is_empty() {
        return Err("format berkas tidak dikenali: tidak ada kontes federal".into());
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate, house, refresh_note: None })
}

/// KNOWiNK "TotalResults" JSON API (North Dakota: api.resultsnd.sos.nd.gov,
/// Arkansas: enr-results-api.totalresults.com). export_url points at
/// {base}/Election/GetElectionList?cId={client}; the election marked
/// isDefault is used. Party labels come from GetElectionInfo (AR keys
/// parties by GUID, ND inlines REP/DEM/IND), names from
/// GetContestSearchList, votes from GetContestResults. lastUpdated is UTC.
pub fn totalresults_feed(list: &[u8], info: &[u8], search: &[u8], results: &[u8], cfg: &SourceCfg) -> Result<StateFeed, String> {
    use chrono::TimeZone;
    use std::collections::HashMap;
    let parse = |b: &[u8]| serde_json::from_slice::<serde_json::Value>(b).map_err(|e| format!("respons tidak dikenali: {e}"));
    let info = parse(info)?;
    let search = parse(search)?;
    let results = parse(results)?;
    let ts = |v: &serde_json::Value| -> Option<DateTime<Utc>> {
        let s = v["lastUpdated"].as_str()?;
        if let Ok(d) = DateTime::parse_from_rfc3339(s) {
            return Some(d.with_timezone(&Utc));
        }
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|n| Utc.from_utc_datetime(&n))
    };
    let source_time = [ts(&info), ts(&search), ts(&results)].into_iter().flatten().max().ok_or("waktu sumber tidak tercantum")?;
    let mut parties: HashMap<String, String> = HashMap::new();
    if let Some(obj) = info["response"]["parties"].as_object() {
        for (k, v) in obj {
            let name = v["partyName"].as_str().filter(|s| !s.is_empty()).or_else(|| v["abbreviation"].as_str()).unwrap_or(k.as_str());
            parties.insert(k.clone(), name.to_string());
        }
    }
    let meta = search["response"]["contests"].as_object().ok_or("daftar kontes tidak ditemukan")?;
    let res = results["response"]["contests"].as_object().ok_or("hasil kontes tidak ditemukan")?;
    let house_re = Regex::new(&cfg.house_regex).map_err(|e| e.to_string())?;
    let prefix_re = Regex::new(r"^([A-Z]{2,5})\s+(.+)$").map_err(|e| e.to_string())?;
    let mut senate = None;
    let mut house = Vec::new();
    for (key, m) in meta {
        let cname = m["contestName"].as_str().unwrap_or("").trim();
        let is_sen = cfg.senate_exact.iter().any(|n| n == cname);
        let cap = house_re.captures(cname);
        if !is_sen && cap.is_none() {
            continue;
        }
        let r = match res.get(key) {
            Some(r) => r,
            None => continue,
        };
        let mut candidates = Vec::new();
        let mut write_in = 0u64;
        for ch in r["choices"].as_array().cloned().unwrap_or_default() {
            let votes = ch["totalVotes"].as_u64().unwrap_or(0);
            let id = ch["choiceID"].as_str().unwrap_or("");
            let meta_name = m["choices"][id]["name"].as_str().unwrap_or("").trim().to_string();
            let lower = meta_name.to_ascii_lowercase();
            if lower.contains("write-in") || lower.contains("write in") || ch["isWriteIn"].as_bool() == Some(true) {
                write_in += votes;
                continue;
            }
            let pid = ch["partyID"].as_str().filter(|s| !s.trim().is_empty()).or_else(|| m["choices"][id]["partyID"].as_str()).unwrap_or("");
            let pname = parties.get(pid).map(|s| s.as_str()).unwrap_or(pid);
            let party = Party::classify(Some(pname));
            // Some clients prefix names with a party code ("REP Tom Cotton").
            let name = match prefix_re.captures(&meta_name) {
                Some(c) if !parties.contains_key(&c[1]) => c[2].trim().to_string(),
                _ => meta_name,
            };
            candidates.push(Candidate { name, party, votes });
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        if candidates.is_empty() {
            return Err(format!("kontes {cname}: tidak ada kandidat terbaca"));
        }
        let precincts = match (r["precinctsReporting"].as_u64(), r["totalPrecincts"].as_u64()) {
            (Some(a), Some(b)) => Some((a as u32, b as u32)),
            _ => None,
        };
        if is_sen {
            senate = Some(Contest { state: cfg.state.clone(), label: cname.to_string(), district: None, candidates, write_in, unresolved: 0, precincts, polling_pct: None });
        } else {
            let dist = cap.and_then(|c| c.get(1)).and_then(|g| g.as_str().parse::<u32>().ok());
            let label = dist.map(|d| format!("Distrik {d}")).unwrap_or_else(|| "Distrik at-large".to_string());
            house.push(Contest { state: cfg.state.clone(), label, district: dist, candidates, write_in, unresolved: 0, precincts, polling_pct: None });
        }
    }
    if senate.is_none() && house.is_empty() {
        return Err("format berkas tidak dikenali: tidak ada kontes federal".into());
    }
    house.sort_by_key(|c| c.district);
    Ok(StateFeed { state: cfg.state.clone(), source_label: cfg.label.clone(), human_url: cfg.human_url.clone(), export_url: cfg.export_url.clone(), source_time, senate, house, refresh_note: None })
}

pub async fn parse_totalresults(client: &reqwest::Client, list_body: &[u8], cfg: &SourceCfg) -> Result<StateFeed, String> {
    let cid = cfg.export_url.split("cId=").nth(1).and_then(|s| s.split('&').next()).ok_or("export_url tidak mencantumkan cId")?.to_string();
    let base = cfg.export_url.split("/Election/GetElectionList").next().ok_or("export_url tidak dikenali")?.to_string();
    let list: serde_json::Value = serde_json::from_slice(list_body).map_err(|e| format!("respons tidak dikenali: {e}"))?;
    let eid = list
        .as_array()
        .and_then(|a| a.iter().find(|e| e["isDefault"].as_bool() == Some(true)))
        .and_then(|e| e["electionID"].as_str())
        .map(|s| s.to_string())
        .ok_or("pemilihan bawaan tidak ditemukan")?;
    let get = |path: String| {
        let client = client.clone();
        async move {
            let r = client.get(&path).send().await.map_err(|e| format!("gagal menghubungi sumber: {e}"))?;
            if !r.status().is_success() {
                return Err(format!("sumber membalas HTTP {}", r.status().as_u16()));
            }
            r.bytes().await.map_err(|e| e.to_string())
        }
    };
    let info = get(format!("{base}/Election/GetElectionInfo?cId={cid}&eId={eid}")).await?;
    let search = get(format!("{base}/Contest/GetContestSearchList?cId={cid}&eId={eid}")).await?;
    let results = get(format!("{base}/Contest/GetContestResults?cId={cid}&eId={eid}")).await?;
    totalresults_feed(list_body, &info, &search, &results, cfg)
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


/// New Jersey: the state publishes no statewide night feed; the official page
/// links each county's Clarity ENR site. cfg.export_url and cfg.extra_urls
/// hold each county's current_ver.txt URL; the live version directory carries
/// json/sum.json with every contest (C), candidate names (CH, party-prefixed),
/// votes (V) and precincts reporting (PR/TP). Contest wording varies by
/// county ("United States Senate" vs "United States Senator"; "2nd Cong" vs
/// "(8th Congressional District)"), so matching is by pattern, not exact
/// names. County coverage is partial (the linked counties only) and the
/// source label says so. Source time is the newest sum.json Last-Modified.
pub async fn parse_njclarity(client: &reqwest::Client, first_ver_body: &[u8], cfg: &SourceCfg) -> Result<StateFeed, String> {
    let mut urls = vec![cfg.export_url.clone()];
    urls.extend(cfg.extra_urls.iter().cloned());
    let mut docs: Vec<(String, Vec<u8>)> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut newest: Option<DateTime<Utc>> = None;
    let mut oldest: Option<DateTime<Utc>> = None;
    let mut first = Some(String::from_utf8_lossy(first_ver_body).trim().to_string());
    for ver_url in &urls {
        let base = ver_url.strip_suffix("current_ver.txt").ok_or("url county tidak berakhiran current_ver.txt")?;
        let county = base
            .trim_end_matches('/')
            .rsplit('/')
            .nth(1)
            .unwrap_or("")
            .replace('_', " ");
        let one: Result<Vec<u8>, String> = async {
            let ver = if let Some(v) = first.take() {
                v
            } else {
                let r = client.get(ver_url).send().await.map_err(|e| format!("{e}"))?;
                if !r.status().is_success() {
                    return Err(format!("HTTP {}", r.status().as_u16()));
                }
                r.text().await.map_err(|e| e.to_string())?.trim().to_string()
            };
            let sum_url = format!("{base}{ver}/json/sum.json");
            let r = client.get(&sum_url).send().await.map_err(|e| format!("{e}"))?;
            if !r.status().is_success() {
                return Err(format!("HTTP {}", r.status().as_u16()));
            }
            if let Some(d) = r.headers().get("last-modified").and_then(|v| v.to_str().ok()).and_then(|s| DateTime::parse_from_rfc2822(s).ok()).map(|d| d.with_timezone(&Utc)) {
                if newest.map(|n| d > n).unwrap_or(true) {
                    newest = Some(d);
                }
                if oldest.map(|o| d < o).unwrap_or(true) {
                    oldest = Some(d);
                }
            }
            Ok(r.bytes().await.map_err(|e| e.to_string())?.to_vec())
        }
        .await;
        match one {
            Ok(body) => docs.push((county, body)),
            Err(e) => failed.push(format!("{county} ({e})")),
        }
    }
    if docs.is_empty() {
        return Err(format!("semua kabupaten gagal diambil: {}", failed.join(", ")));
    }
    let (senate, house) = njclarity_merge(&docs, cfg)?;
    // The aggregate is only as fresh as its slowest county: source_time is
    // the oldest county stamp, and the note carries the full range.
    let source_time = oldest.ok_or("sumber tidak mencantumkan Last-Modified")?;
    let covered: Vec<String> = docs.iter().map(|(c, _)| c.clone()).collect();
    let missing: Vec<&str> = NJ_ALL_COUNTIES
        .iter()
        .filter(|c| !covered.iter().any(|x| x.eq_ignore_ascii_case(c)) && !failed.iter().any(|f| f.to_lowercase().starts_with(&c.to_lowercase())))
        .cloned()
        .collect();
    let mut note = format!(
        "Agregat {} dari 21 kabupaten; belum tercakup: {}. Cap waktu kabupaten {} sampai {} UTC; waktu sumber memakai kabupaten paling lambat. Rasio TPS hanya lingkup kabupaten tercakup",
        covered.len(),
        missing.join(", "),
        oldest.unwrap().format("%H:%M"),
        newest.unwrap().format("%H:%M"),
    );
    if !failed.is_empty() {
        note.push_str(&format!(". Gagal diambil: {}", failed.join(", ")));
    }
    Ok(StateFeed {
        state: cfg.state.clone(),
        source_label: cfg.label.clone(),
        human_url: cfg.human_url.clone(),
        export_url: cfg.export_url.clone(),
        source_time,
        senate,
        house,
        refresh_note: Some(note),
    })
}

/// All 21 New Jersey counties, for coverage reporting.
const NJ_ALL_COUNTIES: &[&str] = &[
    "Atlantic", "Bergen", "Burlington", "Camden", "Cape May", "Cumberland", "Essex",
    "Gloucester", "Hudson", "Hunterdon", "Mercer", "Middlesex", "Monmouth", "Morris",
    "Ocean", "Passaic", "Salem", "Somerset", "Sussex", "Union", "Warren",
];

pub fn njclarity_merge(docs: &[(String, Vec<u8>)], cfg: &SourceCfg) -> Result<(Option<Contest>, Vec<Contest>), String> {
    let sen_re = Regex::new(r"(?i)\b(u\.?\s?s\.?|united states)\s+senat(e|or)\b").map_err(|e| e.to_string())?;
    let house_re = Regex::new(r"(?i)\b(house of rep(resentatives|s)|member of congress)\b").map_err(|e| e.to_string())?;
    let cd_re = Regex::new(r"(?i)\bcd\s*(\d+)").map_err(|e| e.to_string())?;
    let nth_re = Regex::new(r"(?i)(\d+)\s*(?:st|nd|rd|th)\s*cong").map_err(|e| e.to_string())?;
    // Counties whose lone House contest carries no district number and that
    // sit entirely inside one district, per the official Division of
    // Elections documents: congressional-district-by-county-2022.pdf and
    // 2026-official-general-candidates-us-house.pdf list district 2 as
    // "ATLANTIC - CAPE MAY - CUMBERLAND - GLOUCESTER (part) - OCEAN (part) -
    // SALEM" - Cape May and Cumberland whole, unlike the "(part)" counties.
    let county_default: &[(&str, u32)] = &[("Cape_May", 2), ("Cumberland", 2)];
    #[derive(Default)]
    struct CandAgg {
        display: String,
        party: Option<String>,
        votes: u64,
    }
    #[derive(Default)]
    struct Agg {
        candidates: std::collections::BTreeMap<String, CandAgg>,
        pr: u32,
        tp: u32,
    }
    let mut sen: Option<Agg> = None;
    let mut houses: std::collections::BTreeMap<u32, Agg> = std::collections::BTreeMap::new();
    for (county, doc) in docs {
        let g: serde_json::Value = serde_json::from_slice(doc).map_err(|e| format!("respons tidak dikenali: {e}"))?;
        let contests = g["Contests"].as_array().ok_or("daftar kontes tidak dikenali")?;
        for c in contests {
            let name = c["C"].as_str().unwrap_or("").trim();
            enum Which {
                Sen,
                House(u32),
            }
            let which = if sen_re.is_match(name) {
                Which::Sen
            } else if house_re.is_match(name) {
                let d: Option<u32> = cd_re
                    .captures(name)
                    .or_else(|| nth_re.captures(name))
                    .and_then(|c| c[1].parse().ok())
                    .or_else(|| county_default.iter().find(|(c, _)| c.replace('_', " ") == *county).map(|(_, d)| *d));
                match d {
                    Some(d) => Which::House(d),
                    None => continue,
                }
            } else {
                continue;
            };
            let agg = match which {
                Which::Sen => sen.get_or_insert_with(Agg::default),
                Which::House(d) => houses.entry(d).or_default(),
            };
            agg.pr += c["PR"].as_u64().unwrap_or(0) as u32;
            agg.tp += c["TP"].as_u64().unwrap_or(0) as u32;
            let names = c["CH"].as_array().cloned().unwrap_or_default();
            let votes = c["V"].as_array().cloned().unwrap_or_default();
            let parties = c["P"].as_array().cloned().unwrap_or_default();
            for (i, n) in names.iter().enumerate() {
                let raw = n.as_str().unwrap_or("").trim().to_string();
                if raw.is_empty() {
                    continue;
                }
                let party = parties
                    .get(i)
                    .and_then(|p| p.as_str())
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty());
                let (display, prefix_party) = match raw.split_once(' ') {
                    Some((pre, rest))
                        if matches!(pre, "DEM" | "REP" | "NPC" | "NPD" | "IND" | "LIB" | "GRN") && !rest.is_empty() =>
                    {
                        (rest.to_string(), Some(pre.to_string()))
                    }
                    _ => (raw, None),
                };
                let key = display.to_uppercase();
                let v = votes.get(i).and_then(|v| v.as_u64()).unwrap_or(0);
                let e = agg.candidates.entry(key).or_insert_with(|| CandAgg { display: display.clone(), party: None, votes: 0 });
                e.votes += v;
                if e.party.is_none() {
                    e.party = party.or(prefix_party);
                }
                // Prefer mixed-case display over an all-caps variant.
                if e.display.chars().all(|c| !c.is_ascii_lowercase()) && display.chars().any(|c| c.is_ascii_lowercase()) {
                    e.display = display;
                }
            }
        }
    }
    let build = |mut agg: Agg, label: String, district: Option<u32>| -> Option<Contest> {
        if agg.candidates.is_empty() {
            return None;
        }
        // Counties are inconsistent about given names ("BOOKER" next to "Cory
        // BOOKER"). Fold a single-token name into the candidate whose last
        // name it matches.
        let keys: Vec<String> = agg.candidates.keys().cloned().collect();
        for key in &keys {
            if key.contains(' ') {
                continue;
            }
            let targets: Vec<String> = keys
                .iter()
                .filter(|k| k.contains(' ') && k.rsplit(' ').next() == Some(key.as_str()))
                .cloned()
                .collect();
            // Merge only when exactly one candidate carries that surname: a
            // single-token listing is then unambiguously that candidate.
            let target = if targets.len() == 1 { Some(targets[0].clone()) } else { None };
            if let Some(t) = target {
                if let Some(from) = agg.candidates.remove(key) {
                    let e = agg.candidates.get_mut(&t).unwrap();
                    e.votes += from.votes;
                    if e.party.is_none() {
                        e.party = from.party;
                    }
                }
            }
        }
        // Counties that enumerate write-ins list them as party-less names.
        // Real candidates are the ones at least one county gives a party.
        // Only source-labelled write-in rows count as write-ins; any other
        // unresolved name keeps its votes in a separate bucket that can veto
        // a leader declaration, never re-labelled as a write-in by inference.
        let wr_re = Regex::new(r"(?i)^write[-\s]?in\b").ok()?;
        let mut write_in = 0u64;
        let mut unresolved = 0u64;
        let mut candidates: Vec<Candidate> = Vec::new();
        for (_, c) in agg.candidates {
            match c.party {
                Some(p) => candidates.push(Candidate { name: c.display, party: Party::classify(Some(&p)), votes: c.votes }),
                None if wr_re.is_match(&c.display) => write_in += c.votes,
                None => unresolved += c.votes,
            }
        }
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by(|a, b| b.votes.cmp(&a.votes));
        Some(Contest {
            state: cfg.state.clone(),
            label,
            district,
            candidates,
            write_in,
            unresolved,
            precincts: Some((agg.pr, agg.tp)),
            polling_pct: None,
        })
    };
    let senate = sen.and_then(|a| build(a, "Senat AS".into(), None));
    let mut house = Vec::new();
    for (d, agg) in houses {
        if let Some(c) = build(agg, format!("Distrik {d}"), Some(d)) {
            house.push(c);
        }
    }
    Ok((senate, house))
}

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
    if cfg.extra_urls.is_empty() {
        if let Some(Entry { etag: Some(t), feed: Some(_), .. }) = guard.as_ref() {
            req = req.header("If-None-Match", t.clone());
        }
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
        let mut combined: Option<String> = None;
        if cfg.format == "orsos" && !cfg.extra_urls.is_empty() {
            let mut text = String::from_utf8_lossy(&bytes).into_owned();
            for u in &cfg.extra_urls {
                let r2 = client.get(u).send().await.map_err(|e| format!("gagal menghubungi sumber: {e}"))?;
                if !r2.status().is_success() {
                    return Err(format!("sumber membalas HTTP {}", r2.status().as_u16()));
                }
                let b2 = r2.bytes().await.map_err(|e| format!("gagal membaca data: {e}"))?;
                text.push('\n');
                text.push_str(&String::from_utf8_lossy(&b2));
            }
            combined = Some(text);
        }
        let feed = if let Some(t) = &combined {
            parse_orsos(t, cfg)?
        } else if cfg.format == "mdsbe" {
            parse_mdsbe(&String::from_utf8_lossy(&bytes), cfg)?
        } else if cfg.format == "orsos" {
            parse_orsos(&String::from_utf8_lossy(&bytes), cfg)?
        } else if cfg.format == "alsos" {
            parse_alsos(&String::from_utf8_lossy(&bytes), cfg)?
        } else if cfg.format == "hisummary" {
            let t = lm.ok_or_else(|| "sumber tidak mencantumkan Last-Modified".to_string())?;
            parse_hisummary(&String::from_utf8_lossy(&bytes), cfg, t)?
        } else if cfg.format == "vtenr" {
            parse_vtenr(client, &bytes, cfg).await?
        } else if cfg.format == "inenr" {
            parse_inenr(client, &bytes, cfg).await?
        } else if cfg.format == "totalresults" {
            parse_totalresults(client, &bytes, cfg).await?
        } else if cfg.format == "desocrata" {
            parse_desocrata(&bytes, cfg)?
        } else if cfg.format == "njclarity" {
            parse_njclarity(client, &bytes, cfg).await?
        } else if cfg.format == "scxlsx" {
            parse_scxlsx(client, &bytes, cfg).await?
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
