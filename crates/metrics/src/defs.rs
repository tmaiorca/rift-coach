//! Metric registry: every metric the engine emits, with how to read it.
//! The UI, percentile code, and AI facts all use these definitions.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Laning,
    Macro,
    Fighting,
    Vision,
    Items,
    Overall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Count,
    Gold,
    Xp,
    Percent,
    Minutes,
    Seconds,
    PerMinute,
    Ratio,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MetricDef {
    pub key: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub unit: Unit,
    /// `Some(true)` if more is better, `Some(false)` if less is better,
    /// `None` when it depends on context (then no "better than" claim).
    pub higher_is_better: Option<bool>,
    pub decimals: u8,
    /// Derived from ~1-minute position samples or frame snapshots.
    pub approximate: bool,
    /// Measured against the lane opponent. Other players in your games are
    /// mostly your opponents, so their values mirror yours: compare these
    /// only with your own baseline.
    pub zero_sum: bool,
    pub description: &'static str,
}

const fn def(
    key: &'static str,
    label: &'static str,
    group: Group,
    unit: Unit,
    higher_is_better: Option<bool>,
    decimals: u8,
    description: &'static str,
) -> MetricDef {
    MetricDef { key, label, group, unit, higher_is_better, decimals, approximate: false, zero_sum: false, description }
}

const fn approx(d: MetricDef) -> MetricDef {
    MetricDef { approximate: true, ..d }
}

const fn vs_opponent(d: MetricDef) -> MetricDef {
    MetricDef { zero_sum: true, ..d }
}

use Group::*;
use Unit::*;

