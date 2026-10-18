//! Weeks (Monday to Sunday, local time): the week's games, its averages
//! against the four weeks before, champions played, patterns, benchmark gaps,
//! and the weekly trend of key numbers.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use rc_signals::Game;
use rc_store::matches;
use rc_store::metrics::GameFilter;
use rc_store::reports::{self, Report};
use rc_store::rusqlite::Connection;
use rc_store::settings::AppSettings;
use serde::Serialize;

use crate::CoachError;
use crate::benchmarks::GapsView;
use crate::patterns::load_games;
use crate::sessions::{Aggregate, SessionPattern, summarize};

pub const WEEKLY: &str = "weekly";
/// Weeks before the reported one used as its baseline.
const BASELINE_WEEKS: i64 = 4;
/// Weeks shown in the trend.
const TREND_WEEKS: i64 = 8;
/// Metrics in the weekly trend.
pub const TREND_METRICS: [&str; 6] = ["cs_10", "gold_diff_15", "deaths_pre14", "solo_deaths", "kp", "vision_pm"];

pub fn scope_of(week: &str) -> String {
    format!("week:{week}")
}

fn local_ms(d: NaiveDate) -> Option<i64> {
    let dt = d.and_hms_opt(0, 0, 0)?;
    Local.from_local_datetime(&dt).earliest().map(|t| t.timestamp_millis())
}

/// Monday of the local week containing `ms`, as `YYYY-MM-DD`.
pub fn week_of(ms: i64) -> String {
    let d = Local.timestamp_millis_opt(ms).single().map_or_else(|| Local::now().date_naive(), |t| t.date_naive());
    let monday = d - Duration::days(i64::from(d.weekday().num_days_from_monday()));
    monday.format("%Y-%m-%d").to_string()
}

/// `[start, end)` of a week in unix ms.
pub fn bounds(week: &str) -> Option<(i64, i64)> {
    let monday = NaiveDate::parse_from_str(week, "%Y-%m-%d").ok()?;
    if monday.weekday().num_days_from_monday() != 0 {
        return None;
    }
    Some((local_ms(monday)?, local_ms(monday + Duration::days(7))?))
}

fn shift(week: &str, weeks: i64) -> Option<String> {
    let monday = NaiveDate::parse_from_str(week, "%Y-%m-%d").ok()?;
    Some((monday + Duration::weeks(weeks)).format("%Y-%m-%d").to_string())
}

/// The last complete week before `now_ms`.
pub fn last_complete_week(now_ms: i64) -> Option<String> {
    shift(&week_of(now_ms), -1)
}

#[derive(Debug, Clone, Serialize)]
pub struct WeekSummary {
    pub week: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub games: usize,
    pub rift_games: usize,
    pub wins: usize,
    pub losses: usize,
    /// The week is over.
    pub complete: bool,
    pub has_report: bool,
}

