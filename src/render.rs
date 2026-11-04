//! Server-side rendering of the page and the JSON view.
use crate::config::Reference;
use crate::model::*;
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use std::fmt::Write;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Senate,
    House,
}

/// A source whose own timestamp is older than this is flagged as not updating.
pub const STALE_AFTER_MINUTES: i64 = 30;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 2166337 -> "2.166.337"
pub fn num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    out
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "Mei", "Jun", "Jul", "Agu", "Sep", "Okt", "Nov", "Des"];

pub fn wib(t: DateTime<Utc>) -> String {
    let z = FixedOffset::east_opt(7 * 3600).unwrap();
    let l = z.from_utc_datetime(&t.naive_utc());
    use chrono::{Datelike, Timelike};
    format!("{} {} {} {:02}.{:02} WIB", l.day(), MONTHS[l.month0() as usize], l.year(), l.hour(), l.minute())
}

pub fn age(now: DateTime<Utc>, t: DateTime<Utc>) -> String {
    let m = (now - t).num_minutes().max(0);
    if m < 1 {
        "baru saja".into()
    } else if m < 60 {
        format!("{m} menit lalu")
    } else {
        let h = m / 60;
        let r = m % 60;
        if r == 0 {
            format!("{h} jam lalu")
        } else {
            format!("{h} jam {r} menit lalu")
        }
    }
}

pub fn is_stale(now: DateTime<Utc>, t: DateTime<Utc>) -> bool {
    (now - t).num_minutes() > STALE_AFTER_MINUTES
}

fn pcls(p: Party) -> &'static str {
    match p {
        Party::Dem => "d",
        Party::Rep => "r",
        Party::Other => "o",
    }
}

#[derive(Default, Clone, Copy)]
pub struct Tally {
    pub dem: u32,
    pub rep: u32,
    pub other: u32,
    pub tie: u32,
    pub novotes: u32,
}
impl Tally {
    pub fn add(&mut self, l: Lead) {
        match l {
            Lead::Ahead(Party::Dem) => self.dem += 1,
            Lead::Ahead(Party::Rep) => self.rep += 1,
            Lead::Ahead(Party::Other) => self.other += 1,
            Lead::Tie => self.tie += 1,
            Lead::NoVotes => self.novotes += 1,
        }
    }
    pub fn with_data(&self) -> u32 {
        self.dem + self.rep + self.other + self.tie + self.novotes
    }
}

fn feeds_ok(feeds: &[FeedResult]) -> impl Iterator<Item = &StateFeed> {
    feeds.iter().filter_map(|f| if let FeedResult::Ok(s) = f { Some(s) } else { None })
}

pub fn senate_tally(feeds: &[FeedResult]) -> Tally {
    let mut t = Tally::default();
    for f in feeds_ok(feeds) {
        if let Some(c) = &f.senate {
            t.add(c.lead());
        }
    }
    t
}
pub fn house_tally(feeds: &[FeedResult]) -> Tally {
    let mut t = Tally::default();
    for f in feeds_ok(feeds) {
        for c in &f.house {
            t.add(c.lead());
        }
    }
    t
}

fn lead_text(l: Lead) -> &'static str {
    match l {
        Lead::Ahead(Party::Dem) => "Demokrat unggul sementara",
        Lead::Ahead(Party::Rep) => "Republik unggul sementara",
        Lead::Ahead(Party::Other) => "Calon lain unggul sementara",
        Lead::Tie => "Suara imbang",
        Lead::NoVotes => "Belum ada suara",
    }
}

fn cand_line(c: &Candidate) -> String {
    format!(
        "<span class=\"cand\"><i class=\"sw {}\"></i>{} <small>({})</small> <b>{}</b></span>",
        pcls(c.party),
        esc(&c.name),
        c.party.label(),
        num(c.votes)
    )
}

fn precinct_line(c: &Contest) -> String {
    match c.precincts {
        Some((r, p)) => format!("TPS melapor {} dari {}", num(r as u64), num(p as u64)),
        None => "Jumlah TPS melapor: Tidak tersedia".into(),
    }
}

