use hitung_midterm::{config::*, feed::*, model::*, render::*};

fn cfg() -> SourceCfg {
    SourceCfg {
        state: "TS".into(),
        label: "Test".into(),
        human_url: "https://example.test".into(),
        export_url: "https://example.test/x.json".into(),
        senate_exact: vec!["Test Senate".into()],
        house_regex: r"^Test House \((\d+)(?:st|nd|rd|th) District\)$".into(),
        format: String::new(),
        suspended: None,
    }
}

#[test]
fn parses_fixture() {
    let f = parse_export(include_bytes!("fixtures/test_only_export.json"), &cfg()).unwrap();
    let s = f.senate.unwrap();
    assert_eq!(s.candidates[0].name, "Ann Test");
    assert_eq!(s.candidates[0].party, Party::Dem);
    assert_eq!(s.write_in, 3);
    assert_eq!(s.lead(), Lead::Ahead(Party::Dem));
    assert_eq!(s.margin(), Some(40));
    assert_eq!(s.precincts, Some((3, 4)));
    // "House of Delegates" must not match the House pattern
    assert_eq!(f.house.len(), 2);
    assert_eq!(f.house[0].lead(), Lead::Tie);
    assert_eq!(f.house[1].lead(), Lead::NoVotes);
}

#[test]
fn bad_input_is_an_error_not_zero() {
    assert!(parse_export(b"{}", &cfg()).is_err());
    assert!(parse_export(b"<html>", &cfg()).is_err());
}

#[test]
fn clean_name_strips_tags() {
    assert_eq!(clean_name("Jon Ossoff (I) (Dem)"), "Jon Ossoff");
    assert_eq!(clean_name("Tremaine \"Teddy\" Reese (Dem)"), "Tremaine \"Teddy\" Reese");
}

#[test]
fn reference_data_is_consistent() {
    let r = Reference::load();
    assert_eq!(r.house_total(), 435);
    assert_eq!(r.senate.len(), 35);
    let mut seen = std::collections::BTreeSet::new();
    for e in &r.senate {
        assert!(seen.insert(e.state.clone()), "state with two senate contests: {}", e.state);
        assert!(r.map.states.contains_key(&e.state));
    }
    for s in &r.sources {
        assert!(r.map.states.contains_key(&s.state));
        regex::Regex::new(&s.house_regex).unwrap();
    }
    assert_eq!(r.map.states.len(), 50);
}

#[test]
fn page_without_data_never_shows_zero_totals_or_control() {
    let r = Reference::load();
    let failed = vec![FeedResult::Failed {
        state: "VA".into(),
        source_label: "Test".into(),
        human_url: "https://example.test".into(),
        reason: "boom".into(),
    }];
    let html = page(&r, &failed, &hitung_midterm::projection::ProjResult::Failed("x".into()), View::Senate, chrono::Utc::now());
    assert!(html.contains("0 dari 35 pemilihan kursi Senat sudah ada hitungan resmi"));
    assert!(html.contains("Tidak ditampilkan"));
    assert!(html.contains("Sumber tidak tersedia"));
    assert!(!html.contains("dipanggil") && !html.contains("perlombaan"));
}

mod proj {
    use hitung_midterm::{model::Party, projection::*};
    use std::collections::BTreeMap;

    fn names() -> BTreeMap<String, String> {
        (0..50).map(|i| (format!("S{i}"), format!("C{i}"))).collect()
    }
    #[test]
    fn senate_rows_and_calls() {
        let mut wt = String::from("x\n== Race summary ==\n{|\n");
        for i in 0..33 {
            let win = if i == 0 {
                ""
            } else if i % 2 == 0 {
                "{{Party stripe|Democratic Party (US)}}{{aye}} A"
            } else {
                "{{Party stripe|Republican Party (US)}}{{aye}} B"
            };
            wt += &format!("|-\n! [[2026 United States Senate election in S{i}|S{i}]]\n| {win}\n");
        }
        wt += "\n== Alabama ==\n";
        let m = parse_senate(&wt, &names()).unwrap();
        assert!(!m.contains_key("C0"), "an uncalled race must stay absent");
        assert_eq!(m["C1"], Party::Rep);
        assert_eq!(m["C2"], Party::Dem);
        assert_eq!(m.len(), 32);
    }
    #[test]
    fn senate_format_change_is_an_error() {
        assert!(parse_senate("== Race summary ==\n== Alabama ==", &names()).is_err());
    }
    #[test]
    fn house_rows_and_empty_states() {
        let mut wt = String::from("Per state ===\n{|\n");
        for i in 0..50 {
            if i == 3 {
                wt += &format!("|-\n![[#S{i}|S{i}]]\n|7\n| \n|\n|\n|\n");
            } else {
                wt += &format!("|-\n![[#S{i}|S{i}]]\n|7\n|{{{{party shading/Democratic}}}}|'''4'''\n|{{{{steady}}}}\n|3\n|{{{{steady}}}}\n");
            }
        }
        wt += "\n|}";
        let r = parse_house(&wt, &names()).unwrap();
        assert_eq!(r["C0"], (4, 3));
        assert!(!r.contains_key("C3"), "a state with empty cells has no data");
        assert!(parse_house("Per state ===\n|}", &names()).is_err());
    }
    #[test]
    fn totals() {
        let t = parse_totals("| seats_after1 = '''52'''{{efn|x}}\n| seats_after2 = 43\n");
        assert_eq!((t.dem, t.rep), (Some(52), Some(43)));
        assert_eq!(parse_totals("| seats_after1 = \n").dem, None);
    }
}

