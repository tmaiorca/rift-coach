//! The live view's static context, built once when a game starts: your CS
//! target, a lobby benchmark, your planned (or usual) build, and your active
//! focus goals. Everything here is decided before the game; nothing reacts
//! to live state.

use std::collections::HashMap;

use rc_live::view::{Benchmark, CsTarget, GoalReminder, LiveContext, PlannedItem, TargetSource};
use rc_metrics::{ItemCatalog, defs};
use rc_store::metrics::{self as store_metrics, GameFilter};
use rc_store::rusqlite::Connection;
use rc_store::settings::AppSettings;
use serde::Serialize;

use crate::CoachError;

/// Benchmark percentile from the role population in your own games.
const BENCHMARK_PCT: f64 = 70.0;
const BENCHMARK_MIN_POPULATION: usize = 20;
/// Games used for "your usual" CS target and build.
const USUAL_TARGET_MIN_GAMES: usize = 3;
const USUAL_BUILD_GAMES: u32 = 20;
const USUAL_BUILD_ITEMS: usize = 6;

#[derive(Debug, Clone, Copy, Default)]
pub struct ContextInput<'a> {
    pub account_id: Option<i64>,
    pub champion_id: Option<i32>,
    /// Data Dragon id, which is also Match-V5's `championName`.
    pub champion: Option<&'a str>,
    pub role: Option<&'a str>,
    /// Data Dragon patch of `catalog`.
    pub patch: Option<&'a str>,
}

fn percentile(values: &[f64], pct: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = (pct / 100.0 * (v.len() - 1) as f64).round() as usize;
    v[rank.min(v.len() - 1)]
}

fn goal_applies(goal_role: Option<&str>, goal_champ: Option<&str>, role: Option<&str>, champ: Option<&str>) -> bool {
    goal_role.is_none_or(|r| Some(r) == role)
        && goal_champ.is_none_or(|c| champ.is_some_and(|x| x.eq_ignore_ascii_case(c)))
}

pub fn context(
    conn: &Connection,
    input: ContextInput<'_>,
    settings: &AppSettings,
    catalog: Option<&ItemCatalog>,
) -> Result<LiveContext, CoachError> {
    let role = input.role.filter(|r| !r.is_empty());
    let mut ctx = LiveContext {
        account_id: input.account_id,
        champion_id: input.champion_id,
        champion_ddragon_id: input.champion.map(str::to_owned),
        role: role.map(str::to_owned),
        patch: input.patch.map(str::to_owned),
        ..Default::default()
    };

    if let Some(account_id) = input.account_id {
        let goals = rc_store::goals::list(conn, account_id, false)?;
        ctx.goals = goals
            .iter()
            .filter(|g| goal_applies(g.role.as_deref(), g.champion_name.as_deref(), role, input.champion))
            .filter_map(|g| {
                let def = defs::get(&g.metric)?;
                let cmp = rc_signals::Comparator::parse(&g.comparator)?;
                Some(GoalReminder {
                    goal_id: g.id,
                    title: crate::goals::title(def, cmp, g.target, g.games),
                    metric: g.metric.clone(),
                    label: def.label.to_owned(),
                    comparator: g.comparator.clone(),
                    target: g.target,
                })
            })
            .collect();
    }

    // CS target: the setting, else an active CS/min goal, else your usual.
    ctx.cs_target = if let Some(t) = settings.live.cs_target_pm.filter(|t| *t > 0.0) {
        Some(CsTarget { per_min: t, source: TargetSource::Setting })
    } else if let Some(g) = ctx.goals.iter().find(|g| g.metric == "cspm" && g.comparator == "at_least") {
        Some(CsTarget { per_min: g.target, source: TargetSource::Goal })
    } else if let (Some(account_id), Some(role)) = (input.account_id, role) {
        let rows = store_metrics::game_metrics(
            conn,
            account_id,
            &GameFilter { limit: settings.baseline_games, role: Some(role.to_owned()), ..Default::default() },
        )?;
        let v: Vec<f64> = rows.iter().filter_map(|r| r.metrics.get("cspm").copied()).collect();
        (v.len() >= USUAL_TARGET_MIN_GAMES)
            .then(|| CsTarget { per_min: v.iter().sum::<f64>() / v.len() as f64, source: TargetSource::Usual })
    } else {
        None
    };

    // Benchmark: the higher-rank median on this champion (or role) when
    // collected, else the top 30% of your own lobbies.
    if let Some(role) = role {
        let patch = crate::benchmarks::current_patch(conn)?;
        let tier = crate::benchmarks::effective_tier(conn, settings)?;
        let (role_pop, champ_pop) =
            crate::benchmarks::populations(conn, &tier, patch.as_deref(), role, input.champion)?;
        if let Some(b) = champ_pop.or(role_pop)
            && let Some(v) = b.pop.get("cspm").filter(|v| v.len() >= crate::benchmarks::MIN_CHAMPION_SAMPLE)
        {
            ctx.benchmark =
                Some(Benchmark { per_min: rc_metrics::percentile::median(v), label: format!("median of {}", b.label) });
        }
    }
    if let (None, Some(account_id), Some(role)) = (&ctx.benchmark, input.account_id, role) {
        let pop = store_metrics::role_population(conn, account_id, role, None)?;
        if let Some(v) = pop.get("cspm").filter(|v| v.len() >= BENCHMARK_MIN_POPULATION) {
            ctx.benchmark = Some(Benchmark {
                per_min: percentile(v, BENCHMARK_PCT),
                label: format!("top 30% of {} in your games", defs::role_label(role)),
            });
        }
    }

    if let (Some(champion), Some(catalog)) = (input.champion, catalog) {
        let (ids, source) = match rc_store::live::build_plan(conn, champion)? {
            Some(ids) => (ids, "plan"),
            None => match input.account_id {
                Some(a) => (usual_build(conn, a, champion, catalog)?, "usual"),
                None => (Vec::new(), "usual"),
            },
        };
        ctx.build = planned(&ids, catalog);
        ctx.build_source = (!ctx.build.is_empty()).then_some(source);
    }
    Ok(ctx)
}