fn map_svg(reference: &Reference, feeds: &[FeedResult], view: View) -> String {
    let m = &reference.map;
    let mut o = String::new();
    let _ = write!(
        o,
        "<svg class=\"map\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"Peta negara bagian AS\">",
        m.width, m.height
    );
    let feed_of = |st: &str| feeds_ok(feeds).find(|f| f.state == st);
    for (code, s) in &m.states {
        let (cls, tip) = match view {
            View::Senate => {
                if !reference.in_senate_schedule(code) {
                    ("off", format!("{}: tidak ada pemilihan Senat", s.name))
                } else if let Some(c) = feed_of(code).and_then(|f| f.senate.as_ref()) {
                    let cls = match c.lead() {
                        Lead::Ahead(p) => pcls(p),
                        Lead::Tie => "tie",
                        Lead::NoVotes => "none",
                    };
                    (cls, format!("{}: {}", s.name, lead_text(c.lead())))
                } else {
                    ("none", format!("{}: data belum tersedia dari sumber resmi yang tersambung", s.name))
                }
            }
            View::House => match feed_of(code) {
                Some(f) if !f.house.is_empty() => (
                    "has",
                    format!("{}: {} dari {} kursi sudah ada data", s.name, f.house.len(), reference.house_seats.get(code).copied().unwrap_or(0)),
                ),
                _ => ("none", format!("{}: data belum tersedia dari sumber resmi yang tersambung", s.name)),
            },
        };
        let _ = write!(o, "<path class=\"st {cls}\" d=\"{}\"><title>{}</title></path>", s.d, esc(&tip));
    }
    for (code, s) in &m.states {
        let off = view == View::Senate && !reference.in_senate_schedule(code);
        let _ = write!(o, "<text class=\"lb{}\" x=\"{}\" y=\"{}\">{}</text>", if off { " off" } else { "" }, s.cx, s.cy + 3.5, code);
        if view == View::House {
            let total = reference.house_seats.get(code).copied().unwrap_or(0);
            let have = feed_of(code).map(|f| f.house.len()).unwrap_or(0);
            let _ = write!(o, "<text class=\"lb2\" x=\"{}\" y=\"{}\">{}/{}</text>", s.cx, s.cy + 13.5, have, total);
        }
    }
    o.push_str("</svg>");
    o
}

fn seat_bar(segments: &[(&str, u32)], total: u32) -> String {
    let mut o = String::from("<div class=\"bar\" role=\"img\" aria-label=\"Kursi yang sudah ada data\">");
    for (cls, n) in segments {
        if *n > 0 && total > 0 {
            let _ = write!(o, "<i class=\"{cls}\" style=\"flex:{n}\"></i>");
        }
    }
    o.push_str("</div>");
    o
}

