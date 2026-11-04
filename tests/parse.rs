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
    assert!(html.contains("Gagal diperbarui"));
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
    let cfg = SourceCfg { state: "NC".into(), label: "x".into(), human_url: "h".into(), export_url: "e".into(), senate_exact: vec!["US SENATE (VOTE FOR 1)".into()], house_regex: r"^US HOUSE OF REPRESENTATIVES DISTRICT (\d+)\s*\(VOTE FOR 1\)$".into(), format: "ncsbe".into() };
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
    let cfg = SourceCfg { state: "TX".into(), label: "x".into(), human_url: "h".into(), export_url: "e".into(), senate_exact: vec!["U. S. SENATOR".into()], house_regex: r"^U\. S\. REPRESENTATIVE DISTRICT (\d+)$".into(), format: "txsos".into() };
    let tr = |p: &str, c: &str, v: &str| format!(r#"<tr><td class="party">{p}</td><td class="candidate">{c}</td><td class="ballot">{v}</td></tr>"#);
    let html = format!(r#"<div class="election">U. S. SENATOR </div><table>{}{}{}</table><div class="election">U. S. REPRESENTATIVE DISTRICT 1</div><table>{}</table><div class="election">STATE SENATOR DISTRICT 1</div><table>{}</table>"#,
        tr("REPUBLICAN", "A", "1,000"), tr("DEMOCRATIC", "B", "2,000"), tr("WRITE-IN", "W", "5"),
        tr("REPUBLICAN", "C (I)", "7"), tr("DEMOCRATIC", "Z", "99"));
    let f = parse_txsos(&html, &cfg, chrono::Utc::now()).unwrap();
    let s = f.senate.unwrap();
    assert_eq!(s.candidates[0].party, Party::Dem);
    assert_eq!(s.candidates[0].votes, 2000);
    assert_eq!(s.write_in, 5);
    assert_eq!(f.house.len(), 1);
    assert!(parse_txsos("<html>404</html>", &cfg, chrono::Utc::now()).is_err());
}
