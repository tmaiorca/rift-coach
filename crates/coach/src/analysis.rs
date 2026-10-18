//! Compares the player's metrics in one game with two populations:
//! - **role players**: everyone else who played the same role in your stored
//!   Summoner's Rift games (your lobbies, so roughly your skill bracket);
//! - **your games**: your own previous games in that role (the baseline,
//!   limited to the "look back" window).

use rc_metrics::defs::{Group, METRICS, Unit};
use rc_metrics::percentile::{Comparison, compare};
use rc_store::metrics as store_metrics;
use rc_store::rusqlite::Connection;
use serde::Serialize;

/// A champion-specific comparison needs at least this many samples.
pub const MIN_CHAMPION_SAMPLE: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct MetricView {
    pub key: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub unit: Unit,
    pub decimals: u8,
    pub higher_is_better: Option<bool>,
    pub approximate: bool,
    /// Compared only with the player's own games (see `MetricDef::zero_sum`).
    pub zero_sum: bool,
    pub description: &'static str,
    pub value: f64,
    /// vs other players in this role in your stored games.
    pub vs_role: Option<Comparison>,
    /// vs other players on this champion in this role (if enough samples).
    pub vs_champion: Option<Comparison>,
    /// vs your own previous games in this role.
    pub vs_personal: Option<Comparison>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Populations {
    pub role: Option<String>,
    pub champion_name: String,
    pub role_label: String,
    pub champion_label: String,
    pub personal_label: String,
    pub baseline_games: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricComparison {
    pub populations: Populations,
    pub metrics: Vec<MetricView>,
}

pub use rc_metrics::defs::role_label;

/// `role` is the player's `teamPosition` (None in non-Rift modes, where only
/// the raw values are returned).
#[allow(clippy::too_many_arguments)]
pub fn compare_metrics(
    conn: &Connection,
    account_id: i64,
    match_id: &str,
    participant_id: i32,
    role: Option<&str>,
    champion_name: &str,
    game_start_ms: i64,
    baseline_games: u32,
) -> rc_store::Result<MetricComparison> {
    let mine = store_metrics::participant_metrics(conn, match_id, participant_id)?;
    let (role_pop, champ_pop, personal) = match role {
        Some(r) => (
            store_metrics::role_population(conn, account_id, r, None)?,
            store_metrics::role_population(conn, account_id, r, Some(champion_name))?,
            store_metrics::personal_population(conn, account_id, r, None, baseline_games, game_start_ms)?,
        ),
        None => Default::default(),
    };
    let metrics = METRICS
        .iter()
        .filter_map(|d| {
            let value = *mine.get(d.key)?;
            let cmp = |pop: &store_metrics::Population, min: usize| {
                pop.get(d.key).filter(|v| v.len() >= min).and_then(|v| compare(value, v, d.higher_is_better))
            };
            Some(MetricView {
                key: d.key,
                label: d.label,
                group: d.group,
                unit: d.unit,
                decimals: d.decimals,
                higher_is_better: d.higher_is_better,
                approximate: d.approximate,
                zero_sum: d.zero_sum,
                description: d.description,
                value,
                vs_role: if d.zero_sum { None } else { cmp(&role_pop, 1) },
                vs_champion: if d.zero_sum { None } else { cmp(&champ_pop, MIN_CHAMPION_SAMPLE) },
                vs_personal: cmp(&personal, 3),
            })
        })
        .collect();
    let rl = role.map_or("players", role_label);
    Ok(MetricComparison {
        populations: Populations {
            role: role.map(str::to_owned),
            champion_name: champion_name.to_owned(),
            role_label: format!("{rl} in your games"),
            champion_label: format!("{champion_name} {rl} in your games"),
            personal_label: format!("your last {baseline_games} games in this role"),
            baseline_games,
        },
        metrics,
    })
}