pub fn page(reference: &Reference, feeds: &[FeedResult], view: View, now: DateTime<Utc>) -> String {
    let senate_total = reference.senate.len() as u32;
    let house_total = reference.house_total();
    let st = senate_tally(feeds);
    let ht = house_tally(feeds);
    let ok: Vec<&StateFeed> = feeds_ok(feeds).collect();
    let connected = ok.iter().map(|f| f.state.as_str()).collect::<Vec<_>>().join(", ");
    let newest = ok.iter().map(|f| f.source_time).max();
    let name_of = |code: &str| reference.map.states.get(code).map(|s| s.name.clone()).unwrap_or(code.to_string());

    let mut o = String::new();
    o.push_str("<!doctype html><html lang=\"id\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    o.push_str("<title>Hitung Midterm - hasil pemilu sela AS</title>");
    o.push_str("<meta name=\"description\" content=\"Hitungan sementara pemilu sela AS 3 November 2026 dari situs hasil resmi negara bagian. Belum resmi.\">");
    o.push_str("<meta http-equiv=\"refresh\" content=\"300\"><style>");
    o.push_str(CSS);
    o.push_str("</style></head><body><div class=\"wrap\">");

    // header
    let _ = write!(
        o,
        "<header><b class=\"brand\">Hitung Midterm</b><span class=\"mute\">Halaman dibuat {}</span></header>",
        wib(now)
    );
    let title = if view == View::Senate { "Hasil Senat AS" } else { "Hasil DPR AS" };
    let _ = write!(o, "<h1>{title}</h1><p class=\"sub\">Pemilu sela AS, 3 November 2026. Hitungan sementara dari situs hasil resmi negara bagian, belum disertifikasi.</p>");
    let _ = write!(
        o,
        "<nav class=\"tabs\"><a href=\"/?view=senat\" class=\"{}\">Senat</a><a href=\"/?view=dpr\" class=\"{}\">DPR</a></nav>",
        if view == View::Senate { "on" } else { "" },
        if view == View::House { "on" } else { "" }
    );

    o.push_str("<div class=\"grid\"><section class=\"mapcol\">");
    o.push_str(&map_svg(reference, feeds, view));
    // legend
    o.push_str("<ul class=\"legend\">");
    if view == View::Senate {
        o.push_str("<li><i class=\"sw d lead\"></i><i class=\"sw r lead\"></i><i class=\"sw o lead\"></i>Unggul sementara: urutan suara terbanyak sejauh ini, bukan hasil akhir</li>");
        o.push_str("<li><i class=\"sw tie\"></i>Suara imbang</li>");
        o.push_str("<li><i class=\"sw none\"></i>Ada pemilihan, data belum tersedia dari sumber yang tersambung</li>");
        o.push_str("<li><i class=\"sw off\"></i>Tidak ada pemilihan Senat (dari jadwal)</li>");
    } else {
        o.push_str("<li><i class=\"sw has\"></i>Ada data kursi dari sumber resmi negara bagian</li>");
        o.push_str("<li><i class=\"sw none\"></i>Data belum tersedia dari sumber yang tersambung</li>");
        o.push_str("<li class=\"mute\">n/total: kursi yang sudah ada data / jumlah kursi negara bagian</li>");
    }
    o.push_str("<li class=\"mute\">Proyeksi menang: belum ada sumber proyeksi yang tersambung, jadi tidak ditampilkan.</li></ul></section>");

    // side panel
    o.push_str("<aside class=\"panel\">");
    if view == View::Senate {
        let _ = write!(o, "<div class=\"box\"><b>{} dari {} pemilihan kursi Senat sudah ada data</b>", st.with_data(), senate_total);
        let n = senate_total;
        let nodata = n.saturating_sub(st.with_data());
        o.push_str(&seat_bar(&[("d lead", st.dem), ("r lead", st.rep), ("o lead", st.other), ("tie", st.tie), ("none", st.novotes + nodata)], n));
    } else {
        let _ = write!(o, "<div class=\"box\"><b>{} dari {} kursi DPR sudah ada data</b>", ht.with_data(), house_total);
        let nodata = house_total.saturating_sub(ht.with_data());
        o.push_str(&seat_bar(&[("d lead", ht.dem), ("r lead", ht.rep), ("o lead", ht.other), ("tie", ht.tie), ("none", ht.novotes + nodata)], house_total));
    }
    let t = if view == View::Senate { st } else { ht };
    let _ = write!(
        o,
        "<p class=\"stat\"><b>Proyeksi menang</b><br><span class=\"mute\">Tidak tersedia</span></p><p class=\"stat\"><b>Unggul sementara</b><br><span class=\"mute\">Demokrat {} | Republik {} | Lainnya {}{}</span></p><p class=\"stat\"><b>Belum ada data</b><br><span class=\"mute\">{}</span></p></div>",
        t.dem,
        t.rep,
        t.other,
        if t.tie > 0 { format!(" | Imbang {}", t.tie) } else { String::new() },
        if view == View::Senate { senate_total.saturating_sub(t.with_data()) } else { house_total.saturating_sub(t.with_data()) }
    );
    let _ = write!(
        o,
        "<div class=\"box dashed\"><b>{} dan total nasional: Tidak tersedia</b><br><span class=\"mute\">Hanya {} negara bagian yang tersambung ke sumber resmi ({}). Total nasional dan kendali kamar tidak dihitung dari data sebagian.</span></div>",
        if view == View::Senate { "Kendali Senat" } else { "Kendali DPR" },
        ok.len(),
        if connected.is_empty() { "belum ada".to_string() } else { connected }
    );

    // table
    if view == View::Senate {
        o.push_str("<h2>Pemilihan Senat dengan data</h2><div class=\"rows\">");
        for f in &ok {
            if let Some(c) = &f.senate {
                let kind = reference.senate.iter().find(|e| e.state == f.state).map(|e| e.kind.as_str()).unwrap_or("regular");
                let stale = is_stale(now, f.source_time);
                let _ = write!(
                    o,
                    "<div class=\"row\"><div class=\"rh\"><b>{}</b>{}<span class=\"tag\">{}</span></div>",
                    esc(&name_of(&f.state)),
                    if kind == "special" { " <small class=\"mute\">(pemilihan khusus)</small>" } else { "" },
                    lead_text(c.lead())
                );
                for cand in &c.candidates {
                    o.push_str(&cand_line(cand));
                }
                if c.write_in > 0 {
                    let _ = write!(o, "<span class=\"cand mute\">Tulis tangan (write-in) <b>{}</b></span>", num(c.write_in));
                }
                if let Some(mg) = c.margin() {
                    let _ = write!(o, "<small class=\"mute\">Selisih {} suara. {}.</small>", num(mg), precinct_line(c));
                }
                let _ = write!(o, "{}</div>", source_note(f, now, stale));
            }
        }
        for f in feeds.iter() {
            if let FeedResult::Failed { state, source_label, reason, .. } = f {
                let _ = write!(o, "<div class=\"row warn\"><b>{}</b> <span class=\"tag\">Gagal diperbarui</span><small class=\"mute\">{}: {}. Angka tidak ditampilkan sampai sumber pulih.</small></div>", esc(&name_of(state)), esc(source_label), esc(reason));
            }
        }
        o.push_str("</div>");
        let missing: Vec<String> = reference
            .senate
            .iter()
            .filter(|e| !ok.iter().any(|f| f.state == e.state && f.senate.is_some()))
            .map(|e| e.state.clone())
            .collect();
        let _ = write!(o, "<details class=\"miss\"><summary>{} pemilihan lain belum ada datanya</summary><p class=\"mute\">{}</p></details>", missing.len(), missing.join(", "));
    } else {
        o.push_str("<h2>Kursi DPR dengan data</h2><div class=\"rows\">");
        for f in &ok {
            if f.house.is_empty() {
                continue;
            }
            let total = reference.house_seats.get(&f.state).copied().unwrap_or(0);
            let mut t = Tally::default();
            for c in &f.house {
                t.add(c.lead());
            }
            let stale = is_stale(now, f.source_time);
            let _ = write!(
                o,
                "<div class=\"row\"><div class=\"rh\"><b>{}</b><span class=\"tag\">{} dari {} kursi</span></div><span class=\"mute\">Unggul sementara: Demokrat {} | Republik {} | Lainnya {}</span>",
                esc(&name_of(&f.state)),
                f.house.len(),
                total,
                t.dem,
                t.rep,
                t.other
            );
            o.push_str("<details><summary>Rincian per distrik</summary>");
            for c in &f.house {
                let _ = write!(o, "<div class=\"dist\"><b>{}</b>", esc(&c.label));
                for cand in c.candidates.iter().take(3) {
                    o.push_str(&cand_line(cand));
                }
                o.push_str("</div>");
            }
            o.push_str("</details>");
            let _ = write!(o, "{}</div>", source_note(f, now, stale));
        }
        o.push_str("</div>");
        let missing: Vec<String> = reference
            .house_seats
            .keys()
            .filter(|s| !ok.iter().any(|f| &f.state == *s && !f.house.is_empty()))
            .cloned()
            .collect();
        let _ = write!(o, "<details class=\"miss\"><summary>{} negara bagian belum ada datanya</summary><p class=\"mute\">{}</p></details>", missing.len(), missing.join(", "));
    }

    // footer
    o.push_str("</aside></div><footer>");
    let _ = write!(
        o,
        "<p>Sumber: situs hasil resmi kantor pemilu negara bagian (tautan di tiap baris). Semua angka berstatus belum resmi dan bisa berubah sampai disertifikasi.</p><p>Waktu sumber = waktu yang dicap penerbit pada berkas data. Waktu ambil = saat halaman ini dibuat: {}.{}</p><p>Jadwal kursi Senat: Ballotpedia. Jumlah kursi DPR: Sensus AS 2020. Situs ini tidak memberi proyeksi atau penetapan pemenang.</p>",
        wib(now),
        match newest {
            Some(t) => format!(" Data sumber terbaru berstempel {}.", wib(t)),
            None => " Belum ada sumber yang berhasil dibaca.".into(),
        }
    );
    o.push_str("</footer></div></body></html>");
    o
}

fn source_note(f: &StateFeed, now: DateTime<Utc>, stale: bool) -> String {
    format!(
        "<small class=\"src{}\">Sumber: <a href=\"{}\" rel=\"noopener\">{}</a>. Waktu sumber {} ({}).{}</small>",
        if stale { " stale" } else { "" },
        esc(&f.human_url),
        esc(&f.source_label),
        wib(f.source_time),
        age(now, f.source_time),
        if stale { " Sumber belum memperbarui data lebih dari 30 menit." } else { "" }
    )
}

pub fn json(reference: &Reference, feeds: &[FeedResult], now: DateTime<Utc>) -> String {
    let sources: Vec<serde_json::Value> = feeds
        .iter()
        .map(|f| match f {
            FeedResult::Ok(s) => serde_json::json!({
                "state": s.state, "status": "ok", "source": s.source_label, "page": s.human_url,
                "source_time": s.source_time, "stale": is_stale(now, s.source_time),
                "senate": s.senate, "house": s.house,
            }),
            FeedResult::Failed { state, source_label, human_url, reason } => serde_json::json!({
                "state": state, "status": "failed", "source": source_label, "page": human_url, "reason": reason,
            }),
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "fetched_at": now,
        "note": "Unofficial vote counts from state result feeds. No race calls or projections.",
        "senate_contests_scheduled": reference.senate.len(),
        "house_seats_total": reference.house_total(),
        "sources": sources,
    }))
    .unwrap_or_else(|_| "{}".into())
}

