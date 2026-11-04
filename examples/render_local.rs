//! Renders the page from local copies of the feeds (for offline checks).
//! cargo run --example render_local -- va.json ga.json [senat|dpr]
use hitung_midterm::{config::Reference, feed::parse_export, model::FeedResult, render};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let r = Reference::load();
    let mut feeds = Vec::new();
    for (cfg, path) in r.sources.iter().zip([&a[1], &a[2]]) {
        let bytes = std::fs::read(path).unwrap();
        feeds.push(FeedResult::Ok(parse_export(&bytes, cfg).unwrap()));
    }
    let names = r.map.states.iter().map(|(c, st)| (st.name.clone(), c.clone())).collect();
    let wsen = std::fs::read_to_string("/tmp/ex/wsen.txt").unwrap();
    let whou = std::fs::read_to_string("/tmp/ex/whouse.txt").unwrap();
    let proj = hitung_midterm::projection::ProjResult::Ok(hitung_midterm::projection::Projection {
        senate: hitung_midterm::projection::parse_senate(&wsen, &names).unwrap(),
        house: hitung_midterm::projection::parse_house(&whou, &names).unwrap(),
        senate_total: hitung_midterm::projection::parse_totals(&wsen),
        house_total: hitung_midterm::projection::parse_totals(&whou),
        revision_time: chrono::DateTime::parse_from_rfc3339("2026-11-04T07:40:42Z").unwrap().with_timezone(&chrono::Utc),
        senate_rev: 1,
        house_rev: 2,
    });
    let view = if a.get(3).map(|s| s.as_str()) == Some("dpr") { render::View::House } else { render::View::Senate };
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-04T07:28:00Z").unwrap().with_timezone(&chrono::Utc);
    println!("{}", render::page(&r, &feeds, &proj, view, now));
}
