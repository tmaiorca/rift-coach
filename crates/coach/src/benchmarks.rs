//! Higher-rank benchmarks: what to sample (your main roles and champions),
//! which patch to compare against, collection progress, and how your recent
//! numbers sit in the benchmark distribution ("gaps").

use std::collections::HashMap;

use rc_metrics::defs::{self, METRICS, Unit};
use rc_metrics::percentile::{compare, median};
use rc_store::bench::{self, patch_order};
use rc_store::metrics::{GameFilter, Population, game_metrics};
use rc_store::rusqlite::Connection;
use rc_store::settings::AppSettings;
use serde::Serialize;

/// Samples wanted per focus role on the current patch.
pub const ROLE_TARGET: i64 = 300;
/// Samples wanted per focus champion on the current patch.
pub const CHAMPION_TARGET: i64 = 50;
/// A role benchmark needs this many players before it's shown.
pub const MIN_ROLE_SAMPLE: usize = 30;
/// A champion benchmark needs this many.
pub const MIN_CHAMPION_SAMPLE: usize = 15;
/// Recent games used to pick focus roles and champions.
const FOCUS_GAMES: u32 = 50;
const MAX_FOCUS_CHAMPIONS: usize = 3;

pub fn tier_label(tier: &str) -> &'static str {
    match tier {
        "EMERALD" => "Emerald",
        "DIAMOND" => "Diamond",
        "MASTER" => "Master+",
        _ => "higher-rank",
    }
}

/// Roles and champions to sample first.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Focus {
    pub roles: Vec<String>,
    /// Data Dragon ids (= Match-V5 `championName`).
    pub champions: Vec<String>,
}

pub fn focus(conn: &Connection, account_id: Option<i64>, settings: &AppSettings) -> rc_store::Result<Focus> {
    let games = match account_id {
        Some(a) => game_metrics(conn, a, &GameFilter { limit: FOCUS_GAMES, ..Default::default() })?,
        None => Vec::new(),
    };
    let mut roles: HashMap<String, usize> = HashMap::new();
    let mut champs: HashMap<String, usize> = HashMap::new();
    for g in &games {
        if let Some(r) = &g.team_position {
            *roles.entry(r.clone()).or_default() += 1;
        }
        *champs.entry(g.champion_name.clone()).or_default() += 1;
    }
    let mut roles: Vec<(String, usize)> = roles.into_iter().collect();
    roles.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let total = games.len().max(1);
    let roles: Vec<String> = roles
        .iter()
        .enumerate()
        .filter(|(i, (_, n))| *i == 0 || *n * 100 / total >= 30)
        .map(|(_, (r, _))| r.clone())
        .take(2)
        .collect();
    let champions = if settings.benchmarks.champions.is_empty() {
        let mut c: Vec<(String, usize)> = champs.into_iter().filter(|(_, n)| *n >= 5).collect();
        c.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        c.into_iter().take(MAX_FOCUS_CHAMPIONS).map(|(n, _)| n).collect()
    } else {
        settings.benchmarks.champions.iter().take(MAX_FOCUS_CHAMPIONS).cloned().collect()
    };
    Ok(Focus { roles, champions })
}

/// Numeric champion ids for Data Dragon ids, from stored games.
pub fn champion_ids(conn: &Connection, names: &[String]) -> rc_store::Result<Vec<i32>> {
    let mut out = Vec::new();
    for n in names {
        let id: Option<i32> =
            conn.query_row("SELECT champion_id FROM match_participants WHERE champion_name = ?1 LIMIT 1", [n], |r| {
                r.get(0)
            })?;
        out.extend(id);
    }
    Ok(out)
}

