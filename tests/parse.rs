use hitung_midterm::{config::*, feed::*, model::*, render::*};

fn cfg() -> SourceCfg {
    SourceCfg {
        state: "TS".into(),
        label: "Test".into(),
        human_url: "https://example.test".into(),
        export_url: "https://example.test/x.json".into(),
        senate_exact: vec!["Test Senate".into()],
        house_regex: r"^Test House \((\d+)(?:st|nd|rd|th) District\)$".into(),
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
    let html = page(&r, &failed, View::Senate, chrono::Utc::now());
    assert!(html.contains("0 dari 35 pemilihan kursi Senat sudah ada data"));
    assert!(html.contains("Tidak tersedia"));
    assert!(html.contains("Gagal diperbarui"));
    assert!(!html.contains("dipanggil") && !html.contains("perlombaan"));
}