pub fn planned(ids: &[i32], catalog: &ItemCatalog) -> Vec<PlannedItem> {
    ids.iter()
        .filter_map(|id| {
            let i = catalog.get(*id)?;
            Some(PlannedItem { item_id: *id, name: i.name.clone(), total_gold: i.total_gold })
        })
        .collect()
}

/// Your most common finished items (and boots) on a champion over your
/// recent games, most common first. Built from final inventories, so the
/// order is by frequency, not purchase order.
pub fn usual_build(
    conn: &Connection,
    account_id: i64,
    champion: &str,
    catalog: &ItemCatalog,
) -> Result<Vec<i32>, CoachError> {
    let games = store_metrics::game_metrics(
        conn,
        account_id,
        &GameFilter { limit: USUAL_BUILD_GAMES, champion_name: Some(champion.to_owned()), ..Default::default() },
    )?;
    // item → (games, sum of slot positions among finished items)
    let mut counts: HashMap<i32, (u32, usize)> = HashMap::new();
    for g in &games {
        let Some(raw) = rc_store::matches::get_raw(conn, &g.match_id, rc_store::matches::RawKind::Match)? else {
            continue;
        };
        let game = rc_metrics::parse_match(&raw)?;
        let Some(p) = game.info.participants.iter().find(|p| p.participant_id == g.participant_id) else { continue };
        let finished: Vec<i32> = [p.item0, p.item1, p.item2, p.item3, p.item4, p.item5]
            .into_iter()
            .filter(|id| catalog.get(*id).is_some_and(|i| i.plannable))
            .collect();
        for (pos, id) in finished.iter().enumerate() {
            let e = counts.entry(*id).or_default();
            e.0 += 1;
            e.1 += pos;
        }
    }
    let mut ranked: Vec<(i32, u32, f64)> =
        counts.into_iter().map(|(id, (n, pos))| (id, n, pos as f64 / f64::from(n))).collect();
    // Most common first; ties go to the item usually earlier in the inventory.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.total_cmp(&b.2)).then(a.0.cmp(&b.0)));
    let is_boots = |id: i32| catalog.get(id).is_some_and(|i| i.from.contains(&1001));
    let mut out = Vec::new();
    let mut have_boots = false;
    for (id, n, _) in ranked {
        if n < 2 && !out.is_empty() {
            continue;
        }
        if is_boots(id) {
            if have_boots {
                continue;
            }
            have_boots = true;
        }
        out.push(id);
        if out.len() == USUAL_BUILD_ITEMS {
            break;
        }
    }
    Ok(out)
}

/// An item for the build planner.
#[derive(Debug, Clone, Serialize)]
pub struct PlannerItem {
    pub item_id: i32,
    pub name: String,
    pub total_gold: i32,
    pub completed: bool,
}

pub fn planner_items(catalog: &ItemCatalog) -> Vec<PlannerItem> {
    catalog
        .plannable()
        .into_iter()
        .map(|(id, i)| PlannerItem {
            item_id: id,
            name: i.name.clone(),
            total_gold: i.total_gold,
            completed: i.completed,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_and_goal_filters() {
        let v: Vec<f64> = (1..=11).map(f64::from).collect();
        assert_eq!(percentile(&v, 70.0), 8.0);
        assert!(goal_applies(None, None, Some("MIDDLE"), Some("Leblanc")));
        assert!(goal_applies(Some("MIDDLE"), Some("leblanc"), Some("MIDDLE"), Some("Leblanc")));
        assert!(!goal_applies(Some("TOP"), None, Some("MIDDLE"), None));
        assert!(!goal_applies(None, Some("Ahri"), Some("MIDDLE"), Some("Leblanc")));
    }
}
