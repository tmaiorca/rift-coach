//! The pre-game brief, built at the loading screen from the champions in
//! the game (Spectator-V5) and your own history: your record on your
//! champion, the likely lane opponent and your record against it, your
//! record against each enemy champion, your top habits to watch, your
//! focus goals, and benchmark targets. It never uses other players'
//! identities, ranks, or histories: only champions.

use std::collections::BTreeMap;

use rc_metrics::defs;
use rc_signals::Polarity;
use rc_store::metrics::{GameFilter, game_metrics, participant_metrics};
use rc_store::rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};

use crate::CoachError;
use crate::facts::round_to;

pub const PREGAME: &str = "pregame";
/// A champion is the likely lane opponent when at least this share of its
/// stored games are in your role.
const OPPONENT_MIN_SHARE: f64 = 0.4;
const RECORD_GAMES: u32 = 30;
/// Metrics averaged for records and the matchup.
const RECORD_METRICS: [&str; 5] = ["cs_10", "cs_diff_10", "gold_diff_15", "deaths_pre14", "solo_deaths"];
/// Benchmark targets shown in the brief.
const TARGET_METRICS: [&str; 4] = ["cs_10", "cspm", "deaths_pre14", "kp"];

pub fn scope_of(game_id: i64) -> String {
    format!("{PREGAME}:{game_id}")
}

