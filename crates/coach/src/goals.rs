//! Focus goals: creation (with a baseline from your recent games) and
//! progress from the games played since.

use rc_metrics::defs;
use rc_signals::Comparator;
use rc_signals::goals::{GoalGame, Progress, progress};
use rc_store::goals::{self as store_goals, Goal, NewGoal};
use rc_store::metrics::{self as store_metrics, GameFilter};
use rc_store::rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::CoachError;

/// Active goals at once (focus means few).
pub const MAX_ACTIVE: usize = 3;

#[derive(Debug, Clone, Deserialize)]
pub struct GoalInput {
    pub metric: String,
    pub comparator: String,
    pub target: f64,
    pub games: u32,
    pub role: Option<String>,
    pub champion_name: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalView {
    #[serde(flatten)]
    pub goal: Goal,
    pub label: &'static str,
    pub unit: defs::Unit,
    pub decimals: u8,
    pub higher_is_better: Option<bool>,
    /// e.g. "Deaths before 14:00 at most 1 (average over 10 games)".
    pub title: String,
    pub progress: Progress,
}

fn filter(g: &Goal, limit: u32) -> GameFilter {
    GameFilter {
        limit,
        since_ms: Some(g.created_at),
        before_ms: None,
        role: g.role.clone(),
        champion_name: g.champion_name.clone(),
    }
}

pub fn title(def: &defs::MetricDef, comparator: Comparator, target: f64, games: u32) -> String {
    let cmp = match comparator {
        Comparator::AtMost => "at most",
        Comparator::AtLeast => "at least",
    };
    format!("{} {cmp} {} (average over {games} games)", def.label, defs::format_value(def, target))
}

fn view(conn: &Connection, goal: Goal) -> Result<GoalView, CoachError> {
    let def = defs::get(&goal.metric).ok_or_else(|| CoachError::Invalid(format!("unknown metric {}", goal.metric)))?;
    let comparator = Comparator::parse(&goal.comparator)
        .ok_or_else(|| CoachError::Invalid(format!("bad comparator {}", goal.comparator)))?;
    // Oldest first, only games that have the metric.
    let mut rows = store_metrics::game_metrics(conn, goal.account_id, &filter(&goal, 10_000))?;
    rows.reverse();
    let games: Vec<GoalGame> = rows
        .into_iter()
        .filter_map(|r| {
            Some(GoalGame {
                value: *r.metrics.get(def.key)?,
                match_id: r.match_id,
                game_start_ms: r.game_start_ms,
                champion_name: r.champion_name,
                champion_id: r.champion_id,
                win: r.win,
                met: false,
            })
        })
        .collect();
    let p = progress(games, comparator, goal.target, goal.games, goal.baseline, def.higher_is_better);
    Ok(GoalView {
        label: def.label,
        unit: def.unit,
        decimals: def.decimals,
        higher_is_better: def.higher_is_better,
        title: title(def, comparator, goal.target, goal.games),
        progress: p,
        goal,
    })
}

/// Average of the metric over your last `baseline_games` matching games.
fn baseline(
    conn: &Connection,
    account_id: i64,
    input: &GoalInput,
    baseline_games: u32,
) -> rc_store::Result<Option<f64>> {
    let rows = store_metrics::game_metrics(
        conn,
        account_id,
        &GameFilter {
            limit: baseline_games,
            role: input.role.clone(),
            champion_name: input.champion_name.clone(),
            ..Default::default()
        },
    )?;
    let v: Vec<f64> = rows.iter().filter_map(|r| r.metrics.get(&input.metric).copied()).collect();
    Ok((!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64))
}

pub fn create(
    conn: &Connection,
    account_id: i64,
    input: &GoalInput,
    baseline_games: u32,
) -> Result<GoalView, CoachError> {
    let def =
        defs::get(&input.metric).ok_or_else(|| CoachError::Invalid(format!("Unknown metric {}.", input.metric)))?;
    let comparator = Comparator::parse(&input.comparator)
        .ok_or_else(|| CoachError::Invalid("Choose at most or at least.".into()))?;
    if !input.target.is_finite() {
        return Err(CoachError::Invalid("The target must be a number.".into()));
    }
    if !(1..=100).contains(&input.games) {
        return Err(CoachError::Invalid("A goal runs for 1 to 100 games.".into()));
    }
    if store_goals::list(conn, account_id, false)?.len() >= MAX_ACTIVE {
        return Err(CoachError::Invalid(format!(
            "You already have {MAX_ACTIVE} active goals. Finish or drop one first: focus works best on one or two."
        )));
    }
    let source = input.source.as_deref().filter(|s| matches!(*s, "ai" | "pattern")).unwrap_or("user");
    let base = baseline(conn, account_id, input, baseline_games)?;
    let id = store_goals::insert(
        conn,
        &NewGoal {
            account_id,
            metric: def.key,
            comparator: comparator.as_str(),
            target: input.target,
            games: input.games,
            role: input.role.as_deref(),
            champion_name: input.champion_name.as_deref(),
            baseline: base,
            source,
            note: input.note.as_deref(),
        },
        None,
    )?;
    let goal = store_goals::get(conn, id)?.ok_or_else(|| CoachError::NotFound("the goal just saved".into()))?;
    view(conn, goal)
}

/// Goals with progress. Active goals whose games are all played are
/// completed here (achieved if the average met the target).
pub fn list(conn: &Connection, account_id: i64, include_ended: bool) -> Result<Vec<GoalView>, CoachError> {
    let mut out = Vec::new();
    for goal in store_goals::list(conn, account_id, include_ended)? {
        let mut v = view(conn, goal)?;
        if v.goal.status == "active" && v.progress.done {
            let achieved = v.progress.on_track.unwrap_or(false);
            store_goals::set_status(conn, v.goal.id, "completed", Some(achieved))?;
            if let Some(g) = store_goals::get(conn, v.goal.id)? {
                v.goal = g;
            }
            if !include_ended {
                continue;
            }
        }
        out.push(v);
    }
    Ok(out)
}

/// `active`, `completed` or `abandoned`.
pub fn set_status(conn: &Connection, goal_id: i64, status: &str) -> Result<(), CoachError> {
    if !matches!(status, "active" | "completed" | "abandoned") {
        return Err(CoachError::Invalid(format!("bad status {status}")));
    }
    let goal = store_goals::get(conn, goal_id)?.ok_or_else(|| CoachError::NotFound(format!("goal {goal_id}")))?;
    let achieved = if status == "completed" { view(conn, goal)?.progress.on_track } else { None };
    store_goals::set_status(conn, goal_id, status, achieved)?;
    Ok(())
}
