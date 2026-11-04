//! Server-side rendering of the page and the JSON view.
use crate::config::Reference;
use crate::model::*;
use crate::projection::*;
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

fn map_svg(reference: &Reference, feeds: &[FeedResult], proj: Option<&Projection>, view: View) -> String {
    let m = &reference.map;
    let mut o = String::new();
    let _ = write!(
        o,
        "<svg class=\"map\" viewBox=\"0 0 {} {}\" role=\"img\" aria-label=\"Peta negara bagian AS\">",
        m.width, m.height
    );
    let feed_of = |st: &str| feeds_ok(feeds).find(|f| f.state == st);
    if view == View::House {
        // One hard-stop gradient per state with data: bands are proportional to
        // seats (of the state's total) by who leads, and the remainder is grey
        // for seats without data. Bands are a seat split, not a state call.
        o.push_str("<defs>");
        for code in m.states.keys() {
            let total = reference.house_seats.get(code).copied().unwrap_or(0);
            let official = feed_of(code).filter(|f| !f.house.is_empty() && total > 0);
            if official.is_none() {
                if let Some(&(d, r)) = proj.and_then(|p| p.house.get(code)) {
                    if total > 0 {
                        let parts = [(d, "#3B6FE0"), (r, "#D8453F"), (total.saturating_sub(d + r), "#3A3A40")];
                        let _ = write!(o, "<linearGradient id=\"g-{code}\" x1=\"0\" x2=\"1\" y1=\"0\" y2=\"0\">");
                        let mut acc = 0u32;
                        for (n, col) in parts {
                            if n == 0 {
                                continue;
                            }
                            let a = acc as f64 * 100.0 / total as f64;
                            acc += n;
                            let b = acc as f64 * 100.0 / total as f64;
                            let _ = write!(o, "<stop offset=\"{a:.2}%\" stop-color=\"{col}\"/><stop offset=\"{b:.2}%\" stop-color=\"{col}\"/>");
                        }
                        o.push_str("</linearGradient>");
                    }
                }
            }
            if let Some(f) = official {
                let mut t = Tally::default();
                for c in &f.house {
                    t.add(c.lead());
                }
                let have = f.house.len() as u32;
                let nodata = total.saturating_sub(have) + t.novotes;
                let parts = [(t.dem, "#2A3F73"), (t.rep, "#6E2B2A"), (t.other, "#44335C"), (t.tie, "#55524C"), (nodata, "#3A3A40")];
                let _ = write!(o, "<linearGradient id=\"g-{code}\" x1=\"0\" x2=\"1\" y1=\"0\" y2=\"0\">");
                let mut acc = 0u32;
                for (n, col) in parts {
                    if n == 0 {
                        continue;
                    }
                    let a = acc as f64 * 100.0 / total as f64;
                    acc += n;
                    let b = acc as f64 * 100.0 / total as f64;
                    let _ = write!(o, "<stop offset=\"{a:.2}%\" stop-color=\"{col}\"/><stop offset=\"{b:.2}%\" stop-color=\"{col}\"/>");
                }
                o.push_str("</linearGradient>");
            }
        }
        o.push_str("</defs>");
    }
    for (code, s) in &m.states {
        let (cls, tip) = match view {
            View::Senate => {
                if !reference.in_senate_schedule(code) {
                    ("off", format!("{}: tidak ada pemilihan Senat", s.name))
                } else if let Some(pp) = proj.and_then(|p| p.senate.get(code)) {
                    let off = feed_of(code).and_then(|f| f.senate.as_ref()).map(|c| c.lead());
                    let conflict = matches!(off, Some(Lead::Ahead(q)) if q != *pp);
                    (
                        match (pp, conflict) {
                            (Party::Dem, false) => "pd",
                            (Party::Rep, false) => "pr",
                            (Party::Other, false) => "po",
                            (Party::Dem, true) => "pd conflict",
                            (Party::Rep, true) => "pr conflict",
                            (Party::Other, true) => "po conflict",
                        },
                        format!(
                            "{}: {} entri pemenang di Wikipedia (belum diverifikasi){}",
                            s.name,
                            pp.label(),
                            if conflict { ". Hitungan resmi sementara menunjukkan pemimpin berbeda" } else { "" }
                        ),
                    )
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
                Some(f) if !f.house.is_empty() => {
                    let mut t = Tally::default();
                    for c in &f.house {
                        t.add(c.lead());
                    }
                    (
                        "band",
                        format!(
                            "{}: {} dari {} kursi ada data. Unggul sementara: Demokrat {}, Republik {}, Lainnya {}",
                            s.name,
                            f.house.len(),
                            reference.house_seats.get(code).copied().unwrap_or(0),
                            t.dem,
                            t.rep,
                            t.other
                        ),
                    )
                }
                _ => match proj.and_then(|p| p.house.get(code)) {
                    Some(&(d, r)) => (
                        "band",
                        format!("{}: entri pemenang di Wikipedia (belum diverifikasi): Demokrat {}, Republik {} dari {} kursi", s.name, d, r, reference.house_seats.get(code).copied().unwrap_or(0)),
                    ),
                    None => ("none", format!("{}: data belum tersedia", s.name)),
                },
            },
        };
        let fill = if cls == "band" { format!(" style=\"fill:url(#g-{code})\"") } else { String::new() };
        let _ = write!(o, "<path class=\"st {cls}\"{fill} d=\"{}\"><title>{}</title></path>", s.d, esc(&tip));
    }
    for (code, s) in &m.states {
        let off = view == View::Senate && !reference.in_senate_schedule(code);
        let _ = write!(o, "<text class=\"lb{}\" x=\"{}\" y=\"{}\">{}</text>", if off { " off" } else { "" }, s.cx, s.cy + 3.5, code);
        if view == View::House {
            let total = reference.house_seats.get(code).copied().unwrap_or(0);
            let have = match feed_of(code).filter(|f| !f.house.is_empty()) {
                Some(f) => f.house.len(),
                None => proj.and_then(|p| p.house.get(code)).map(|&(d, r)| (d + r) as usize).unwrap_or(0),
            };
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

pub fn page(reference: &Reference, feeds: &[FeedResult], projr: &ProjResult, view: View, now: DateTime<Utc>) -> String {
    let proj: Option<&Projection> = match projr {
        ProjResult::Ok(p) => Some(p),
        ProjResult::Failed(_) => None,
    };
    let senate_total = reference.senate.len() as u32;
    let house_total = reference.house_total();
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
    let _ = write!(o, "<h1>{title}</h1><p class=\"sub\">Pemilu sela AS, 3 November 2026. Hitungan sementara dari situs hasil resmi negara bagian. Keduanya belum resmi dan ditandai terpisah.</p>");
    let _ = write!(
        o,
        "<nav class=\"tabs\"><a href=\"/?view=senat\" class=\"{}\">Senat</a><a href=\"/?view=dpr\" class=\"{}\">DPR</a></nav>",
        if view == View::Senate { "on" } else { "" },
        if view == View::House { "on" } else { "" }
    );

    o.push_str("<div class=\"grid\"><section class=\"mapcol\">");
    o.push_str(&map_svg(reference, feeds, proj, view));
    // legend
    o.push_str("<ul class=\"legend\">");
    if view == View::Senate {
        o.push_str("<li><i class=\"sw d lead\"></i><i class=\"sw r lead\"></i><i class=\"sw o lead\"></i>Unggul sementara: urutan suara terbanyak sejauh ini, bukan hasil akhir</li>");
        o.push_str("<li><i class=\"sw tie\"></i>Suara imbang</li>");
        o.push_str("<li><i class=\"sw none\"></i>Ada pemilihan, data belum tersedia dari sumber yang tersambung</li>");
        o.push_str("<li><i class=\"sw off\"></i>Tidak ada pemilihan Senat (dari jadwal)</li>");
    } else {
        o.push_str("<li><i class=\"sw d lead\"></i><i class=\"sw r lead\"></i><i class=\"sw o lead\"></i>Pita warna = pembagian kursi yang unggul sementara, sebanding dengan jumlah kursi negara bagian</li>");
        o.push_str("<li><i class=\"sw none\"></i>Abu-abu di dalam pita = kursi tanpa hitungan resmi dan tanpa proyeksi</li>");
        o.push_str("<li><i class=\"sw none\"></i>Data belum tersedia dari sumber yang tersambung</li>");
        o.push_str("<li class=\"mute\">n/total: kursi dengan hitungan resmi / jumlah kursi negara bagian (proyeksi tidak masuk n)</li>");
    }
    if proj.is_some() {
        if view == View::Senate {
            o.push_str("<li><i class=\"sw pd\"></i><i class=\"sw pr\"></i><i class=\"sw po\"></i>Entri pemenang di Wikipedia: belum diverifikasi ke sumber medianya, bukan hitungan resmi</li>");
            o.push_str("<li><i class=\"sw d lead conflict\"></i>Garis putus-putus putih: hitungan resmi sementara menunjuk pemimpin berbeda dari proyeksi</li>");
        } else {
            o.push_str("<li><i class=\"sw pd\"></i><i class=\"sw pr\"></i>Pita warna terang = kursi yang sudah tercatat menang di Wikipedia (belum diverifikasi). Pita gelap = unggul sementara dari hitungan resmi. Negara bagian dengan hitungan resmi memakai pita gelap.</li>");
        }
    } else {
    }
    o.push_str("</ul></section>");

    // side panel
    o.push_str("<aside class=\"panel\">");
    // per-state precedence: projection first, then official lead, then nothing
    let (mut pj, mut ol, mut nod) = (Tally::default(), Tally::default(), 0u32);
    let (mut hp_d, mut hp_r, mut hp_states, mut h_proj_seats) = (0u32, 0u32, 0u32, 0u32);
    if view == View::Senate {
        for e in &reference.senate {
            let off = ok.iter().find(|f| f.state == e.state).and_then(|f| f.senate.as_ref());
            match (proj.and_then(|p| p.senate.get(&e.state)), off) {
                (Some(p), _) => match p {
                    Party::Dem => pj.dem += 1,
                    Party::Rep => pj.rep += 1,
                    Party::Other => pj.other += 1,
                },
                (None, Some(c)) => ol.add(c.lead()),
                (None, None) => nod += 1,
            }
        }
        let have = senate_total - nod;
        let _ = write!(o, "<div class=\"box\"><b>{} dari {} pemilihan kursi Senat sudah ada hitungan resmi</b>", have, senate_total);
        o.push_str(&seat_bar(
            &[("pd", pj.dem), ("pr", pj.rep), ("po", pj.other), ("d lead", ol.dem), ("r lead", ol.rep), ("o lead", ol.other), ("tie", ol.tie), ("none", ol.novotes + nod)],
            senate_total,
        ));
        if proj.is_some() {
        let _ = write!(
            o,
            "<p class=\"stat\"><b>Entri pemenang Wikipedia</b> <small class=\"mute\">(belum diverifikasi)</small><br><span class=\"mute\">{}</span></p>",
            if proj.is_some() { format!("Demokrat {} | Republik {} | Lainnya {}", pj.dem, pj.rep, pj.other) } else { "Tidak tersedia".to_string() }
        );
        }
        let _ = write!(o, "<p class=\"stat\"><b>Unggul sementara</b> <small class=\"mute\">(hitungan resmi sementara)</small><br><span class=\"mute\">Demokrat {} | Republik {} | Lainnya {}{}</span></p><p class=\"stat\"><b>Belum ada data</b><br><span class=\"mute\">{}</span></p></div>", ol.dem, ol.rep, ol.other, if ol.tie > 0 { format!(" | Imbang {}", ol.tie) } else { String::new() }, nod + ol.novotes);
    } else {
        for (code, total) in &reference.house_seats {
            let off = ok.iter().find(|f| &f.state == code && !f.house.is_empty());
            if off.is_none() {
                if let Some(&(d, r)) = proj.and_then(|p| p.house.get(code)) {
                    hp_d += d;
                    hp_r += r;
                    hp_states += 1;
                    h_proj_seats += (d + r).min(*total);
                }
            }
        }
        let have = ht.with_data() + h_proj_seats;
        let _ = write!(o, "<div class=\"box\"><b>{} dari {} kursi DPR sudah ada hitungan resmi</b>", have, house_total);
        let nodata = house_total.saturating_sub(have) + ht.novotes;
        o.push_str(&seat_bar(&[("pd", hp_d), ("pr", hp_r), ("d lead", ht.dem), ("r lead", ht.rep), ("o lead", ht.other), ("tie", ht.tie), ("none", nodata)], house_total));
        if proj.is_some() {
        let _ = write!(
            o,
            "<p class=\"stat\"><b>Entri pemenang Wikipedia</b> <small class=\"mute\">(belum diverifikasi; {} negara bagian tanpa hitungan resmi di sini)</small><br><span class=\"mute\">{}</span></p>",
            hp_states,
            if proj.is_some() { format!("Demokrat {} | Republik {}", hp_d, hp_r) } else { "Tidak tersedia".to_string() }
        );
        }
        let _ = write!(o, "<p class=\"stat\"><b>Unggul sementara</b> <small class=\"mute\">(hitungan resmi sementara)</small><br><span class=\"mute\">Demokrat {} | Republik {} | Lainnya {}{}</span></p><p class=\"stat\"><b>Belum ada data</b><br><span class=\"mute\">{}</span></p></div>", ht.dem, ht.rep, ht.other, if ht.tie > 0 { format!(" | Imbang {}", ht.tie) } else { String::new() }, nodata);
    }
    let chamber = if view == View::Senate { "Senat" } else { "DPR" };
    let _ = write!(
        o,
        "<div class=\"box dashed\"><b>Total nasional {chamber}</b><br><span class=\"mute\">Tidak ditampilkan. Situs ini baru memuat hitungan resmi dari beberapa negara bagian, jadi belum ada dasar untuk total kamar. Angka di atas hanya jumlah pemilihan yang sudah ada hitungan resmi.</span><br><span class=\"mute\">Kendali kamar tidak disimpulkan oleh situs ini. Hitungan resmi baru tersambung dari {} negara bagian ({}).</span></div>",
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
                if let Some(p) = proj.and_then(|p| p.senate.get(&f.state)) {
                    let conflict = matches!(c.lead(), Lead::Ahead(q) if q != *p);
                    let _ = write!(
                        o,
                        "<small class=\"mute\">Entri pemenang di Wikipedia (belum diverifikasi): {}.{}</small>",
                        p.label(),
                        if conflict { " Tidak sinkron dengan hitungan resmi sementara di bawah." } else { "" }
                    );
                }
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
        let _ = write!(o, "<details class=\"miss\"><summary>{} pemilihan lain belum ada hitungan resmi di sini</summary><p class=\"mute\">{}</p></details>", missing.len(), missing.join(", "));
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
        let _ = write!(o, "<details class=\"miss\"><summary>{} negara bagian belum ada hitungan resmi di sini</summary><p class=\"mute\">{}</p></details>", missing.len(), missing.join(", "));
    }

    // footer
    o.push_str("</aside></div><footer>");
    let _ = write!(
        o,
        "<p>Sumber: situs hasil resmi kantor pemilu negara bagian (tautan di tiap baris). Semua angka berstatus belum resmi dan bisa berubah sampai disertifikasi.</p><p>Waktu sumber = waktu yang dicap penerbit pada berkas data. Waktu ambil = saat halaman ini dibuat: {}.{}</p><p>{}</p><p>Jadwal kursi Senat: Ballotpedia. Jumlah kursi DPR: Sensus AS 2020. Situs ini tidak membuat proyeksi sendiri dan tidak menetapkan pemenang.</p>",
        wib(now),
        match newest {
            Some(t) => format!(" Data sumber terbaru berstempel {}.", wib(t)),
            None => " Belum ada sumber yang berhasil dibaca.".into(),
        },
        match proj {
            Some(p) => format!(
                "Entri pemenang di Wikipedia: ditulis sukarelawan (CC BY-SA 4.0), belum diverifikasi ke laporan media per pemilihan, disunting manual sehingga bisa terlambat atau keliru. Artikel Senat <a href=\"https://en.wikipedia.org/w/index.php?title={}&amp;oldid={}\" rel=\"noopener\">revisi {}</a>, artikel DPR <a href=\"https://en.wikipedia.org/w/index.php?title={}&amp;oldid={}\" rel=\"noopener\">revisi {}</a>. Artikel terakhir disunting {} ({}). Itu waktu penyuntingan di Wikipedia, bukan waktu data hasil, dan tiap hasil belum kami telusuri ke kutipan medianya. Jika proyeksi dan hitungan resmi berbeda, keduanya ditampilkan.",
                SENATE_TITLE, p.senate_rev, p.senate_rev, HOUSE_TITLE, p.house_rev, p.house_rev, wib(p.revision_time), age(now, p.revision_time)
            ),
            None if matches!(projr, ProjResult::Failed(e) if e == "off") => "Tidak ada penanda pemenang atau total kamar: situs ini hanya menampilkan hitungan dari sumber resmi negara bagian yang sudah tersambung.".to_string(),
            None => format!("Entri Wikipedia: sumber gagal dibaca saat halaman ini dibuat{}.", match projr { ProjResult::Failed(e) => format!(" ({})", esc(e)), _ => String::new() }),
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
        format!(
            "{}{}",
            if stale { " Sumber belum memperbarui data lebih dari 30 menit." } else { "" },
            match &f.refresh_note {
                Some(n) => format!(" Peringatan: {}.", esc(n)),
                None => String::new(),
            }
        )
    )
}

pub fn json(reference: &Reference, feeds: &[FeedResult], projr: &ProjResult, now: DateTime<Utc>) -> String {
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
        "projection": match projr {
            ProjResult::Ok(p) => serde_json::json!({
                "status": "ok", "origin": "Wikipedia (CC BY-SA 4.0), volunteer-compiled from media reports; not official",
                "revision_time": p.revision_time, "senate_rev": p.senate_rev, "house_rev": p.house_rev,
                "senate_projected_winner_party": p.senate.iter().map(|(k, v)| (k.clone(), v.label())).collect::<std::collections::BTreeMap<_, _>>(),
                "house_seats_called": p.house.iter().map(|(k, v)| (k.clone(), serde_json::json!({"dem": v.0, "rep": v.1}))).collect::<std::collections::BTreeMap<_, _>>(),
            }),
            ProjResult::Failed(e) => serde_json::json!({"status": "failed", "reason": e}),
        },
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
.st.band{stroke:#8A8A92;stroke-width:1.2}
.st.pd{fill:var(--blue)}.st.pr{fill:var(--red)}.st.po{fill:var(--purp)}.st.conflict{stroke:var(--ink);stroke-width:2;stroke-dasharray:4 2}
.sw.pd{background:var(--blue)}.sw.pr{background:var(--red)}.sw.po{background:var(--purp)}
.bar i.pd{background:var(--blue)}.bar i.pr{background:var(--red)}.bar i.po{background:var(--purp)}
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
