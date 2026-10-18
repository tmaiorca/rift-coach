//! Play sessions: games grouped by the gap between them, with session
//! averages against your usual numbers and the patterns that showed up.

use std::collections::BTreeMap;

use rc_common::queues::{is_standard_rift, queue_name};
use rc_metrics::defs::{self, Unit};
use rc_signals::rules::RULES;
use rc_signals::{Game, GameFlag, Polarity};
use rc_store::matches::{self, MatchSummary};
use rc_store::metrics::GameFilter;
use rc_store::reports::{self, Report};
use rc_store::rusqlite::Connection;
use serde::Serialize;

use crate::CoachError;
use crate::patterns::load_games;

pub const SESSION: &str = "session";
/// Sessions are built from this many of the newest stored games.
const HISTORY: u32 = 1000;

/// Metrics summarised per session (in this order).
pub const SESSION_METRICS: [&str; 12] = [
    "cs_10",
    "cs_diff_10",
    "gold_diff_15",
    "deaths_pre14",
    "deaths",
    "solo_deaths",
    "deaths_before_obj",
    "kp",
    "dmg_share",
    "obj_part",
    "vision_pm",
    "cspm",
];

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    /// `session:<first match id>:<games>`; also the AI report scope.
    pub scope: String,
    pub first_match_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub games: usize,
    /// Summoner's Rift games (the ones with metrics), remakes excluded.
    pub rift_games: usize,
    pub wins: usize,
    pub losses: usize,
    /// Champion ids in play order.
    pub champion_ids: Vec<i32>,
    pub has_report: bool,
}

pub fn scope_of(first_match_id: &str, games: usize) -> String {
    format!("{SESSION}:{first_match_id}:{games}")
}

/// All sessions, newest first, with their games oldest first.
fn sessions_with_games(
    conn: &Connection,
    account_id: i64,
    gap_min: u32,
) -> rc_store::Result<Vec<(SessionSummary, Vec<MatchSummary>)>> {
    let mut all = matches::list_account_matches(conn, account_id, HISTORY, 0)?;
    all.reverse();
    let spans: Vec<(i64, i64)> = all.iter().map(|m| (m.game_start_ms, m.game_start_ms + m.duration_s * 1000)).collect();
    let mut out = Vec::new();
    for range in rc_signals::sessions::group(&spans, i64::from(gap_min) * 60_000) {
        let games: Vec<MatchSummary> = all[range].to_vec();
        let Some(first) = games.first() else { continue };
        let counted: Vec<&MatchSummary> = games.iter().filter(|m| !m.is_remake).collect();
        let scope = scope_of(&first.match_id, games.len());
        let summary = SessionSummary {
            has_report: reports::latest_for_scope(conn, SESSION, account_id, &scope)?.is_some(),
            scope,
            first_match_id: first.match_id.clone(),
            start_ms: first.game_start_ms,
            end_ms: games.last().map_or(first.game_start_ms, |m| m.game_start_ms + m.duration_s * 1000),
            games: games.len(),
            rift_games: counted.iter().filter(|m| is_standard_rift(m.queue_id, m.map_id)).count(),
            wins: counted.iter().filter(|m| m.win).count(),
            losses: counted.iter().filter(|m| !m.win).count(),
            champion_ids: games.iter().map(|m| m.champion_id).collect(),
        };
        out.push((summary, games));
    }
    out.reverse();
    Ok(out)
}

pub fn list(conn: &Connection, account_id: i64, gap_min: u32, limit: usize) -> rc_store::Result<Vec<SessionSummary>> {
    Ok(sessions_with_games(conn, account_id, gap_min)?.into_iter().take(limit).map(|(s, _)| s).collect())
}

