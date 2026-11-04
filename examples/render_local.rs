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
    let view = if a.get(3).map(|s| s.as_str()) == Some("dpr") { render::View::House } else { render::View::Senate };
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-04T07:28:00Z").unwrap().with_timezone(&chrono::Utc);
    println!("{}", render::page(&r, &feeds, view, now));
}
