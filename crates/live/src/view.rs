//! Live view numbers: CS pacing against your target, gold toward the next
//! item of your planned build, and your focus goals' values this game. Only
//! the player's own numbers and static, pre-set targets: no advice, no
//! alerts, nothing about other players.

use rc_metrics::ItemCatalog;
use serde::Serialize;

use crate::model::{LiveItem, OwnSnapshot};

/// Pacing is shown from this game time on (before it, the first waves make
/// any per-minute rate meaningless).
pub const PACING_FROM_S: f64 = 300.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSource {
    /// An active focus goal on CS per minute.
    Goal,
    /// The target set in Settings.
    Setting,
    /// Your own average over your recent games in this role.
    Usual,
}

#[derive(Debug, Clone, Serialize)]
pub struct CsTarget {
    pub per_min: f64,
    pub source: TargetSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct Benchmark {
    pub per_min: f64,
    /// e.g. "top 30% of mid laners in your games".
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalReminder {
    pub goal_id: i64,
    pub title: String,
    pub metric: String,
    pub label: String,
    /// `at_most` / `at_least`.
    pub comparator: String,
    pub target: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedItem {
    pub item_id: i32,
    pub name: String,
    pub total_gold: i32,
}

/// Static for the whole game: loaded once when it starts.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveContext {
    pub account_id: Option<i64>,
    pub champion_id: Option<i32>,
    pub champion_ddragon_id: Option<String>,
    /// Role key (`MIDDLE`...) used for targets.
    pub role: Option<String>,
    pub cs_target: Option<CsTarget>,
    pub benchmark: Option<Benchmark>,
    pub build: Vec<PlannedItem>,
    /// `plan` (saved by you) or `usual` (your most common build).
    pub build_source: Option<&'static str>,
    pub goals: Vec<GoalReminder>,
    /// Patch of the item data, for icons.
    pub patch: Option<String>,
}

/// Values frozen at a game time (CS at 10:00, deaths before 14:00).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Marks {
    /// `None` if the poller wasn't running at 10:00.
    pub cs_10: Option<i32>,
    pub deaths_14: Option<i32>,
    /// First game time seen; marks before it are unknown.
    first_seen_s: Option<f64>,
}

impl Marks {
    pub fn update(&mut self, s: &OwnSnapshot) {
        let first = *self.first_seen_s.get_or_insert(s.game_time_s);
        if self.cs_10.is_none() && s.game_time_s >= 600.0 && first < 600.0 {
            self.cs_10 = Some(s.scores.creep_score);
        }
        if self.deaths_14.is_none() && s.game_time_s >= 840.0 && first < 840.0 {
            self.deaths_14 = Some(s.scores.deaths);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Pacing {
    pub cs: i32,
    pub cs_per_min: f64,
    pub target: Option<CsTarget>,
    /// Target × minutes played (from [`PACING_FROM_S`]).
    pub expected_cs: Option<f64>,
    /// cs − expected.
    pub diff: Option<f64>,
    pub benchmark: Option<Benchmark>,
}

pub fn pacing(s: &OwnSnapshot, ctx: &LiveContext) -> Pacing {
    let minutes = s.game_time_s / 60.0;
    let cs = s.scores.creep_score;
    let cs_per_min = if minutes > 0.0 { f64::from(cs) / minutes } else { 0.0 };
    let expected_cs = ctx.cs_target.as_ref().filter(|_| s.game_time_s >= PACING_FROM_S).map(|t| t.per_min * minutes);
    Pacing {
        cs,
        cs_per_min,
        diff: expected_cs.map(|e| f64::from(cs) - e),
        expected_cs,
        target: ctx.cs_target.clone(),
        benchmark: ctx.benchmark.clone(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanSlot {
    pub item_id: i32,
    pub name: String,
    pub owned: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NextItem {
    pub item_id: i32,
    pub name: String,
    pub total_gold: i32,
    /// Gold already in its components you own.
    pub components_gold: i32,
    /// Gold still needed beyond components and current gold (0 = covered).
    pub remaining_gold: i32,
    /// (components + current gold) / cost, 0–100.
    pub progress_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildProgress {
    pub plan: Vec<PlanSlot>,
    pub next: Option<NextItem>,
}

/// Gold value of the owned items that fit into `id`'s recipe, consuming them
/// from `pool` so each owned item counts once.
fn owned_toward(id: i32, catalog: &ItemCatalog, pool: &mut Vec<i32>, depth: u8) -> i32 {
    let Some(info) = catalog.get(id) else { return 0 };
    if depth > 6 {
        return 0;
    }
    info.from
        .iter()
        .map(|c| {
            if let Some(pos) = pool.iter().position(|x| x == c) {
                pool.swap_remove(pos);
                catalog.get(*c).map_or(0, |i| i.total_gold)
            } else {
                owned_toward(*c, catalog, pool, depth + 1)
            }
        })
        .sum()
}

pub fn build_progress(
    plan: &[PlannedItem],
    catalog: &ItemCatalog,
    owned: &[LiveItem],
    current_gold: f64,
) -> Option<BuildProgress> {
    if plan.is_empty() {
        return None;
    }
    let mut pool: Vec<i32> =
        owned.iter().flat_map(|i| std::iter::repeat_n(i.item_id, i.count.max(1) as usize)).collect();
    let mut slots = Vec::with_capacity(plan.len());
    for p in plan {
        let owned = if let Some(pos) = pool.iter().position(|x| *x == p.item_id) {
            pool.swap_remove(pos);
            true
        } else {
            false
        };
        slots.push(PlanSlot { item_id: p.item_id, name: p.name.clone(), owned });
    }
    let next = plan.iter().zip(&slots).find(|(_, s)| !s.owned).map(|(p, _)| {
        let components_gold = owned_toward(p.item_id, catalog, &mut pool.clone(), 0).min(p.total_gold);
        let have = f64::from(components_gold) + current_gold.max(0.0);
        let total = f64::from(p.total_gold.max(1));
        NextItem {
            item_id: p.item_id,
            name: p.name.clone(),
            total_gold: p.total_gold,
            components_gold,
            remaining_gold: (total - have).max(0.0).ceil() as i32,
            progress_pct: (have / total * 100.0).min(100.0),
        }
    });
    Some(BuildProgress { plan: slots, next })
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalLive {
    #[serde(flatten)]
    pub goal: GoalReminder,
    /// This game's value so far, when it can be read from live data.
    pub value: Option<f64>,
    /// Why there's no value (e.g. "after 10:00").
    pub pending: Option<&'static str>,
    /// This game's value meets the target (only once it's final or for
    /// counts that can only grow past it).
    pub met: Option<bool>,
}

/// Metrics a goal can track live, from the player's own scoreboard numbers.
pub const LIVE_METRICS: [&str; 8] = ["cspm", "cs_10", "deaths", "deaths_pre14", "kills", "assists", "kda", "vision_pm"];

pub fn goals_live(goals: &[GoalReminder], s: &OwnSnapshot, marks: &Marks) -> Vec<GoalLive> {
    let minutes = s.game_time_s / 60.0;
    let sc = &s.scores;
    goals
        .iter()
        .map(|g| {
            let at_most = g.comparator == "at_most";
            // (value, final) — a count can only grow, so "at most" fails for good once passed.
            let (value, is_final, pending): (Option<f64>, bool, Option<&'static str>) = match g.metric.as_str() {
                "cspm" => (Some(if minutes > 0.0 { f64::from(sc.creep_score) / minutes } else { 0.0 }), false, None),
                "cs_10" => match marks.cs_10 {
                    Some(v) => (Some(f64::from(v)), true, None),
                    None if s.game_time_s < 600.0 => (Some(f64::from(sc.creep_score)), false, None),
                    None => (None, false, Some("missed 10:00")),
                },
                "deaths" => (Some(f64::from(sc.deaths)), false, None),
                "deaths_pre14" => match marks.deaths_14 {
                    Some(v) => (Some(f64::from(v)), true, None),
                    None if s.game_time_s < 840.0 => (Some(f64::from(sc.deaths)), false, None),
                    None => (None, false, Some("missed 14:00")),
                },
                "kills" => (Some(f64::from(sc.kills)), false, None),
                "assists" => (Some(f64::from(sc.assists)), false, None),
                "kda" => (Some(f64::from(sc.kills + sc.assists) / f64::from(sc.deaths.max(1))), false, None),
                "vision_pm" => (Some(if minutes > 0.0 { sc.ward_score / minutes } else { 0.0 }), false, None),
                _ => (None, false, Some("shown after the game")),
            };
            let counts_up = matches!(g.metric.as_str(), "deaths" | "deaths_pre14");
            let met = value.and_then(|v| {
                let ok = if at_most { v <= g.target + 1e-9 } else { v >= g.target - 1e-9 };
                if is_final || (counts_up && at_most && !ok) { Some(ok) } else { None }
            });
            GoalLive { goal: g.clone(), value, pending, met }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rc_metrics::model::ItemInfo;

    use super::*;
    use crate::model::Scores;

    fn item(name: &str, gold: i32, from: &[i32]) -> ItemInfo {
        ItemInfo { name: name.into(), total_gold: gold, completed: gold >= 2200, plannable: true, from: from.to_vec() }
    }

    fn catalog() -> ItemCatalog {
        ItemCatalog::from_items([
            (1052, item("Amplifying Tome", 400, &[])),
            (3802, item("Lost Chapter", 1200, &[1052, 1027])),
            (1027, item("Sapphire Crystal", 300, &[])),
            (6655, item("Luden's Companion", 2850, &[3802, 1052])),
            (1001, item("Boots", 300, &[])),
            (3020, item("Sorcerer's Shoes", 1100, &[1001])),
        ])
    }

    fn plan(ids: &[i32]) -> Vec<PlannedItem> {
        let c = catalog();
        ids.iter()
            .map(|id| {
                let i = c.get(*id).expect("item");
                PlannedItem { item_id: *id, name: i.name.clone(), total_gold: i.total_gold }
            })
            .collect()
    }

    fn owned(ids: &[i32]) -> Vec<LiveItem> {
        ids.iter().map(|id| LiveItem { item_id: *id, count: 1, slot: 0 }).collect()
    }

    #[test]
    fn next_item_counts_components_in_the_recipe_tree() {
        let c = catalog();
        // Own a tome (direct component) and a sapphire (inside Lost Chapter).
        let b = build_progress(&plan(&[6655, 3020]), &c, &owned(&[1052, 1027]), 1000.0).expect("some");
        let next = b.next.expect("next");
        assert_eq!(next.item_id, 6655);
        assert_eq!(next.components_gold, 700);
        assert_eq!(next.remaining_gold, 2850 - 700 - 1000);
        assert!(!b.plan[0].owned);

        // Luden's owned: boots are next; owned basic boots count.
        let b = build_progress(&plan(&[6655, 3020]), &c, &owned(&[6655, 1001]), 900.0).expect("some");
        assert!(b.plan[0].owned);
        let next = b.next.expect("next");
        assert_eq!((next.item_id, next.components_gold, next.remaining_gold), (3020, 300, 0));
        assert_eq!(next.progress_pct, 100.0);

        // Everything owned.
        let b = build_progress(&plan(&[6655]), &c, &owned(&[6655]), 0.0).expect("some");
        assert!(b.next.is_none());
        assert!(build_progress(&[], &c, &[], 0.0).is_none());
    }

    #[test]
    fn pacing_starts_after_five_minutes() {
        let ctx = LiveContext {
            cs_target: Some(CsTarget { per_min: 7.0, source: TargetSource::Setting }),
            ..Default::default()
        };
        let mut s = OwnSnapshot {
            game_time_s: 240.0,
            scores: Scores { creep_score: 20, ..Default::default() },
            ..Default::default()
        };
        assert!(pacing(&s, &ctx).expected_cs.is_none());
        s.game_time_s = 600.0;
        s.scores.creep_score = 60;
        let p = pacing(&s, &ctx);
        assert_eq!(p.expected_cs, Some(70.0));
        assert_eq!(p.diff, Some(-10.0));
        assert!((p.cs_per_min - 6.0).abs() < 1e-9);
    }

    #[test]
    fn goal_values_and_marks() {
        let goal = |metric: &str, cmp: &str, target: f64| GoalReminder {
            goal_id: 1,
            title: String::new(),
            metric: metric.into(),
            label: String::new(),
            comparator: cmp.into(),
            target,
        };
        let goals = [
            goal("deaths_pre14", "at_most", 1.0),
            goal("cs_10", "at_least", 70.0),
            goal("gold_diff_15", "at_least", 0.0),
        ];
        let mut marks = Marks::default();
        let mut s = OwnSnapshot {
            game_time_s: 300.0,
            scores: Scores { deaths: 1, creep_score: 30, ..Default::default() },
            ..Default::default()
        };
        marks.update(&s);
        let v = goals_live(&goals, &s, &marks);
        assert_eq!((v[0].value, v[0].met), (Some(1.0), None));
        assert_eq!(v[2].pending, Some("shown after the game"));

        s.game_time_s = 602.0;
        s.scores.creep_score = 72;
        s.scores.deaths = 2;
        marks.update(&s);
        let v = goals_live(&goals, &s, &marks);
        assert_eq!(v[0].met, Some(false), "a third death can't be undone");
        assert_eq!((v[1].value, v[1].met), (Some(72.0), Some(true)));

        s.game_time_s = 900.0;
        s.scores.deaths = 4;
        s.scores.creep_score = 120;
        marks.update(&s);
        assert_eq!(marks.deaths_14, Some(4), "first poll past 14:00");
        assert_eq!(marks.cs_10, Some(72), "frozen at 10:00");

        // Poller started late: no mark.
        let mut late = Marks::default();
        late.update(&s);
        assert_eq!(late.cs_10, None);
        assert_eq!(goals_live(&goals, &s, &late)[1].pending, Some("missed 10:00"));
    }
}
