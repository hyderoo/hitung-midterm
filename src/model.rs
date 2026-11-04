//! Plain data types shared by the parser and the renderer.
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Party {
    Dem,
    Rep,
    Other,
}

impl Party {
    pub fn classify(raw: Option<&str>) -> Party {
        match raw.map(|s| { let l = s.trim().trim_start_matches('(').trim().to_ascii_lowercase(); l.strip_prefix("prefers ").unwrap_or(&l).to_string() }) {
            Some(s) if s.starts_with("dem") => Party::Dem,
            Some(s) if s.starts_with("rep") => Party::Rep,
            _ => Party::Other,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Party::Dem => "Demokrat",
            Party::Rep => "Republik",
            Party::Other => "Lainnya",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub name: String,
    pub party: Party,
    pub votes: u64,
}

/// Who is ahead on the votes counted so far. This is a vote lead only:
/// the feeds carry no race calls, so nothing here is a projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lead {
    NoVotes,
    Tie,
    /// Votes exist, but unattributed votes are large enough that they could
    /// change the ranking, so no leader is declared.
    Unresolved,
    Ahead(Party),
}

#[derive(Clone, Debug, Serialize)]
pub struct Contest {
    pub state: String,
    pub label: String,
    pub district: Option<u32>,
    /// Named candidates, highest vote count first. Write-ins are kept apart.
    pub candidates: Vec<Candidate>,
    pub write_in: u64,
    /// Votes a source lists under names that are neither party-labelled
    /// candidates nor source-labelled write-ins (New Jersey counties
    /// enumerate individual write-in names). Kept apart from write_in: their
    /// attribution is unknown, not "write-in".
    #[serde(default)]
    pub unresolved: u64,
    /// Precincts reporting / participating, summed from the locality rows of
    /// the same feed. None when the feed does not provide them.
    pub precincts: Option<(u32, u32)>,
    /// Share of polling locations reporting, in percent, for sources that
    /// publish only a percentage (Texas).
    pub polling_pct: Option<u8>,
}

impl Contest {
    pub fn lead(&self) -> Lead {
        let top = match self.candidates.first() {
            Some(c) if c.votes > 0 => c,
            _ => return Lead::NoVotes,
        };
        if self.candidates.get(1).map(|c| c.votes) == Some(top.votes) {
            return Lead::Tie;
        }
        if self.unresolved > 0 && self.margin().map(|m| self.unresolved >= m).unwrap_or(true) {
            return Lead::Unresolved;
        }
        Lead::Ahead(top.party)
    }
    pub fn margin(&self) -> Option<u64> {
        match (self.candidates.first(), self.candidates.get(1)) {
            (Some(a), Some(b)) => Some(a.votes - b.votes),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StateFeed {
    pub state: String,
    pub source_label: String,
    pub human_url: String,
    pub export_url: String,
    /// Timestamp the publisher stamped on the file (createdAt).
    pub source_time: DateTime<Utc>,
    pub senate: Option<Contest>,
    pub house: Vec<Contest>,
    /// Set when this copy was served from the in-memory cache because the
    /// latest refresh failed.
    pub refresh_note: Option<String>,
}

#[derive(Clone, Debug)]
pub enum FeedResult {
    Ok(StateFeed),
    Failed {
        state: String,
        source_label: String,
        human_url: String,
        reason: String,
    },
}

impl FeedResult {
    pub fn state(&self) -> &str {
        match self {
            FeedResult::Ok(f) => &f.state,
            FeedResult::Failed { state, .. } => state,
        }
    }
}