pub fn list(conn: &Connection, account_id: i64, limit: usize, now_ms: i64) -> rc_store::Result<Vec<WeekSummary>> {
    let all = matches::list_account_matches(conn, account_id, 2000, 0)?;
    let mut weeks: BTreeMap<String, Vec<&matches::MatchSummary>> = BTreeMap::new();
    for m in &all {
        weeks.entry(week_of(m.game_start_ms)).or_default().push(m);
    }
    let mut out = Vec::new();
    for (week, ms) in weeks.into_iter().rev().take(limit) {
        let Some((start_ms, end_ms)) = bounds(&week) else { continue };
        let counted: Vec<_> = ms.iter().filter(|m| !m.is_remake).collect();
        out.push(WeekSummary {
            has_report: reports::latest_for_scope(conn, WEEKLY, account_id, &scope_of(&week))?.is_some(),
            rift_games: counted.iter().filter(|m| rc_common::queues::is_standard_rift(m.queue_id, m.map_id)).count(),
            wins: counted.iter().filter(|m| m.win).count(),
            losses: counted.iter().filter(|m| !m.win).count(),
            games: ms.len(),
            complete: now_ms >= end_ms,
            week,
            start_ms,
            end_ms,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct ChampionWeek {
    pub champion: String,
    pub champion_id: i32,
    pub games: usize,
    pub wins: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendPoint {
    pub week: String,
    pub games: usize,
    pub win_rate_pct: Option<f64>,
    /// Weekly average per trend metric (missing when no game had it).
    pub metrics: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeekDetail {
    pub summary: WeekSummary,
    pub baseline_games: usize,
    pub aggregates: Vec<Aggregate>,
    pub patterns: Vec<SessionPattern>,
    pub champions: Vec<ChampionWeek>,
    /// Oldest first, ending with this week.
    pub trend: Vec<TrendPoint>,
    /// This week's main role against the benchmark tier.
    pub benchmark: Option<GapsView>,
    pub report: Option<Report>,
}

fn average(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn trend_point(week: String, games: &[Game]) -> TrendPoint {
    let metrics = TREND_METRICS
        .iter()
        .filter_map(|k| {
            let v: Vec<f64> = games.iter().filter_map(|g| g.metric(k)).collect();
            average(&v).map(|a| ((*k).to_owned(), a))
        })
        .collect();
    let wins = games.iter().filter(|g| g.win).count();
    TrendPoint {
        week,
        games: games.len(),
        win_rate_pct: (!games.is_empty()).then(|| (wins as f64 / games.len() as f64 * 100.0).round()),
        metrics,
    }
}

pub fn detail(
    conn: &Connection,
    account_id: i64,
    week: &str,
    settings: &AppSettings,
    now_ms: i64,
) -> Result<WeekDetail, CoachError> {
    let (start, end) =
        bounds(week).ok_or_else(|| CoachError::Invalid(format!("{week} is not the Monday of a week")))?;
    let summary = list(conn, account_id, 10_000, now_ms)?
        .into_iter()
        .find(|w| w.week == week)
        .ok_or_else(|| CoachError::NotFound(format!("games in the week of {week}")))?;
    let range = |since: i64, before: i64| GameFilter {
        limit: 10_000,
        since_ms: Some(since),
        before_ms: Some(before),
        ..Default::default()
    };
    let mut games = load_games(conn, account_id, &range(start, end))?;
    games.reverse();
    let base_start = bounds(&shift(week, -BASELINE_WEEKS).unwrap_or_default()).map_or(start, |b| b.0);
    let baseline = load_games(conn, account_id, &range(base_start, start))?;
    let (aggregates, patterns) = summarize(&games, &baseline);

    let mut champs: BTreeMap<String, ChampionWeek> = BTreeMap::new();
    for g in matches::list_account_matches(conn, account_id, 2000, 0)?
        .into_iter()
        .filter(|m| m.game_start_ms >= start && m.game_start_ms < end && !m.is_remake)
        .filter(|m| rc_common::queues::is_standard_rift(m.queue_id, m.map_id))
    {
        let e = champs.entry(g.champion_name.clone()).or_insert(ChampionWeek {
            champion: g.champion_name.clone(),
            champion_id: g.champion_id,
            games: 0,
            wins: 0,
        });
        e.games += 1;
        e.wins += usize::from(g.win);
    }
    let mut champions: Vec<ChampionWeek> = champs.into_values().collect();
    champions.sort_by(|a, b| b.games.cmp(&a.games).then(a.champion.cmp(&b.champion)));

    let mut trend = Vec::new();
    for k in (0..TREND_WEEKS).rev() {
        let Some(w) = shift(week, -k) else { continue };
        let Some((s, e)) = bounds(&w) else { continue };
        let g = if k == 0 { games.clone() } else { load_games(conn, account_id, &range(s, e))? };
        trend.push(trend_point(w, &g));
    }

    // The week's main role against the benchmark tier.
    let mut roles: BTreeMap<&str, usize> = BTreeMap::new();
    for g in &games {
        if let Some(r) = g.role.as_deref() {
            *roles.entry(r).or_default() += 1;
        }
    }
    let main_role = roles.into_iter().max_by_key(|(_, n)| *n).map(|(r, _)| r.to_owned());
    let benchmark = match main_role {
        Some(role) => {
            let g = crate::benchmarks::gaps(conn, account_id, settings, &role, None, range(start, end))?;
            g.reference.is_some().then_some(g)
        }
        None => None,
    };

    Ok(WeekDetail {
        report: reports::latest_for_scope(conn, WEEKLY, account_id, &scope_of(week))?,
        baseline_games: baseline.len(),
        summary,
        aggregates,
        patterns,
        champions,
        trend,
        benchmark,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weeks_start_on_monday() {
        // 2026-10-14 is a Wednesday.
        let wed = local_ms(NaiveDate::from_ymd_opt(2026, 10, 14).expect("date")).expect("ms") + 3_600_000;
        assert_eq!(week_of(wed), "2026-10-12");
        let (s, e) = bounds("2026-10-12").expect("bounds");
        assert!(s <= wed && wed < e);
        assert!(e - s >= 167 * 3_600_000, "a week (allowing for DST)");
        assert!(bounds("2026-10-14").is_none(), "not a Monday");
        assert_eq!(shift("2026-10-12", -1).as_deref(), Some("2026-10-05"));
        assert_eq!(last_complete_week(wed).as_deref(), Some("2026-10-05"));
    }
}
