//! Layer 3 of the analytics engine: deterministic signals across games.
//!
//! Input is the player's recent games (metrics plus death context, loaded by
//! the caller) and comparison populations. Output is typed [`Signal`]s with
//! evidence, ranked so that recurring, high-impact patterns come first, plus
//! win-correlation statistics. Pure functions, no I/O.

pub mod correlation;
pub mod cross;
pub mod goals;
pub mod rules;
pub mod sessions;
mod stats;

use std::collections::{BTreeMap, HashMap};

use rc_metrics::zones::Zone;
use serde::Serialize;

pub use correlation::{WinLink, win_links};
pub use rules::{GameFlag, game_flags};

#[derive(Debug, Clone, PartialEq)]
pub struct Death {
    pub t_s: f64,
    pub x: i32,
    pub y: i32,
    pub zone: Zone,
    /// Enemy champions credited (killer + assists).
    pub enemies: i32,
    pub solo: bool,
    pub before_14: bool,
    pub enemy_side: bool,
    pub gold_vs_opponent: Option<i32>,
    pub unspent_gold: Option<i32>,
    pub shutdown_bounty: i32,
    pub enemy_objective: Option<String>,
    pub killer_champion: Option<String>,
}

/// One of the player's Summoner's Rift games.
#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub match_id: String,
    pub start_ms: i64,
    pub duration_s: i64,
    pub win: bool,
    /// `teamPosition` (`MIDDLE`, ...).
    pub role: Option<String>,
    pub champion: String,
    /// Played on the blue side (bottom-left base).
    pub blue_side: bool,
    pub metrics: BTreeMap<String, f64>,
    pub deaths: Vec<Death>,
}

impl Game {
    pub fn metric(&self, key: &str) -> Option<f64> {
        self.metrics.get(key).copied()
    }

    pub fn end_ms(&self) -> i64 {
        self.start_ms + self.duration_s * 1000
    }
}

/// Comparison data for the detectors.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// role → metric → values of other players in that role (your lobbies).
    pub role_populations: HashMap<String, HashMap<String, Vec<f64>>>,
    /// role → zone → deaths of other players in that role.
    pub role_death_zones: HashMap<String, BTreeMap<Zone, u32>>,
    /// Games closer than this belong to one session.
    pub session_gap_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Weakness,
    Strength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    AtMost,
    AtLeast,
}

impl Comparator {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AtMost => "at_most",
            Self::AtLeast => "at_least",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "at_most" => Some(Self::AtMost),
            "at_least" => Some(Self::AtLeast),
            _ => None,
        }
    }

    pub fn met(self, value: f64, target: f64) -> bool {
        const EPS: f64 = 1e-9;
        match self {
            Self::AtMost => value <= target + EPS,
            Self::AtLeast => value >= target - EPS,
        }
    }

    /// The natural comparator for a metric's direction.
    pub fn for_direction(higher_is_better: bool) -> Self {
        if higher_is_better { Self::AtLeast } else { Self::AtMost }
    }
}

/// A concrete spot in a game backing a signal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evidence {
    pub match_id: String,
    /// Game time in seconds, for events (deaths); `None` for whole-game facts.
    pub t_s: Option<f64>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub note: String,
}

/// A goal a signal suggests ("deaths before 14:00 at most 1").
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GoalSuggestion {
    pub metric: &'static str,
    pub comparator: Comparator,
    pub target: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    /// Stable id, e.g. `early_deaths`, `death_zone:enemy_jungle`, `weak:cs_10:MIDDLE`.
    pub id: String,
    /// Detector name.
    pub kind: &'static str,
    pub polarity: Polarity,
    pub severity: Severity,
    /// Ranking score (higher first). Recurring and win-relevant patterns score higher.
    pub score: f64,
    pub title: String,
    /// One or two factual sentences, every number taken from the data.
    pub detail: String,
    /// A generic coaching hint for this kind of pattern.
    pub tip: &'static str,
    pub games_considered: usize,
    pub games_affected: usize,
    /// Share of considered games affected (or the measured share, e.g. of deaths), 0–100.
    pub rate_pct: f64,
    pub metric: Option<&'static str>,
    /// Your win rate in affected games and in the others (when both have ≥ 3 games).
    pub win_rate_affected: Option<f64>,
    pub win_rate_other: Option<f64>,
    /// Newest first, capped.
    pub evidence: Vec<Evidence>,
    pub goal: Option<GoalSuggestion>,
}

