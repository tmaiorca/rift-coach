//! Loads the player's games and comparison populations from the store and
//! runs the signal detectors (rc-signals) over them.

use std::collections::{BTreeMap, HashMap};

use rc_metrics::zones::Zone;
use rc_signals::{Analysis, Context, Death, Game, GameFlag, Signal, WinLink};
use rc_store::metrics::{self as store_metrics, DeathRow, GameFilter, GameMetrics};
use rc_store::rusqlite::Connection;
use serde::Serialize;

fn death(d: DeathRow) -> Option<Death> {
    Some(Death {
        t_s: d.t_ms as f64 / 1000.0,
        x: d.x,
        y: d.y,
        zone: Zone::parse(&d.zone)?,
        enemies: d.enemies,
        solo: d.solo,
        before_14: d.before_14,
        enemy_side: d.enemy_side,
        gold_vs_opponent: d.gold_vs_opponent,
        unspent_gold: d.unspent_gold,
        shutdown_bounty: d.shutdown_bounty,
        enemy_objective: d.enemy_objective,
        killer_champion: d.killer_champion,
    })
}

pub fn to_game(conn: &Connection, g: GameMetrics) -> rc_store::Result<Game> {
    let deaths = store_metrics::participant_deaths(conn, &g.match_id, g.participant_id)?;
    Ok(Game {
        match_id: g.match_id,
        start_ms: g.game_start_ms,
        duration_s: g.duration_s,
        win: g.win,
        role: g.team_position,
        champion: g.champion_name,
        // Riot numbers the blue team 1-5.
        blue_side: g.participant_id <= 5,
        metrics: g.metrics,
        deaths: deaths.into_iter().filter_map(death).collect(),
    })
}

/// The account's games matching `filter`, newest first, with deaths.
pub fn load_games(conn: &Connection, account_id: i64, filter: &GameFilter) -> rc_store::Result<Vec<Game>> {
    store_metrics::game_metrics(conn, account_id, filter)?.into_iter().map(|g| to_game(conn, g)).collect()
}

/// Populations for the roles that appear in `games`.
pub fn context(conn: &Connection, account_id: i64, games: &[Game], session_gap_min: u32) -> rc_store::Result<Context> {
    let mut ctx = Context { session_gap_ms: i64::from(session_gap_min) * 60_000, ..Default::default() };
    let mut roles: Vec<&str> = games.iter().filter_map(|g| g.role.as_deref()).collect();
    roles.sort_unstable();
    roles.dedup();
    for role in roles {
        ctx.role_populations.insert(role.to_owned(), store_metrics::role_population(conn, account_id, role, None)?);
        let zones: BTreeMap<Zone, u32> = store_metrics::role_death_zones(conn, account_id, role)?
            .into_iter()
            .filter_map(|(z, n)| Some((Zone::parse(&z)?, n)))
            .collect();
        ctx.role_death_zones.insert(role.to_owned(), zones);
    }
    Ok(ctx)
}

/// One death on the patterns heat map.
#[derive(Debug, Clone, Serialize)]
pub struct MapDeath {
    pub match_id: String,
    pub t_s: f64,
    pub x: i32,
    pub y: i32,
    pub zone: Zone,
    pub win: bool,
    pub blue_side: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PatternsView {
    pub window: u32,
    pub role: Option<String>,
    /// Roles in the window with game counts, most played first.
    pub roles: Vec<(String, usize)>,
    pub games: usize,
    pub wins: usize,
    pub signals: Vec<Signal>,
    pub win_links: Vec<WinLink>,
    pub deaths: Vec<MapDeath>,
}

pub fn patterns(
    conn: &Connection,
    account_id: i64,
    window: u32,
    role: Option<&str>,
    session_gap_min: u32,
) -> rc_store::Result<PatternsView> {
    let all = load_games(conn, account_id, &GameFilter { limit: window, ..Default::default() })?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for g in &all {
        if let Some(r) = &g.role {
            *counts.entry(r.clone()).or_default() += 1;
        }
    }
    let mut roles: Vec<(String, usize)> = counts.into_iter().collect();
    roles.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let games: Vec<Game> = match role {
        Some(r) => all.into_iter().filter(|g| g.role.as_deref() == Some(r)).collect(),
        None => all,
    };
    let ctx = context(conn, account_id, &games, session_gap_min)?;
    let Analysis { games: n, wins, signals, win_links } = rc_signals::analyze(&games, &ctx);
    let deaths = games
        .iter()
        .flat_map(|g| {
            g.deaths.iter().map(|d| MapDeath {
                match_id: g.match_id.clone(),
                t_s: d.t_s,
                x: d.x,
                y: d.y,
                zone: d.zone,
                win: g.win,
                blue_side: g.blue_side,
            })
        })
        .collect();
    Ok(PatternsView { window, role: role.map(str::to_owned), roles, games: n, wins, signals, win_links, deaths })
}

/// A pattern this game shows, and whether it's one of your recurring ones.
#[derive(Debug, Clone, Serialize)]
pub struct MatchPattern {
    #[serde(flatten)]
    pub flag: GameFlag,
    /// Set when the same pattern recurs in your recent games.
    pub recurring: Option<Recurring>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recurring {
    pub games_affected: usize,
    pub games_considered: usize,
    pub rate_pct: f64,
    pub win_rate_affected: Option<f64>,
    pub win_rate_other: Option<f64>,
}

/// Patterns in one game, judged against the `window` games up to and
/// including it ("as of this game").
pub fn match_patterns(
    conn: &Connection,
    account_id: i64,
    match_id: &str,
    game_start_ms: i64,
    window: u32,
    session_gap_min: u32,
) -> rc_store::Result<Vec<MatchPattern>> {
    let games = load_games(
        conn,
        account_id,
        &GameFilter { limit: window, before_ms: Some(game_start_ms + 1), ..Default::default() },
    )?;
    let Some(this) = games.iter().find(|g| g.match_id == match_id) else { return Ok(vec![]) };
    let flags = rc_signals::game_flags(this);
    if flags.is_empty() {
        return Ok(vec![]);
    }
    let ctx = context(conn, account_id, &games, session_gap_min)?;
    let signals = rc_signals::analyze(&games, &ctx).signals;
    Ok(flags
        .into_iter()
        .map(|flag| {
            let recurring = signals.iter().find(|s| s.id == flag.kind).map(|s| Recurring {
                games_affected: s.games_affected,
                games_considered: s.games_considered,
                rate_pct: s.rate_pct,
                win_rate_affected: s.win_rate_affected,
                win_rate_other: s.win_rate_other,
            });
            MatchPattern { flag, recurring }
        })
        .collect())
}