#[test]
fn ncsbe_statewide_rows() {
    use hitung_midterm::{config::SourceCfg, feed::parse_ncsbe, model::Party};
    let cfg = SourceCfg { state: "NC".into(), label: "x".into(), human_url: "h".into(), export_url: "e".into(), senate_exact: vec!["US SENATE (VOTE FOR 1)".into()], house_regex: r"^US HOUSE OF REPRESENTATIVES DISTRICT (\d+)\s*\(VOTE FOR 1\)$".into(), format: "ncsbe".into(), suspended: None };
    let row = |c: &str, b: &str, p: &str, v: &str| format!(r#"{{"cnm":"{c}","bnm":"{b}","pty":"{p}","vct":"{v}","prt":"5","ptl":"6"}}"#);
    let j = format!("[{},{},{},{},{}]",
        row("US SENATE (VOTE FOR 1)", "B Rep", "REP", "10"), row("US SENATE (VOTE FOR 1)", "A Dem", "DEM", "30"),
        row("US HOUSE OF REPRESENTATIVES DISTRICT 02  (VOTE FOR 1)", "C", "DEM", "7"),
        row("US HOUSE OF REPRESENTATIVES DISTRICT 02  (VOTE FOR 1)", "Write-In (Miscellaneous)", "", "2"),
        row("NC STATE SENATE DISTRICT 14 (VOTE FOR 1)", "Z", "DEM", "99"));
    let f = parse_ncsbe(j.as_bytes(), &cfg, chrono::Utc::now()).unwrap();
    assert_eq!(f.senate.as_ref().unwrap().candidates[0].party, Party::Dem);
    assert_eq!(f.senate.as_ref().unwrap().precincts, Some((5, 6)));
    assert_eq!(f.house.len(), 1);
    assert_eq!(f.house[0].district, Some(2));
    assert_eq!(f.house[0].write_in, 2);
}

#[test]
fn txsos_html_rows() {
    use hitung_midterm::{config::SourceCfg, feed::parse_txsos, model::Party};
    let cfg = SourceCfg { state: "TX".into(), label: "x".into(), human_url: "h".into(), export_url: "e".into(), senate_exact: vec!["U. S. SENATOR".into()], house_regex: r"^U\. S\. REPRESENTATIVE DISTRICT (\d+)$".into(), format: "txsos".into(), suspended: None };
    let tr = |p: &str, c: &str, v: &str| format!(r#"<tr><td class="party">{p}</td><td class="candidate">{c}</td><td class="ballot">{v}</td></tr>"#);
    let html = format!(r#"<div class="election">U. S. SENATOR </div><div class="polling">Polling Locations Reporting: 47%</div><table>{}{}{}</table><div class="election">U. S. REPRESENTATIVE DISTRICT 1</div><table>{}</table><div class="election">STATE SENATOR DISTRICT 1</div><table>{}</table>"#,
        tr("REPUBLICAN", "A", "1,000"), tr("DEMOCRATIC", "B", "2,000"), tr("WRITE-IN", "W", "5"),
        tr("REPUBLICAN", "C (I)", "7"), tr("DEMOCRATIC", "Z", "99"));
    let f = parse_txsos(&html, &cfg, chrono::Utc::now()).unwrap();
    let s = f.senate.unwrap();
    assert_eq!(s.candidates[0].party, Party::Dem);
    assert_eq!(s.candidates[0].votes, 2000);
    assert_eq!(s.write_in, 5);
    assert_eq!(s.polling_pct, Some(47));
    assert_eq!(f.house.len(), 1);
    assert!(parse_txsos("<html>404</html>", &cfg, chrono::Utc::now()).is_err());
}

#[test]
fn txsos_zero_votes_partial_and_ties() {
    use hitung_midterm::{config::SourceCfg, feed::parse_txsos, model::Lead, render::{age, is_partial, Tally}};
    let cfg = SourceCfg { state: "TX".into(), label: "x".into(), human_url: "h".into(), export_url: "e".into(), senate_exact: vec!["U. S. SENATOR".into()], house_regex: r"^U\. S\. REPRESENTATIVE DISTRICT (\d+)$".into(), format: "txsos".into(), suspended: None };
    let tr = |p: &str, c: &str, v: &str| format!(r#"<tr><td class="party">{p}</td><td class="candidate">{c}</td><td class="ballot">{v}</td></tr>"#);
    let blk = |n: &str, pct: u32, rows: String| format!(r#"<div class="election">{n}</div><div class="polling">Polling Locations Reporting: {pct}%</div><table>{rows}</table>"#);
    let html = [
        blk("U. S. REPRESENTATIVE DISTRICT 1", 100, tr("REPUBLICAN", "A", "10") + &tr("DEMOCRATIC", "B", "5")),
        blk("U. S. REPRESENTATIVE DISTRICT 2", 0, tr("REPUBLICAN", "C", "0") + &tr("DEMOCRATIC", "D", "0")),
        blk("U. S. REPRESENTATIVE DISTRICT 3", 56, tr("REPUBLICAN", "E", "7") + &tr("DEMOCRATIC", "F", "7")),
        blk("U. S. REPRESENTATIVE DISTRICT 4", 99, tr("REPUBLICAN", "G", "3") + &tr("DEMOCRATIC", "H", "9")),
    ].concat();
    let now = chrono::Utc::now();
    let f = parse_txsos(&html, &cfg, now).unwrap();
    assert_eq!(f.house[1].lead(), Lead::NoVotes);
    assert_eq!(f.house[2].lead(), Lead::Tie);
    assert_eq!(f.house[1].polling_pct, Some(0));
    let flags: Vec<bool> = f.house.iter().map(is_partial).collect();
    assert_eq!(flags, vec![false, true, true, true], "99% is partial; only 100% is not");
    let mut t = Tally::default();
    for c in &f.house { t.add(c.lead()); }
    assert_eq!(t.with_data(), 3, "zero-vote contest is not counted as having data");
    assert_eq!((t.dem, t.rep, t.tie, t.novotes), (1, 1, 1, 1));
    let old = now - chrono::Duration::minutes(178);
    assert!(age(now, old).contains("2 jam 58 menit"), "{}", age(now, old));
}

#[test]
fn orsos_html_rows_and_time() {
    use hitung_midterm::{config::SourceCfg, feed::parse_orsos, model::Party, render::is_partial};
    let cfg = SourceCfg { state: "OR".into(), label: "x".into(), human_url: "h".into(), export_url: "e".into(), senate_exact: vec!["US Senator".into()], house_regex: r"^US Representative, (\d+)(?:st|nd|rd|th) District$".into(), format: "orsos".into(), suspended: None };
    let cand = |n: &str, p: &str, v: &str| format!(r#"<div class="col display-results-box-d"><h1>{n}</h1><h2 class="X">{p}</h2></div><div class="col display-results-box-f"> <div>{v}</div></div>"#);
    let html = format!(r#"<div class="next-count">Results last updated: 11/4/2026 | 12:45 AM PT</div><h1>US Senator</h1>{}{}{}<h1>US Representative, 2nd District</h1>{}<h1>State Senator</h1>{}"#,
        cand("A", "Republican", "600,760"), cand("B", "Democrat, Working Families", "845,830"), cand("Write-in", "", "1,106"),
        cand("C", "Republican", "5"), cand("Z", "Democrat", "9"));
    let f = parse_orsos(&html, &cfg).unwrap();
    let s = f.senate.unwrap();
    assert_eq!(s.candidates[0].party, Party::Dem);
    assert_eq!(s.candidates[0].votes, 845830);
    assert_eq!(s.write_in, 1106);
    assert_eq!(f.house.len(), 1);
    assert_eq!(f.house[0].district, Some(2));
    assert_eq!(f.source_time.to_rfc3339(), "2026-11-04T08:45:00+00:00");
    assert!(!is_partial(&s), "unavailable reporting status is not asserted as partial");
    assert!(parse_orsos("<html></html>", &cfg).is_err());
}

#[test]
fn mdsbe_html_rows_and_time() {
    use hitung_midterm::{config::SourceCfg, feed::parse_mdsbe, model::Party};
    let cfg = SourceCfg { state: "MD".into(), label: "x".into(), human_url: "h".into(), export_url: "e".into(), senate_exact: vec![], house_regex: "".into(), format: "mdsbe".into(), suspended: None };
    let tr = |n: &str, p: &str, t: &str| format!("<tr>\n\t<td><div>{n}</div></td>\n\t<td>{p}</td>\n\t<td class=\"alignRight\"> 1</td><td class=\"alignRight\"> NR</td>\n\t<td class=\"alignRight\">{t}</td>\n\t<td class=\"alignRight\">44.08%</td>\n</tr>");
    let html = format!("<div>Last refreshed: 11/04/2026 03:44:31 AM</div><h3>District 1</h3>{}{}{}{}{}<h3>District 2</h3>{}", tr("A","Democratic","119,088"), tr("B","Republican","150,615"), tr("C (Write-In)","Unaffiliated","3"), tr("Other Write-Ins","","477"), tr("<b>Totals</b>","","270,180"), tr("D","Republican","5"));
    let f = parse_mdsbe(&html, &cfg).unwrap();
    assert_eq!(f.house.len(), 2);
    assert_eq!(f.house[0].candidates[0].party, Party::Rep);
    assert_eq!(f.house[0].candidates.len(), 2);
    assert_eq!(f.house[0].write_in, 480);
    assert_eq!(f.source_time.to_rfc3339(), "2026-11-04T08:44:31+00:00");
    assert!(parse_mdsbe("<html></html>", &cfg).is_err());
}

#[test]
fn party_classify_prefers_wording() {
    use hitung_midterm::model::Party;
    assert_eq!(Party::classify(Some("(Prefers Democratic Party)")), Party::Dem);
    assert_eq!(Party::classify(Some("(Prefers Republican Party)")), Party::Rep);
    assert_eq!(Party::classify(Some("Democratic")), Party::Dem);
    assert_eq!(Party::classify(Some("(Prefers Independent Party)")), Party::Other);
    assert_eq!(Party::classify(None), Party::Other);
}

#[tokio::test]
async fn suspended_source_is_not_fetched() {
    use hitung_midterm::{feed::fetch_state, model::FeedResult};
    let mut c = cfg();
    c.export_url = "http://127.0.0.1:9/never".into();
    c.suspended = Some("alasan uji".into());
    let client = reqwest::Client::new();
    match fetch_state(&client, &c).await {
        FeedResult::Failed { reason, .. } => assert!(reason.contains("dihentikan") && reason.contains("alasan uji")),
        _ => panic!("harus gagal tanpa mengambil data"),
    }
}

#[test]
fn orsos_nm_layout_mt_time_and_precincts() {
    use hitung_midterm::{config::SourceCfg, feed::parse_orsos, model::Party};
    let cfg = SourceCfg { state: "NM".into(), label: "x".into(), human_url: "h".into(), export_url: "e".into(), senate_exact: vec!["United States Senator".into()], house_regex: r"^United States Representative DISTRICT (\d+)$".into(), format: "orsos".into(), suspended: None };
    let cand = |n: &str, p: &str, v: &str| format!(r#"<div class="col display-results-box-d"><h1>{n}</h1><h2 class="X">{p}</h2></div><div class="col display-results-box-f"><h1>{v}</h1></div>"#);
    let html = format!(r#"Results last updated:</span><span>11/4/2026 2:21 AM MT</span> <h1>United States Senator </h1><div class="precinct-fully">Precincts Fully: 2204 / 2204</div>{}{}<h1>United States Representative DISTRICT 2</h1><div>Precincts Fully: 10 / 20</div>{}{}"#,
        cand("BEN R LUJAN","Democrat","455,165"), cand("LARRY E MARKER","Republican","265,557"), cand("A","Democrat","86,650"), cand("B","Republican","123,113"));
    let f = parse_orsos(&html, &cfg).unwrap();
    assert_eq!(f.source_time.to_rfc3339(), "2026-11-04T09:21:00+00:00");
    let s = f.senate.unwrap();
    assert_eq!(s.candidates[0].party, Party::Dem);
    assert_eq!(s.precincts, Some((2204, 2204)));
    assert_eq!(f.house[0].precincts, Some((10, 20)));
    assert_eq!(f.house[0].candidates[0].votes, 123113);
}