/// The newest session, if it has ended (no game for `gap_min` minutes).
pub fn latest_ended(
    conn: &Connection,
    account_id: i64,
    gap_min: u32,
    now_ms: i64,
) -> rc_store::Result<Option<SessionSummary>> {
    let latest = sessions_with_games(conn, account_id, gap_min)?.into_iter().next().map(|(s, _)| s);
    Ok(latest.filter(|s| now_ms - s.end_ms > i64::from(gap_min) * 60_000))
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionGame {
    pub summary: MatchSummary,
    pub queue_name: &'static str,
    pub is_standard_rift: bool,
    pub metrics: BTreeMap<String, f64>,
    pub flags: Vec<GameFlag>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Aggregate {
    pub metric: &'static str,
    pub label: &'static str,
    pub unit: Unit,
    pub decimals: u8,
    pub higher_is_better: Option<bool>,
    pub session: f64,
    /// Your average over the games before this session.
    pub baseline: Option<f64>,
    /// Session better than baseline (None if no direction or no baseline).
    pub better: Option<bool>,
    pub games: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionPattern {
    pub kind: &'static str,
    pub title: &'static str,
    pub what: &'static str,
    pub polarity: Polarity,
    pub session_games_affected: usize,
    pub session_games: usize,
    /// How often it happens in your games before the session, 0–100.
    pub baseline_rate_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail {
    pub summary: SessionSummary,
    /// Play order.
    pub games: Vec<SessionGame>,
    pub aggregates: Vec<Aggregate>,
    pub patterns: Vec<SessionPattern>,
    pub baseline_games: usize,
    pub report: Option<Report>,
}

fn average(values: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = values.collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

pub fn detail(
    conn: &Connection,
    account_id: i64,
    scope: &str,
    gap_min: u32,
    baseline_games: u32,
) -> Result<SessionDetail, CoachError> {
    let (summary, matches) =
        sessions_with_games(conn, account_id, gap_min)?.into_iter().find(|(s, _)| s.scope == scope).ok_or_else(
            || CoachError::NotFound(format!("session {scope} (it may have changed; reopen it from the list)")),
        )?;
    let mut rift: Vec<Game> = load_games(
        conn,
        account_id,
        &GameFilter {
            limit: 1000,
            since_ms: Some(summary.start_ms),
            before_ms: Some(summary.end_ms),
            ..Default::default()
        },
    )?;
    rift.reverse();
    let baseline: Vec<Game> = load_games(
        conn,
        account_id,
        &GameFilter { limit: baseline_games, before_ms: Some(summary.start_ms), ..Default::default() },
    )?;

    let games = matches
        .into_iter()
        .map(|m| {
            let g = rift.iter().find(|g| g.match_id == m.match_id);
            SessionGame {
                queue_name: queue_name(m.queue_id),
                is_standard_rift: is_standard_rift(m.queue_id, m.map_id) && !m.is_remake,
                metrics: g.map(|g| g.metrics.clone()).unwrap_or_default(),
                flags: g.map(rc_signals::game_flags).unwrap_or_default(),
                summary: m,
            }
        })
        .collect();

    let (aggregates, patterns) = summarize(&rift, &baseline);

    Ok(SessionDetail {
        report: reports::latest_for_scope(conn, SESSION, account_id, scope)?,
        baseline_games: baseline.len(),
        summary,
        games,
        aggregates,
        patterns,
    })
}

/// Averages of `period` games against `baseline` games, and the per-game
/// rules that fired in the period (with their usual rate). Shared by
/// sessions and weeks.
pub(crate) fn summarize(rift: &[Game], baseline: &[Game]) -> (Vec<Aggregate>, Vec<SessionPattern>) {
    let aggregates: Vec<Aggregate> = SESSION_METRICS
        .iter()
        .filter_map(|key| {
            let d = defs::get(key)?;
            let in_session: Vec<f64> = rift.iter().filter_map(|g| g.metric(key)).collect();
            let session = average(in_session.iter().copied())?;
            let base = average(baseline.iter().filter_map(|g| g.metric(key)));
            let better = d.higher_is_better.zip(base).and_then(|(higher, b)| {
                ((session - b).abs() > 1e-9).then_some(if higher { session > b } else { session < b })
            });
            Some(Aggregate {
                metric: d.key,
                label: d.label,
                unit: d.unit,
                decimals: d.decimals,
                higher_is_better: d.higher_is_better,
                session,
                baseline: base,
                better,
                games: in_session.len(),
            })
        })
        .collect();

    let patterns: Vec<SessionPattern> = RULES
        .iter()
        .filter_map(|r| {
            let considered: Vec<&Game> = rift.iter().filter(|g| (r.considered)(g)).collect();
            let hits = considered.iter().filter(|g| (r.hit)(g).is_some()).count();
            if hits == 0 {
                return None;
            }
            let base: Vec<&Game> = baseline.iter().filter(|g| (r.considered)(g)).collect();
            let base_hits = base.iter().filter(|g| (r.hit)(g).is_some()).count();
            Some(SessionPattern {
                kind: r.kind,
                title: r.title,
                what: r.what,
                polarity: r.polarity,
                session_games_affected: hits,
                session_games: considered.len(),
                baseline_rate_pct: (base.len() >= 5).then(|| (base_hits as f64 / base.len() as f64 * 100.0).round()),
            })
        })
        .collect();

    (aggregates, patterns)
}