/// The patch to compare a game against: its own patch if the role has
/// enough benchmark samples, else the newest patch that does.
pub fn pick_patch(
    conn: &Connection,
    tier: &str,
    preferred: Option<&str>,
    role: &str,
) -> rc_store::Result<Option<String>> {
    let enough = |p: &str| -> rc_store::Result<bool> {
        Ok(bench::sample_count(conn, tier, p, role, None)? >= MIN_ROLE_SAMPLE as i64)
    };
    if let Some(p) = preferred
        && enough(p)?
    {
        return Ok(Some(p.to_owned()));
    }
    for (p, _) in bench::patches(conn, tier)? {
        if enough(&p)? {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

/// A benchmark population with its label, e.g. "Diamond mid laners, patch 16.20".
#[derive(Debug, Clone)]
pub struct BenchPop {
    pub label: String,
    pub patch: String,
    pub pop: Population,
}

/// Role and champion benchmark populations for a role (and champion).
pub fn populations(
    conn: &Connection,
    tier: &str,
    patch: Option<&str>,
    role: &str,
    champion: Option<&str>,
) -> rc_store::Result<(Option<BenchPop>, Option<BenchPop>)> {
    let Some(patch) = pick_patch(conn, tier, patch, role)? else { return Ok((None, None)) };
    let t = tier_label(tier);
    let role_pop = BenchPop {
        label: format!("{t} {}, patch {patch}", defs::role_label(role)),
        pop: bench::population(conn, tier, &patch, role, None)?,
        patch: patch.clone(),
    };
    let champ_pop = match champion {
        Some(c) if bench::sample_count(conn, tier, &patch, role, Some(c))? >= MIN_CHAMPION_SAMPLE as i64 => {
            Some(BenchPop {
                label: format!("{t} {c} {}, patch {patch}", defs::role_name(role)),
                pop: bench::population(conn, tier, &patch, role, Some(c))?,
                patch: patch.clone(),
            })
        }
        _ => None,
    };
    Ok((Some(role_pop), champ_pop))
}

/// The newest patch in your own games (what benchmarks should cover).
pub fn current_patch(conn: &Connection) -> rc_store::Result<Option<String>> {
    let mut st = conn.prepare("SELECT DISTINCT patch FROM matches WHERE benchmark_tier IS NULL AND patch != ''")?;
    let ps: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rc_store::rusqlite::Result<_>>()?;
    Ok(ps.into_iter().max_by_key(|p| patch_order(p)))
}

#[derive(Debug, Clone, Serialize)]
pub struct SampleProgress {
    pub role: String,
    pub champion: Option<String>,
    pub label: String,
    pub samples: i64,
    pub target: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub enabled: bool,
    pub tier: String,
    pub tier_label: &'static str,
    pub patch: Option<String>,
    pub focus: Focus,
    pub targets: Vec<SampleProgress>,
    /// Benchmark games per patch, newest first.
    pub games_by_patch: Vec<(String, i64)>,
    pub collector: bench::CollectorCounts,
}

impl Progress {
    pub fn complete(&self) -> bool {
        !self.targets.is_empty() && self.targets.iter().all(|t| t.samples >= t.target)
    }

    /// Roles (and champions) still short of their target.
    pub fn needs_role(&self) -> bool {
        self.targets.iter().any(|t| t.champion.is_none() && t.samples < t.target)
    }

    pub fn needs_champion(&self) -> bool {
        self.targets.iter().any(|t| t.champion.is_some() && t.samples < t.target)
    }
}

pub fn progress(conn: &Connection, account_id: Option<i64>, settings: &AppSettings) -> rc_store::Result<Progress> {
    let tier = settings.benchmarks.tier.clone();
    let focus = focus(conn, account_id, settings)?;
    let patch = current_patch(conn)?;
    let key_gen = rc_store::accounts::latest_key_gen(conn)?;
    let mut targets = Vec::new();
    if let Some(p) = &patch {
        for role in &focus.roles {
            targets.push(SampleProgress {
                role: role.clone(),
                champion: None,
                label: defs::role_label(role).to_owned(),
                samples: bench::sample_count(conn, &tier, p, role, None)?,
                target: ROLE_TARGET,
            });
        }
        if let Some(main) = focus.roles.first() {
            for c in &focus.champions {
                targets.push(SampleProgress {
                    role: main.clone(),
                    champion: Some(c.clone()),
                    label: format!("{c} {}", defs::role_name(main)),
                    samples: bench::sample_count(conn, &tier, p, main, Some(c))?,
                    target: CHAMPION_TARGET,
                });
            }
        }
    }
    Ok(Progress {
        enabled: settings.benchmarks.enabled,
        tier_label: tier_label(&tier),
        games_by_patch: bench::patches(conn, &tier)?,
        collector: match key_gen {
            Some(k) => bench::collector_counts(conn, k, &tier)?,
            None => Default::default(),
        },
        tier,
        patch,
        focus,
        targets,
    })
}

/// Your typical value for a metric against the benchmark distribution.
#[derive(Debug, Clone, Serialize)]
pub struct Gap {
    pub metric: &'static str,
    pub label: &'static str,
    pub unit: Unit,
    pub decimals: u8,
    pub higher_is_better: bool,
    /// Median over your recent games.
    pub yours: f64,
    pub benchmark_median: f64,
    /// Where your median sits among benchmark players, 0-100 (higher = better).
    pub better_than_pct: f64,
    pub n: usize,
    pub early: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GapsView {
    pub reference: Option<String>,
    pub role: String,
    pub champion: Option<String>,
    pub games: usize,
    /// Worst first.
    pub gaps: Vec<Gap>,
}

/// Metrics measured by 15:00 (actionable in lane).
fn is_early(key: &str) -> bool {
    key.ends_with("_10") || key.ends_with("_15") || matches!(key, "deaths_pre14" | "first_back_min" | "first_back_gold")
}

/// Compares the medians of your games matching `filter` in `role`
/// (optionally on `champion`) with the benchmark tier.
pub fn gaps(
    conn: &Connection,
    account_id: i64,
    settings: &AppSettings,
    role: &str,
    champion: Option<&str>,
    filter: GameFilter,
) -> rc_store::Result<GapsView> {
    let mine = game_metrics(
        conn,
        account_id,
        &GameFilter { role: Some(role.to_owned()), champion_name: champion.map(str::to_owned), ..filter },
    )?;
    let tier = &settings.benchmarks.tier;
    let patch = current_patch(conn)?;
    let (role_pop, champ_pop) = populations(conn, tier, patch.as_deref(), role, champion)?;
    let reference = champ_pop.as_ref().or(role_pop.as_ref());
    let mut out = Vec::new();
    if let Some(r) = reference {
        for d in METRICS.iter().filter(|d| !d.zero_sum) {
            let Some(higher) = d.higher_is_better else { continue };
            let values: Vec<f64> = mine.iter().filter_map(|g| g.metrics.get(d.key).copied()).collect();
            let Some(pop) = r.pop.get(d.key).filter(|p| p.len() >= MIN_CHAMPION_SAMPLE) else { continue };
            if values.len() < 3 {
                continue;
            }
            let yours = median(&values);
            let Some(c) = compare(yours, pop, Some(higher)) else { continue };
            out.push(Gap {
                metric: d.key,
                label: d.label,
                unit: d.unit,
                decimals: d.decimals,
                higher_is_better: higher,
                yours,
                benchmark_median: c.median,
                better_than_pct: c.better_than_pct,
                n: c.n,
                early: is_early(d.key),
            });
        }
    }
    out.sort_by(|a, b| a.better_than_pct.total_cmp(&b.better_than_pct));
    Ok(GapsView {
        reference: reference.map(|r| r.label.clone()),
        role: role.to_owned(),
        champion: champ_pop.as_ref().and(champion.map(str::to_owned)),
        games: mine.len(),
        gaps: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_metrics() {
        assert!(is_early("cs_10") && is_early("gold_diff_15") && is_early("deaths_pre14"));
        assert!(!is_early("kp") && !is_early("dpm"));
        assert_eq!(tier_label("MASTER"), "Master+");
    }
}