/// Everything the detectors found in a window of games.
#[derive(Debug, Clone, Serialize)]
pub struct Analysis {
    pub games: usize,
    pub wins: usize,
    /// Highest score first.
    pub signals: Vec<Signal>,
    /// Strongest link first.
    pub win_links: Vec<WinLink>,
}

/// Evidence lists are capped at this many entries.
pub const MAX_EVIDENCE: usize = 20;

pub fn analyze(games: &[Game], ctx: &Context) -> Analysis {
    let mut signals = rules::detect(games);
    let covered: Vec<&str> = signals.iter().filter_map(|s| s.metric).collect();
    let more = cross::percentile_signals(games, ctx, &covered);
    signals.extend(more);
    signals.extend(cross::death_zones(games, ctx));
    signals.extend(cross::session_effects(games, ctx.session_gap_ms));
    signals.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    Analysis { games: games.len(), wins: games.iter().filter(|g| g.win).count(), signals, win_links: win_links(games) }
}

// ---------- shared helpers ----------

pub(crate) fn pct(part: usize, whole: usize) -> f64 {
    if whole == 0 { 0.0 } else { part as f64 / whole as f64 * 100.0 }
}

/// Win rates (0–100, rounded) in affected vs other games; each side needs ≥ 3 games.
pub(crate) fn win_rates<'a>(
    affected: impl IntoIterator<Item = &'a Game>,
    other: impl IntoIterator<Item = &'a Game>,
) -> (Option<f64>, Option<f64>) {
    let rate = |it: Vec<&Game>| (it.len() >= 3).then(|| pct(it.iter().filter(|g| g.win).count(), it.len()).round());
    (rate(affected.into_iter().collect()), rate(other.into_iter().collect()))
}

/// Positive when affected games go worse (weakness) or better (strength).
pub(crate) fn win_gap(polarity: Polarity, wr: (Option<f64>, Option<f64>)) -> Option<f64> {
    let (a, o) = (wr.0?, wr.1?);
    Some(match polarity {
        Polarity::Weakness => o - a,
        Polarity::Strength => a - o,
    })
}

pub(crate) fn severity(rate: f64, gap: Option<f64>) -> Severity {
    let base = if rate >= 60.0 {
        2
    } else if rate >= 40.0 {
        1
    } else {
        0
    };
    match base + i32::from(gap.is_some_and(|g| g >= 20.0)) {
        0 => Severity::Low,
        1 => Severity::Medium,
        _ => Severity::High,
    }
}

/// `strength` in 0..1 (how pronounced), scaled by how many games show it and
/// how much it costs (or wins) games.
pub(crate) fn score(strength: f64, affected: usize, gap: Option<f64>) -> f64 {
    strength * (affected as f64).sqrt() * (1.0 + gap.unwrap_or(0.0).max(0.0) / 50.0)
}

pub(crate) fn win_sentence(wr: (Option<f64>, Option<f64>)) -> String {
    match wr {
        (Some(a), Some(o)) => format!(" You won {a}% of those games and {o}% of the rest."),
        _ => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn game(i: usize, win: bool, metrics: &[(&str, f64)]) -> Game {
        Game {
            match_id: format!("NA1_{i}"),
            start_ms: i as i64 * 3_600_000 * 24,
            duration_s: 1800,
            win,
            role: Some("MIDDLE".into()),
            champion: "Leblanc".into(),
            blue_side: true,
            metrics: metrics.iter().map(|(k, v)| ((*k).to_string(), *v)).collect(),
            deaths: vec![],
        }
    }

    pub fn death(t_s: f64, zone: Zone) -> Death {
        Death {
            t_s,
            x: 7000,
            y: 7000,
            zone,
            enemies: 2,
            solo: false,
            before_14: t_s < 840.0,
            enemy_side: false,
            gold_vs_opponent: None,
            unspent_gold: None,
            shutdown_bounty: 0,
            enemy_objective: None,
            killer_champion: Some("Ahri".into()),
        }
    }
}