pub const METRICS: &[MetricDef] = &[
    // Laning
    def("cs_10", "CS at 10:00", Laning, Count, Some(true), 0, "Minions + monsters killed by 10:00."),
    def("cs_15", "CS at 15:00", Laning, Count, Some(true), 0, "Minions + monsters killed by 15:00."),
    def("gold_10", "Gold at 10:00", Laning, Gold, Some(true), 0, "Total gold earned by 10:00."),
    def("gold_15", "Gold at 15:00", Laning, Gold, Some(true), 0, "Total gold earned by 15:00."),
    def("xp_10", "XP at 10:00", Laning, Xp, Some(true), 0, "Experience by 10:00."),
    def("xp_15", "XP at 15:00", Laning, Xp, Some(true), 0, "Experience by 15:00."),
    vs_opponent(def(
        "cs_diff_10",
        "CS diff at 10:00",
        Laning,
        Count,
        Some(true),
        0,
        "Your CS minus your lane opponent's at 10:00.",
    )),
    vs_opponent(def(
        "cs_diff_15",
        "CS diff at 15:00",
        Laning,
        Count,
        Some(true),
        0,
        "Your CS minus your lane opponent's at 15:00.",
    )),
    vs_opponent(def(
        "gold_diff_10",
        "Gold diff at 10:00",
        Laning,
        Gold,
        Some(true),
        0,
        "Your gold minus your lane opponent's at 10:00.",
    )),
    vs_opponent(def(
        "gold_diff_15",
        "Gold diff at 15:00",
        Laning,
        Gold,
        Some(true),
        0,
        "Your gold minus your lane opponent's at 15:00.",
    )),
    vs_opponent(def(
        "xp_diff_10",
        "XP diff at 10:00",
        Laning,
        Xp,
        Some(true),
        0,
        "Your XP minus your lane opponent's at 10:00.",
    )),
    vs_opponent(def(
        "xp_diff_15",
        "XP diff at 15:00",
        Laning,
        Xp,
        Some(true),
        0,
        "Your XP minus your lane opponent's at 15:00.",
    )),
    def(
        "deaths_pre14",
        "Deaths before 14:00",
        Laning,
        Count,
        Some(false),
        0,
        "Deaths in the laning phase (before 14:00).",
    ),
    def("solo_kills", "Solo kills", Laning, Count, Some(true), 0, "Champion kills you got with no assists."),
    def("solo_deaths", "Solo deaths", Laning, Count, Some(false), 0, "Deaths to a single enemy with no assists."),
    approx(def(
        "first_back_min",
        "First back",
        Laning,
        Minutes,
        None,
        1,
        "Game time of your first shop visit after 1:30 (first purchase).",
    )),
    def(
        "first_back_gold",
        "First back gold spent",
        Laning,
        Gold,
        Some(true),
        0,
        "Gold spent on your first back (purchases minus components used).",
    ),
    // Macro
    def(
        "kp",
        "Kill participation",
        Macro,
        Percent,
        Some(true),
        0,
        "Your kills + assists as a share of your team's kills.",
    ),
    def(
        "obj_part",
        "Objective participation",
        Macro,
        Percent,
        Some(true),
        0,
        "Share of your team's dragons, grubs, heralds, barons and towers you took part in.",
    ),
    def("time_dead_pct", "Time dead", Macro, Percent, Some(false), 1, "Share of the game spent dead."),
    approx(def(
        "unspent_gold_death",
        "Unspent gold at death",
        Macro,
        Gold,
        Some(false),
        0,
        "Average gold in your pocket at the last minute snapshot before each death.",
    )),
    def(
        "shutdown_given",
        "Shutdown gold given",
        Macro,
        Gold,
        Some(false),
        0,
        "Bounty gold the enemy earned by killing you.",
    ),
    def(
        "max_team_lead",
        "Biggest team gold lead",
        Macro,
        Gold,
        None,
        0,
        "Largest gold lead your team held at any minute (negative if you never led).",
    ),
    // Fighting
    def("kda", "KDA", Fighting, Ratio, Some(true), 2, "(Kills + assists) / max(1, deaths)."),
    def("kills", "Kills", Fighting, Count, Some(true), 0, "Champion kills."),
    def("deaths", "Deaths", Fighting, Count, Some(false), 0, "Times you died."),
    def("assists", "Assists", Fighting, Count, Some(true), 0, "Champion kill assists."),
    def("dpm", "Damage per minute", Fighting, PerMinute, Some(true), 0, "Damage to champions per minute."),
    def(
        "dmg_share",
        "Damage share",
        Fighting,
        Percent,
        Some(true),
        1,
        "Your share of your team's damage to champions.",
    ),
    def("gold_share", "Gold share", Fighting, Percent, None, 1, "Your share of your team's gold."),
    def(
        "dmg_per_gold_share",
        "Damage share / gold share",
        Fighting,
        Ratio,
        Some(true),
        2,
        "Above 1 means you dealt more than your share of damage for the gold you had.",
    ),
    def("dmg_taken_share", "Damage taken share", Fighting, Percent, None, 1, "Your share of your team's damage taken."),
    // Overall economy
    def("cspm", "CS per minute", Overall, PerMinute, Some(true), 1, "Minions + monsters per minute."),
    def("gpm", "Gold per minute", Overall, PerMinute, Some(true), 0, "Gold earned per minute."),
    // Vision
    def("vision_pm", "Vision score per minute", Vision, PerMinute, Some(true), 2, "Vision score per minute."),
    def("wards_placed_pm", "Wards placed per minute", Vision, PerMinute, Some(true), 2, "Wards placed per minute."),
    def(
        "wards_killed_pm",
        "Wards cleared per minute",
        Vision,
        PerMinute,
        Some(true),
        2,
        "Enemy wards destroyed per minute.",
    ),
    def("control_wards", "Control wards placed", Vision, Count, Some(true), 0, "Control wards placed."),
    // Items
    def(
        "first_item_min",
        "First completed item",
        Items,
        Minutes,
        Some(false),
        1,
        "Game time you bought your first finished (non-boots) item.",
    ),
];

pub fn get(key: &str) -> Option<&'static MetricDef> {
    METRICS.iter().find(|d| d.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique() {
        let mut keys: Vec<_> = METRICS.iter().map(|d| d.key).collect();
        keys.sort_unstable();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }
}