#[derive(Debug, Clone, Serialize)]
pub struct Record {
    pub games: usize,
    pub wins: usize,
    /// Averages of [`RECORD_METRICS`] (only those present).
    pub averages: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Opponent {
    pub champion: String,
    pub champion_id: i32,
    /// Share of this champion's stored games played in your role, 0-100.
    pub role_share_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnemyRecord {
    pub champion: String,
    pub champion_id: i32,
    pub games: usize,
    pub wins: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub tip: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub metric: &'static str,
    pub label: &'static str,
    pub benchmark_median: f64,
    pub your_average: Option<f64>,
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct Brief {
    /// Spectator game id.
    pub game_id: i64,
    pub champion: String,
    pub champion_id: i32,
    /// Your usual role on this champion (Spectator doesn't report roles).
    pub role: Option<String>,
    pub record: Record,
    pub likely_opponent: Option<Opponent>,
    /// Your games in this role against the likely opponent.
    pub matchup: Option<Record>,
    pub enemies: Vec<EnemyRecord>,
    pub reminders: Vec<Reminder>,
    pub goals: Vec<String>,
    pub benchmark_label: Option<String>,
    pub targets: Vec<Target>,
}

fn champion_name(conn: &Connection, id: i32) -> rc_store::Result<String> {
    let name: Option<String> = conn
        .query_row("SELECT champion_name FROM match_participants WHERE champion_id = ?1 LIMIT 1", [id], |r| r.get(0))
        .ok();
    Ok(name.unwrap_or_else(|| format!("Champion {id}")))
}

fn record(conn: &Connection, rows: &[(String, i32, bool)]) -> rc_store::Result<Record> {
    let mut sums: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for (m, pid, _) in rows {
        let metrics = participant_metrics(conn, m, *pid)?;
        for k in RECORD_METRICS {
            if let Some(v) = metrics.get(k) {
                let e = sums.entry(k.to_owned()).or_default();
                e.0 += v;
                e.1 += 1;
            }
        }
    }
    Ok(Record {
        games: rows.len(),
        wins: rows.iter().filter(|r| r.2).count(),
        averages: sums.into_iter().map(|(k, (s, n))| (k, s / n as f64)).collect(),
    })
}

/// Your Summoner's Rift games where `champion_id` was on the enemy team
/// (in `lane_role` when given): (match id, your participant id, win).
fn games_against(
    conn: &Connection,
    account_id: i64,
    champion_id: i32,
    lane_role: Option<&str>,
) -> rc_store::Result<Vec<(String, i32, bool)>> {
    let queues = rc_common::queues::STANDARD_RIFT_QUEUES.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT am.match_id, me.participant_id, me.win
           FROM account_matches am
           JOIN matches m ON m.match_id = am.match_id
           JOIN match_participants me ON me.match_id = am.match_id AND me.participant_id = am.participant_id
           JOIN match_participants op ON op.match_id = am.match_id AND op.team_id != me.team_id
          WHERE am.account_id = ?1 AND op.champion_id = ?2 AND m.queue_id IN ({queues}) AND m.is_remake = 0
            AND (?3 IS NULL OR (me.team_position = ?3 AND op.team_position = ?3))
          ORDER BY m.game_start_ms DESC LIMIT 100"
    );
    let mut st = conn.prepare(&sql)?;
    Ok(st
        .query_map(params![account_id, champion_id, lane_role], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rc_store::rusqlite::Result<_>>()?)
}

/// Share of a champion's stored Summoner's Rift games played in `role`.
fn role_share(conn: &Connection, champion_id: i32, role: &str) -> rc_store::Result<(f64, i64)> {
    let (in_role, total): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(p.team_position = ?2), 0), COUNT(*) FROM match_participants p
           JOIN matches m ON m.match_id = p.match_id
          WHERE p.champion_id = ?1 AND m.map_id = 11 AND p.team_position IS NOT NULL",
        params![champion_id, role],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((if total > 0 { in_role as f64 / total as f64 } else { 0.0 }, total))
}

pub struct BriefInput {
    pub account_id: i64,
    pub game_id: i64,
    pub champion_id: i32,
    pub enemies: Vec<i32>,
}

pub fn brief(conn: &Connection, input: &BriefInput) -> Result<Brief, CoachError> {
    let settings = rc_store::settings::app(conn)?;
    let champion = champion_name(conn, input.champion_id)?;
    let on_champ = game_metrics(
        conn,
        input.account_id,
        &GameFilter { limit: RECORD_GAMES, champion_name: Some(champion.clone()), ..Default::default() },
    )?;
    // Your usual role on this champion, else your main role.
    let mut roles: BTreeMap<String, usize> = BTreeMap::new();
    for g in &on_champ {
        if let Some(r) = &g.team_position {
            *roles.entry(r.clone()).or_default() += 1;
        }
    }
    let role = match roles.into_iter().max_by_key(|(_, n)| *n).map(|(r, _)| r) {
        Some(r) => Some(r),
        None => crate::benchmarks::focus(conn, Some(input.account_id), &settings)?.roles.first().cloned(),
    };
    let rows: Vec<(String, i32, bool)> =
        on_champ.iter().map(|g| (g.match_id.clone(), g.participant_id, g.win)).collect();
    let rec = record(conn, &rows)?;

    let mut enemies = Vec::new();
    let mut best: Option<(f64, i32)> = None;
    for &e in &input.enemies {
        let games = games_against(conn, input.account_id, e, None)?;
        enemies.push(EnemyRecord {
            champion: champion_name(conn, e)?,
            champion_id: e,
            wins: games.iter().filter(|g| g.2).count(),
            games: games.len(),
        });
        if let Some(r) = &role {
            let (share, total) = role_share(conn, e, r)?;
            if total >= 3 && share >= OPPONENT_MIN_SHARE && best.is_none_or(|b| share > b.0) {
                best = Some((share, e));
            }
        }
    }
    let likely_opponent = match best {
        Some((share, id)) => Some(Opponent {
            champion: champion_name(conn, id)?,
            champion_id: id,
            role_share_pct: (share * 100.0).round(),
        }),
        None => None,
    };
    let matchup = match (&likely_opponent, &role) {
        (Some(o), Some(r)) => {
            let games = games_against(conn, input.account_id, o.champion_id, Some(r))?;
            (!games.is_empty()).then(|| record(conn, &games)).transpose()?
        }
        _ => None,
    };

    let reminders = match &role {
        Some(r) => crate::patterns::patterns(
            conn,
            input.account_id,
            settings.baseline_games,
            Some(r),
            settings.session_gap_min,
        )?
        .signals
        .into_iter()
        .filter(|s| s.polarity == Polarity::Weakness)
        .take(2)
        .map(|s| Reminder { id: format!("pattern:{}", s.id), title: s.title, detail: s.detail, tip: s.tip.to_owned() })
        .collect(),
        None => Vec::new(),
    };
    let goals = crate::goals::list(conn, input.account_id, false)?.into_iter().map(|g| g.title).collect();

    let (mut benchmark_label, mut targets) = (None, Vec::new());
    if let Some(r) = &role {
        let patch = crate::benchmarks::current_patch(conn)?;
        let tier = crate::benchmarks::effective_tier(conn, &settings)?;
        let (role_pop, champ_pop) = crate::benchmarks::populations(conn, &tier, patch.as_deref(), r, Some(&champion))?;
        if let Some(b) = champ_pop.or(role_pop) {
            for k in TARGET_METRICS {
                let (Some(d), Some(v)) = (defs::get(k), b.pop.get(k)) else { continue };
                if v.len() < crate::benchmarks::MIN_CHAMPION_SAMPLE {
                    continue;
                }
                targets.push(Target {
                    metric: d.key,
                    label: d.label,
                    benchmark_median: rc_metrics::percentile::median(v),
                    your_average: rec.averages.get(k).copied().or_else(|| {
                        let mine: Vec<f64> = on_champ.iter().filter_map(|g| g.metrics.get(k).copied()).collect();
                        (!mine.is_empty()).then(|| mine.iter().sum::<f64>() / mine.len() as f64)
                    }),
                    decimals: d.decimals,
                });
            }
            benchmark_label = Some(b.label);
        }
    }

    Ok(Brief {
        game_id: input.game_id,
        champion,
        champion_id: input.champion_id,
        role,
        record: rec,
        likely_opponent,
        matchup,
        enemies,
        reminders,
        goals,
        benchmark_label,
        targets,
    })
}

fn record_json(r: &Record) -> Value {
    json!({
        "games": r.games,
        "wins": r.wins,
        "averages": r.averages.iter().map(|(k, v)| {
            let d = defs::get(k).map_or(1, |d| d.decimals.max(1));
            (k.clone(), json!(round_to(*v, d)))
        }).collect::<serde_json::Map<_, _>>(),
    })
}

/// Facts for the AI game plan.
pub fn facts(b: &Brief) -> Value {
    let mut notes = vec![
        "Only champions are known about the other players; nothing about their skill or history.".to_string(),
        "The role is your usual role on this champion; the game itself doesn't report roles.".to_string(),
    ];
    if b.record.games < 5 {
        notes.push("Few games on this champion: the record is thin.".into());
    }
    json!({
        "record": {
            "id": "record",
            "champion": b.champion,
            "role": b.role,
            "your_recent_games": record_json(&b.record),
        },
        "matchup": {
            "id": "matchup",
            "likely_lane_opponent": b.likely_opponent.as_ref().map(|o| &o.champion),
            "opponent_plays_your_role_pct": b.likely_opponent.as_ref().map(|o| o.role_share_pct),
            "your_games_vs_them_in_lane": b.matchup.as_ref().map(record_json),
        },
        "enemies": b.enemies.iter().map(|e| json!({
            "id": format!("enemy:{}", e.champion),
            "champion": e.champion,
            "your_games_against": e.games,
            "your_wins_against": e.wins,
        })).collect::<Vec<_>>(),
        "patterns": b.reminders.iter().map(|r| json!({"id": r.id, "title": r.title, "detail": r.detail})).collect::<Vec<_>>(),
        "goals": b.goals.iter().enumerate().map(|(i, g)| json!({"id": format!("goal_{}", i + 1), "title": g})).collect::<Vec<_>>(),
        "benchmark_targets": {
            "reference": b.benchmark_label,
            "metrics": b.targets.iter().map(|t| json!({
                "id": format!("bench:{}", t.metric),
                "label": t.label,
                "benchmark_median": round_to(t.benchmark_median, t.decimals.max(1)),
                "your_average": t.your_average.map(|v| round_to(v, t.decimals.max(1))),
            })).collect::<Vec<_>>(),
        },
        "notes": notes,
    })
}
