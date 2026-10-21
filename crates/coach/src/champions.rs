//! Champion pages: every champion you play (not just one main), with your
//! record, typical numbers against your own average in that role, lane
//! matchups, the patterns that show up on it, and the benchmark gaps on it.

use std::collections::{BTreeMap, HashMap};

use rc_signals::{Game, Signal};
use rc_store::metrics::{GameFilter, GameMetrics, game_metrics};
use rc_store::rusqlite::{Connection, params};
use rc_store::settings::AppSettings;
use serde::Serialize;

use crate::CoachError;
use crate::benchmarks::GapsView;
use crate::patterns::{context, load_games};

/// Summoner's Rift games the champion list looks at.
const LIST_GAMES: u32 = 300;
/// Numbers shown per champion.
pub const CHAMPION_METRICS: [&str; 8] =
    ["kda", "cs_10", "cspm", "gold_diff_15", "deaths_pre14", "kp", "dpm", "vision_pm"];
/// Lane matchups shown on a champion page.
const MAX_MATCHUPS: usize = 20;

#[derive(Debug, Clone, Serialize)]
pub struct ChampionSummary {
    /// Data Dragon id (= Match-V5 `championName`).
    pub champion: String,
    pub champion_id: i32,
    pub games: usize,
    pub wins: usize,
    /// Roles played on it with game counts, most first.
    pub roles: Vec<(String, usize)>,
    pub last_played_ms: i64,
    /// Averages of [`CHAMPION_METRICS`] that the games have.
    pub averages: BTreeMap<String, f64>,
    /// Benchmarks sample this champion first.
    pub focus: bool,
}

fn average(v: impl Iterator<Item = f64>) -> Option<f64> {
    let (s, n) = v.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| s / n as f64)
}

fn averages(games: &[&GameMetrics]) -> BTreeMap<String, f64> {
    CHAMPION_METRICS
        .iter()
        .filter_map(|k| average(games.iter().filter_map(|g| g.metrics.get(*k).copied())).map(|a| ((*k).to_owned(), a)))
        .collect()
}

fn summarize(champion: &str, games: &[&GameMetrics], focus: &[String]) -> Option<ChampionSummary> {
    let first = games.first()?;
    let mut roles: HashMap<String, usize> = HashMap::new();
    for g in games {
        if let Some(r) = &g.team_position {
            *roles.entry(r.clone()).or_default() += 1;
        }
    }
    let mut roles: Vec<(String, usize)> = roles.into_iter().collect();
    roles.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Some(ChampionSummary {
        champion: champion.to_owned(),
        champion_id: first.champion_id,
        games: games.len(),
        wins: games.iter().filter(|g| g.win).count(),
        roles,
        last_played_ms: games.iter().map(|g| g.game_start_ms).max().unwrap_or(0),
        averages: averages(games),
        focus: focus.iter().any(|f| f.eq_ignore_ascii_case(champion)),
    })
}

/// Champions in your recent Summoner's Rift games, most played first.
pub fn list(conn: &Connection, account_id: i64, settings: &AppSettings) -> rc_store::Result<Vec<ChampionSummary>> {
    let all = game_metrics(conn, account_id, &GameFilter { limit: LIST_GAMES, ..Default::default() })?;
    let focus = crate::benchmarks::focus(conn, Some(account_id), settings)?.champions;
    let mut by: BTreeMap<&str, Vec<&GameMetrics>> = BTreeMap::new();
    for g in &all {
        by.entry(g.champion_name.as_str()).or_default().push(g);
    }
    let mut out: Vec<ChampionSummary> = by.iter().filter_map(|(c, gs)| summarize(c, gs, &focus)).collect();
    out.sort_by(|a, b| b.games.cmp(&a.games).then(b.last_played_ms.cmp(&a.last_played_ms)));
    Ok(out)
}

/// Your record against one lane opponent on this champion.
#[derive(Debug, Clone, Serialize)]
pub struct Matchup {
    pub champion: String,
    pub champion_id: i32,
    pub games: usize,
    pub wins: usize,
    /// Your average gold and CS difference with them (lane diffs).
    pub gold_diff_15: Option<f64>,
    pub cs_diff_10: Option<f64>,
    pub deaths_pre14: Option<f64>,
}

