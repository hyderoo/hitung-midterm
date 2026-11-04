//! Checked-in reference data. Nothing here is election results: it is the
//! contest schedule, House seat totals, source registry and map geometry.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize, Clone, Debug)]
pub struct SourceCfg {
    pub state: String,
    pub label: String,
    pub human_url: String,
    pub export_url: String,
    pub senate_exact: Vec<String>,
    pub house_regex: String,
    /// "" = Enhanced Voting Media Export; "ncsbe" = NC State Board statewide results file.
    #[serde(default)]
    pub format: String,
    /// When set, the origin is not fetched at all (reason shown on the page).
    #[serde(default)]
    pub suspended: Option<String>,
}
#[derive(Deserialize)]
struct Sources {
    sources: Vec<SourceCfg>,
}
#[derive(Deserialize, Clone, Debug)]
pub struct SenateEntry {
    pub state: String,
    pub kind: String,
}
#[derive(Deserialize)]
struct Schedule {
    contests: Vec<SenateEntry>,
}
#[derive(Deserialize)]
struct Seats {
    seats: BTreeMap<String, u32>,
}
#[derive(Deserialize, Clone, Debug)]
pub struct MapState {
    pub name: String,
    pub d: String,
    pub cx: f64,
    pub cy: f64,
}
#[derive(Deserialize)]
pub struct MapData {
    pub width: f64,
    pub height: f64,
    pub states: BTreeMap<String, MapState>,
}

pub struct Reference {
    pub sources: Vec<SourceCfg>,
    pub senate: Vec<SenateEntry>,
    pub house_seats: BTreeMap<String, u32>,
    pub map: MapData,
}

impl Reference {
    pub fn load() -> Reference {
        let s: Sources = serde_json::from_str(include_str!("../data/sources.json")).expect("sources.json");
        let sch: Schedule = serde_json::from_str(include_str!("../data/senate_schedule.json")).expect("senate_schedule.json");
        let seats: Seats = serde_json::from_str(include_str!("../data/house_seats.json")).expect("house_seats.json");
        let map: MapData = serde_json::from_str(include_str!("../data/map.json")).expect("map.json");
        Reference { sources: s.sources, senate: sch.contests, house_seats: seats.seats, map }
    }
    pub fn house_total(&self) -> u32 {
        self.house_seats.values().sum()
    }
    pub fn in_senate_schedule(&self, st: &str) -> bool {
        self.senate.iter().any(|e| e.state == st)
    }
}