const CSS: &str = r#"
@font-face{font-family:"Public Sans";src:url(/PublicSans.ttf) format("truetype");font-weight:100 900;font-display:swap}
:root{--bg:#17171A;--ink:#EDE9E1;--mute:#A09C92;--line:#3A3A40;--dash:#5A5A62;--blue:#3B6FE0;--red:#D8453F;--purp:#8E6BB8}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:400 15px/1.45 "Public Sans",system-ui,sans-serif}
.wrap{max-width:1440px;margin:0 auto;padding:0 20px 48px}
header{display:flex;justify-content:space-between;align-items:center;padding:20px 0 14px;border-bottom:1px solid var(--line)}
.brand{font-weight:700;font-size:15px}.mute{color:var(--mute)}header .mute{font-size:12px}
h1{font-size:30px;line-height:1.1;margin:26px 0 6px;font-weight:700}
.sub{margin:0 0 18px;color:var(--mute);font-size:13px;max-width:70ch}
.tabs{display:flex;margin-bottom:22px}
.tabs a{padding:8px 28px;border:1px solid var(--line);color:var(--mute);text-decoration:none;font-weight:700;font-size:14px}
.tabs a.on{background:var(--ink);color:var(--bg)}
.grid{display:grid;grid-template-columns:1fr;gap:24px}
@media(min-width:960px){h1{font-size:44px}.grid{grid-template-columns:minmax(0,1.5fr) minmax(340px,1fr)}.wrap{padding:0 48px 64px}}
.map{width:100%;height:auto;display:block}
.st{stroke:var(--bg);stroke-width:1;stroke-linejoin:round;fill:var(--line)}
.st.off{fill:none;stroke:var(--dash);stroke-dasharray:3 3}
.st.d{fill:#2A3F73;stroke:var(--blue);stroke-width:1.5}.st.r{fill:#6E2B2A;stroke:var(--red);stroke-width:1.5}
.st.o{fill:#44335C;stroke:var(--purp);stroke-width:1.5}.st.tie{fill:#55524C;stroke:var(--ink);stroke-dasharray:4 2}
.st.has{fill:#4A4A52}
.lb{font:700 10.5px "Public Sans",sans-serif;fill:var(--mute);text-anchor:middle;pointer-events:none}.lb.off{fill:var(--dash)}
.lb2{font:400 9px "Public Sans",sans-serif;fill:var(--mute);text-anchor:middle;pointer-events:none}
.legend{list-style:none;padding:0;margin:14px 0 0;font-size:13px;display:grid;gap:6px}
.sw{display:inline-block;width:14px;height:14px;margin-right:5px;vertical-align:-2px;background:var(--line)}
.sw.d{background:var(--blue)}.sw.r{background:var(--red)}.sw.o{background:var(--purp)}
.sw.d.lead{background:#2A3F73;box-shadow:inset 0 0 0 2px var(--blue)}.sw.r.lead{background:#6E2B2A;box-shadow:inset 0 0 0 2px var(--red)}.sw.o.lead{background:#44335C;box-shadow:inset 0 0 0 2px var(--purp)}
.sw.tie{background:#55524C}.sw.off{background:none;border:1px dashed var(--dash)}.sw.has{background:#4A4A52}
.box{border:1px solid var(--line);padding:16px 18px;margin-bottom:14px}.box.dashed{border:1px dashed var(--dash);font-size:13px}
.bar{display:flex;gap:2px;height:22px;margin:10px 0}.bar i{display:block;background:var(--line);min-width:3px}
.bar i.d{background:#2A3F73;box-shadow:inset 0 0 0 2px var(--blue)}.bar i.r{background:#6E2B2A;box-shadow:inset 0 0 0 2px var(--red)}
.bar i.o{background:#44335C;box-shadow:inset 0 0 0 2px var(--purp)}.bar i.tie{background:#55524C}
.stat{margin:8px 0;font-size:13px}
h2{font-size:11px;letter-spacing:.1em;text-transform:uppercase;color:var(--mute);margin:26px 0 0;padding-bottom:8px;border-bottom:1px solid var(--line)}
.row{padding:12px 0;border-bottom:1px solid var(--line);display:flex;flex-direction:column;gap:3px}
.rh{display:flex;justify-content:space-between;gap:10px;align-items:baseline}
.tag{border:1px solid var(--dash);padding:1px 8px;font-size:11px;font-weight:700;color:var(--mute);white-space:nowrap}
.cand{font-size:14px}.cand b{float:right}.cand small{color:var(--mute)}
.src{color:var(--mute);font-size:11.5px;margin-top:4px}.src a{color:var(--mute)}.src.stale{color:#E0B050}
.row.warn .tag{border-color:var(--red);color:var(--ink)}
details{font-size:13px;color:var(--mute)}summary{cursor:pointer;margin:6px 0}
.dist{display:flex;flex-direction:column;padding:6px 0;border-top:1px solid var(--line)}
.miss{margin-top:14px}
footer{margin-top:36px;padding-top:14px;border-top:1px solid var(--line);color:var(--mute);font-size:12px;max-width:80ch}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn number_format() {
        assert_eq!(num(0), "0");
        assert_eq!(num(999), "999");
        assert_eq!(num(1000), "1.000");
        assert_eq!(num(2166337), "2.166.337");
    }
    #[test]
    fn wib_time() {
        let t = Utc.with_ymd_and_hms(2026, 11, 4, 7, 25, 0).unwrap();
        assert_eq!(wib(t), "4 Nov 2026 14.25 WIB");
    }
    #[test]
    fn stale_rule() {
        let t = Utc.with_ymd_and_hms(2026, 11, 4, 4, 45, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 7, 27, 0).unwrap();
        assert!(is_stale(now, t));
        assert_eq!(age(now, t), "2 jam 42 menit lalu");
        assert!(!is_stale(t + chrono::Duration::minutes(10), t));
    }
}