/// One number on the champion against your average in the same role.
#[derive(Debug, Clone, Serialize)]
pub struct ChampionVsRole {
    pub metric: &'static str,
    pub label: &'static str,
    pub on_champion: f64,
    pub role_average: Option<f64>,
    /// Better on this champion than your role average (`None` = no direction).
    pub better: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentGame {
    pub match_id: String,
    pub start_ms: i64,
    pub win: bool,
    pub role: Option<String>,
    pub opponent: Option<String>,
    pub opponent_id: Option<i32>,
    pub kills: Option<f64>,
    pub deaths: Option<f64>,
    pub assists: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChampionDetail {
    pub summary: ChampionSummary,
    /// The role most played on it (what the comparisons use).
    pub role: Option<String>,
    /// Your games in that role on other champions.
    pub role_games: usize,
    pub vs_role: Vec<ChampionVsRole>,
    pub matchups: Vec<Matchup>,
    /// Recurring patterns in your games on this champion.
    pub signals: Vec<Signal>,
    pub benchmark: Option<GapsView>,
    pub recent: Vec<RecentGame>,
}

/// Your lane opponent (same `teamPosition`, enemy team) in each game.
fn lane_opponents(
    conn: &Connection,
    account_id: i64,
    champion: &str,
) -> rc_store::Result<HashMap<String, (String, i32)>> {
    let mut st = conn.prepare(
        "SELECT am.match_id, op.champion_name, op.champion_id
           FROM account_matches am
           JOIN match_participants me ON me.match_id = am.match_id AND me.participant_id = am.participant_id
           JOIN match_participants op ON op.match_id = am.match_id AND op.team_id != me.team_id
                                      AND op.team_position = me.team_position
          WHERE am.account_id = ?1 AND me.champion_name = ?2 COLLATE NOCASE AND me.team_position IS NOT NULL
                AND me.team_position != ''",
    )?;
    let rows = st.query_map(params![account_id, champion], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))?;
    Ok(rows.collect::<rc_store::rusqlite::Result<_>>()?)
}

pub fn detail(
    conn: &Connection,
    account_id: i64,
    champion: &str,
    settings: &AppSettings,
) -> Result<ChampionDetail, CoachError> {
    let filter = GameFilter { limit: LIST_GAMES, champion_name: Some(champion.to_owned()), ..Default::default() };
    let rows = game_metrics(conn, account_id, &filter)?;
    let focus = crate::benchmarks::focus(conn, Some(account_id), settings)?.champions;
    let refs: Vec<&GameMetrics> = rows.iter().collect();
    let summary = summarize(champion, &refs, &focus)
        .ok_or_else(|| CoachError::NotFound(format!("Summoner's Rift games on {champion}")))?;
    let champion = summary.champion.clone();
    let role = summary.roles.first().map(|(r, _)| r.clone());

    // This champion against your other champions in the same role.
    let (mut vs_role, mut role_games) = (Vec::new(), 0);
    if let Some(r) = &role {
        let others: Vec<GameMetrics> = game_metrics(
            conn,
            account_id,
            &GameFilter { limit: LIST_GAMES, role: Some(r.clone()), ..Default::default() },
        )?
        .into_iter()
        .filter(|g| !g.champion_name.eq_ignore_ascii_case(&champion))
        .collect();
        role_games = others.len();
        let mine: Vec<&GameMetrics> = refs.iter().copied().filter(|g| g.team_position.as_deref() == Some(r)).collect();
        for k in CHAMPION_METRICS {
            let Some(d) = rc_metrics::defs::get(k) else { continue };
            let Some(on) = average(mine.iter().filter_map(|g| g.metrics.get(k).copied())) else { continue };
            let base = average(others.iter().filter_map(|g| g.metrics.get(k).copied()));
            let better = match (base, d.higher_is_better) {
                (Some(b), Some(h)) if (on - b).abs() > f64::EPSILON => Some((on > b) == h),
                _ => None,
            };
            vs_role.push(ChampionVsRole { metric: d.key, label: d.label, on_champion: on, role_average: base, better });
        }
    }

    // Lane matchups.
    let opponents = lane_opponents(conn, account_id, &champion)?;
    let mut mu: BTreeMap<(String, i32), Vec<&GameMetrics>> = BTreeMap::new();
    for g in &rows {
        if let Some(o) = opponents.get(&g.match_id) {
            mu.entry(o.clone()).or_default().push(g);
        }
    }
    let avg_of = |gs: &[&GameMetrics], k: &str| average(gs.iter().filter_map(|g| g.metrics.get(k).copied()));
    let mut matchups: Vec<Matchup> = mu
        .into_iter()
        .map(|((name, id), gs)| Matchup {
            gold_diff_15: avg_of(&gs, "gold_diff_15"),
            cs_diff_10: avg_of(&gs, "cs_diff_10"),
            deaths_pre14: avg_of(&gs, "deaths_pre14"),
            games: gs.len(),
            wins: gs.iter().filter(|g| g.win).count(),
            champion: name,
            champion_id: id,
        })
        .collect();
    matchups.sort_by(|a, b| b.games.cmp(&a.games).then(a.champion.cmp(&b.champion)));
    matchups.truncate(MAX_MATCHUPS);

    // Patterns on this champion (the same detectors as the Patterns page).
    let games: Vec<Game> = load_games(conn, account_id, &filter)?;
    let ctx = context(conn, account_id, &games, settings.session_gap_min)?;
    let signals = rc_signals::analyze(&games, &ctx).signals;

    let benchmark = match &role {
        Some(r) => {
            let window = GameFilter { limit: settings.baseline_games, ..Default::default() };
            let g = crate::benchmarks::gaps(conn, account_id, settings, r, Some(&champion), window)?;
            g.reference.is_some().then_some(g)
        }
        None => None,
    };

    let recent = rows
        .iter()
        .take(10)
        .map(|g| {
            let o = opponents.get(&g.match_id);
            RecentGame {
                match_id: g.match_id.clone(),
                start_ms: g.game_start_ms,
                win: g.win,
                role: g.team_position.clone(),
                opponent: o.map(|o| o.0.clone()),
                opponent_id: o.map(|o| o.1),
                kills: g.metrics.get("kills").copied(),
                deaths: g.metrics.get("deaths").copied(),
                assists: g.metrics.get("assists").copied(),
            }
        })
        .collect();

    Ok(ChampionDetail { summary, role, role_games, vs_role, matchups, signals, benchmark, recent })
}
