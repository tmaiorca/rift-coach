//! Detectors that compare across games rather than testing each game:
//! consistent percentile weaknesses/strengths, where you die, and how your
//! results change within a play session.

use std::collections::{BTreeMap, BTreeSet};

use rc_metrics::defs::{METRICS, format_value, lower_label, role_label, role_name};
use rc_metrics::percentile::{compare, median};
use rc_metrics::zones::Zone;

use crate::sessions::session_positions;
use crate::{
    Comparator, Evidence, Game, GoalSuggestion, MAX_EVIDENCE, Polarity, Signal, pct, score, severity, win_gap,
    win_rates, win_sentence,
};

/// Games per role before percentile patterns are judged.
pub const MIN_ROLE_GAMES: usize = 8;
/// Population values needed for a percentile.
pub const MIN_POPULATION: usize = 20;
/// Median percentile at or below which a metric is a weakness (and at or
/// above 100 minus this, a strength).
pub const WEAK_PCT: f64 = 30.0;

/// Outcome-driven or already covered by a rule detector.
const PERCENTILE_SKIP: [&str; 10] = [
    "kills",
    "assists",
    "kda",
    "deaths",
    "deaths_pre14",
    "solo_deaths",
    "deaths_ahead",
    "deaths_before_obj",
    "shutdown_given",
    "unspent_gold_death",
];

/// Metrics where you're consistently in the bottom (or top) of players in
/// your role in your games.
pub fn percentile_signals(games: &[Game], ctx: &crate::Context, covered: &[&str]) -> Vec<Signal> {
    let mut by_role: BTreeMap<&str, Vec<&Game>> = BTreeMap::new();
    for g in games {
        if let Some(r) = g.role.as_deref() {
            by_role.entry(r).or_default().push(g);
        }
    }
    let mut out = Vec::new();
    for (role, mut games) in by_role {
        if games.len() < MIN_ROLE_GAMES {
            continue;
        }
        games.sort_by_key(|g| std::cmp::Reverse(g.start_ms));
        let Some(pop) = ctx.role_populations.get(role) else { continue };
        let rl = role_label(role);
        for d in METRICS {
            let Some(higher) = d.higher_is_better else { continue };
            if d.zero_sum || PERCENTILE_SKIP.contains(&d.key) || covered.contains(&d.key) {
                continue;
            }
            let Some(pop_values) = pop.get(d.key).filter(|v| v.len() >= MIN_POPULATION) else { continue };
            let scored: Vec<(&Game, f64, f64)> = games
                .iter()
                .filter_map(|g| {
                    let v = g.metric(d.key)?;
                    Some((*g, v, compare(v, pop_values, Some(higher))?.better_than_pct))
                })
                .collect();
            if scored.len() < MIN_ROLE_GAMES {
                continue;
            }
            let pcts: Vec<f64> = scored.iter().map(|s| s.2).collect();
            let med_pct = median(&pcts).round();
            let polarity = if med_pct <= WEAK_PCT {
                Polarity::Weakness
            } else if med_pct >= 100.0 - WEAK_PCT {
                Polarity::Strength
            } else {
                continue;
            };
            let affected_of = |p: f64| match polarity {
                Polarity::Weakness => p < 40.0,
                Polarity::Strength => p > 60.0,
            };
            let (hit, rest): (Vec<&(&Game, f64, f64)>, Vec<_>) = scored.iter().partition(|s| affected_of(s.2));
            let wr = win_rates(hit.iter().map(|s| s.0), rest.iter().map(|s| s.0));
            let gap = win_gap(polarity, wr);
            let n = scored.len();
            let yours = median(&scored.iter().map(|s| s.1).collect::<Vec<_>>());
            let theirs = median(pop_values);
            let strength = (50.0 - med_pct).abs() / 50.0;
            let (title, verb) = match polarity {
                Polarity::Weakness => (format!("{} below most {rl}", d.label), "only"),
                Polarity::Strength => (format!("{} ahead of most {rl}", d.label), ""),
            };
            let detail = format!(
                "In {n} {role_games} games, your typical {label} ({yours}) beat {verb}{sp}{med_pct}% of {rl} in \
                 your games (their median {theirs}).{wins}",
                role_games = role_name(role),
                label = lower_label(d.label),
                yours = format_value(d, yours),
                sp = if verb.is_empty() { "" } else { " " },
                theirs = format_value(d, theirs),
                wins = win_sentence(wr),
            );
            out.push(Signal {
                id: format!("{}:{}:{role}", if polarity == Polarity::Weakness { "weak" } else { "strong" }, d.key),
                kind: if polarity == Polarity::Weakness { "percentile_weakness" } else { "percentile_strength" },
                polarity,
                severity: severity(pct(hit.len(), n), gap),
                score: score(strength, hit.len(), gap),
                title,
                detail,
                tip: d.description,
                games_considered: n,
                games_affected: hit.len(),
                rate_pct: pct(hit.len(), n).round(),
                metric: Some(d.key),
                win_rate_affected: wr.0,
                win_rate_other: wr.1,
                evidence: hit
                    .iter()
                    .take(MAX_EVIDENCE)
                    .map(|s| Evidence {
                        match_id: s.0.match_id.clone(),
                        t_s: None,
                        x: None,
                        y: None,
                        note: format!("{} (better than {}%)", format_value(d, s.1), s.2.round()),
                    })
                    .collect(),
                goal: (polarity == Polarity::Weakness).then(|| GoalSuggestion {
                    metric: d.key,
                    comparator: Comparator::for_direction(higher),
                    target: round_to(theirs, d.decimals),
                }),
            });
        }
    }
    out
}

fn round_to(v: f64, decimals: u8) -> f64 {
    let f = 10f64.powi(i32::from(decimals));
    (v * f).round() / f
}

/// Deaths needed in a zone (and games they span) before it's a hot spot.
pub const MIN_ZONE_DEATHS: usize = 4;
pub const MIN_ZONE_GAMES: usize = 3;
/// Role population deaths needed for an expected share.
pub const MIN_ZONE_POPULATION: u32 = 50;

/// Where you die more often than players in your role do.
pub fn death_zones(games: &[Game], ctx: &crate::Context) -> Vec<Signal> {
    let mut newest: Vec<&Game> = games.iter().collect();
    newest.sort_by_key(|g| std::cmp::Reverse(g.start_ms));
    // Expected share per zone, weighted by the role of the game each death was in.
    let mut expected: BTreeMap<Zone, f64> = BTreeMap::new();
    let mut counted = 0usize;
    let mut mine: BTreeMap<Zone, Vec<(&Game, &crate::Death)>> = BTreeMap::new();
    let mut roles: BTreeSet<&str> = BTreeSet::new();
    for g in &newest {
        let Some(role) = g.role.as_deref() else { continue };
        let Some(pop) = ctx.role_death_zones.get(role) else { continue };
        let total: u32 = pop.values().sum();
        if total < MIN_ZONE_POPULATION {
            continue;
        }
        roles.insert(role);
        for d in &g.deaths {
            counted += 1;
            for z in Zone::ALL {
                *expected.entry(z).or_default() += f64::from(pop.get(&z).copied().unwrap_or(0)) / f64::from(total);
            }
            mine.entry(d.zone).or_default().push((g, d));
        }
    }
    if counted == 0 {
        return vec![];
    }
    let who = match roles.iter().next() {
        Some(r) if roles.len() == 1 => role_label(r),
        _ => "players in your roles",
    };
    let games_with_deaths = newest.iter().filter(|g| !g.deaths.is_empty() && g.role.is_some()).count();
    let mut out: Vec<Signal> = mine
        .into_iter()
        .filter_map(|(zone, deaths)| {
            let share = pct(deaths.len(), counted);
            let exp = expected.get(&zone).copied().unwrap_or(0.0) / counted as f64 * 100.0;
            let games: BTreeSet<&str> = deaths.iter().map(|(g, _)| g.match_id.as_str()).collect();
            let hot = deaths.len() >= MIN_ZONE_DEATHS
                && games.len() >= MIN_ZONE_GAMES
                && share >= 20.0
                && share - exp >= 10.0;
            if !hot {
                return None;
            }
            let affected: Vec<&Game> = newest.iter().copied().filter(|g| games.contains(g.match_id.as_str())).collect();
            let wr = win_rates(
                affected.iter().copied(),
                newest.iter().copied().filter(|g| !games.contains(g.match_id.as_str()) && g.role.is_some()),
            );
            let gap = win_gap(Polarity::Weakness, wr);
            let (share_r, exp_r) = (share.round(), exp.round());
            Some(Signal {
                id: format!("death_zone:{}", zone.as_str()),
                kind: "death_zone",
                polarity: Polarity::Weakness,
                severity: severity(share * 2.0, gap),
                score: score(((share - exp) / 50.0).min(1.0), games.len(), gap),
                title: format!("Dying in {}", zone.label()),
                detail: format!(
                    "{share_r}% of your deaths ({} in {} games) were in {} (approx.); for {who} in your games it's \
                     {exp_r}%.{}",
                    deaths.len(),
                    games.len(),
                    zone.label(),
                    win_sentence(wr)
                ),
                tip: zone_tip(zone),
                games_considered: games_with_deaths,
                games_affected: games.len(),
                rate_pct: share_r,
                metric: matches!(zone, Zone::EnemyJungle | Zone::EnemyBase).then_some("deaths_enemy_side"),
                win_rate_affected: wr.0,
                win_rate_other: wr.1,
                evidence: deaths
                    .iter()
                    .take(MAX_EVIDENCE)
                    .map(|(g, d)| Evidence {
                        match_id: g.match_id.clone(),
                        t_s: Some(d.t_s),
                        x: Some(d.x),
                        y: Some(d.y),
                        note: d
                            .killer_champion
                            .as_ref()
                            .map_or_else(|| "killed by tower/minions".to_owned(), |k| format!("killed by {k}")),
                    })
                    .collect(),
                goal: matches!(zone, Zone::EnemyJungle | Zone::EnemyBase).then_some(GoalSuggestion {
                    metric: "deaths_enemy_side",
                    comparator: Comparator::AtMost,
                    target: 1.0,
                }),
            })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(2);
    out
}

fn zone_tip(zone: Zone) -> &'static str {
    match zone {
        Zone::EnemyJungle => "Only walk into the enemy jungle with vision or when the enemy is shown elsewhere.",
        Zone::EnemyBase => "Dives and base fights need a clear numbers advantage; back off when the wave is gone.",
        Zone::TopRiver | Zone::BotRiver => {
            "River deaths are often face-checks: ward or sweep before walking through, especially near objectives."
        }
        Zone::TopLane | Zone::BotLane => {
            "Side-lane deaths usually come from pushing without vision; know where the enemy is before pushing past the river."
        }
        Zone::MidLane => "Mid is exposed from both sides; respect the enemy jungler's possible paths when you push.",
        Zone::OwnJungle => {
            "Deaths in your own jungle suggest invades or collapses; move with your jungler or ward entrances."
        }
        Zone::OwnBase => "Deaths in your base often mean defending outnumbered; wait for the team or clear from range.",
    }
}

/// Session games needed on each side before comparing.
pub const MIN_SESSION_GAMES: usize = 5;

/// Results right after a loss, and late in long sessions.
pub fn session_effects(games: &[Game], gap_ms: i64) -> Vec<Signal> {
    let mut sorted: Vec<&Game> = games.iter().collect();
    sorted.sort_by_key(|g| g.start_ms);
    let pos = session_positions(&sorted.iter().map(|g| (g.start_ms, g.end_ms())).collect::<Vec<_>>(), gap_ms);
    let after_loss: Vec<bool> = (0..sorted.len()).map(|i| i > 0 && pos[i] > 0 && !sorted[i - 1].win).collect();
    let late: Vec<bool> = pos.iter().map(|p| *p >= 3).collect();
    let mut out = Vec::new();
    for (kind, flags, title, phrase, tip) in [
        (
            "after_loss",
            &after_loss,
            "Losses snowball within a session",
            "games queued right after a loss (same session)",
            "After a loss, take a few minutes away before queuing again, and review what went wrong first.",
        ),
        (
            "long_sessions",
            &late,
            "Results drop late in a session",
            "games from the 4th game of a session onwards",
            "Set a session limit (e.g. 3 games) or stop after two losses in a row.",
        ),
    ] {
        let hit: Vec<&Game> = sorted.iter().zip(flags.iter()).filter(|(_, f)| **f).map(|(g, _)| *g).collect();
        let rest: Vec<&Game> = sorted.iter().zip(flags.iter()).filter(|(_, f)| !**f).map(|(g, _)| *g).collect();
        if hit.len() < MIN_SESSION_GAMES || rest.len() < MIN_SESSION_GAMES {
            continue;
        }
        let wr = win_rates(hit.iter().copied(), rest.iter().copied());
        let (Some(a), Some(o)) = wr else { continue };
        if o - a < 15.0 {
            continue;
        }
        let gap = Some(o - a);
        out.push(Signal {
            id: kind.to_owned(),
            kind,
            polarity: Polarity::Weakness,
            severity: severity(o - a + 20.0, None),
            score: score(((o - a) / 50.0).min(1.0), hit.len(), gap),
            title: title.to_owned(),
            detail: format!("You won {a}% of {} {phrase}, vs {o}% of your other games.", hit.len()),
            tip,
            games_considered: sorted.len(),
            games_affected: hit.len(),
            rate_pct: pct(hit.len(), sorted.len()).round(),
            metric: None,
            win_rate_affected: Some(a),
            win_rate_other: Some(o),
            evidence: hit
                .iter()
                .rev()
                .take(MAX_EVIDENCE)
                .map(|g| Evidence {
                    match_id: g.match_id.clone(),
                    t_s: None,
                    x: None,
                    y: None,
                    note: if g.win { "win".into() } else { "loss".into() },
                })
                .collect(),
            goal: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::Context;
    use crate::test_support::{death, game};

    #[test]
    fn percentile_weakness_against_role_population() {
        // Population of other mids: CS@10 from 60 to 79.
        let pop: Vec<f64> = (60..80).map(f64::from).collect();
        let ctx = Context {
            role_populations: HashMap::from([("MIDDLE".to_string(), HashMap::from([("cs_10".to_string(), pop)]))]),
            ..Default::default()
        };
        let games: Vec<Game> = (0..10).map(|i| game(i, i % 2 == 0, &[("cs_10", 55.0 + i as f64 % 3.0)])).collect();
        let s = percentile_signals(&games, &ctx, &[]);
        assert_eq!(s.len(), 1);
        let s = &s[0];
        assert_eq!(s.id, "weak:cs_10:MIDDLE");
        assert_eq!(s.games_affected, 10);
        assert!(s.detail.contains("beat only 0% of mid laners"), "{}", s.detail);
        assert!(s.detail.contains("their median 70"), "{}", s.detail);
        let goal = s.goal.as_ref().expect("goal");
        assert_eq!((goal.metric, goal.comparator, goal.target), ("cs_10", Comparator::AtLeast, 70.0));
        // Covered by a rule → skipped.
        assert!(percentile_signals(&games, &ctx, &["cs_10"]).is_empty());
    }

    #[test]
    fn death_zone_hot_spot() {
        // Mid laners in your games die 10% of the time in the enemy jungle.
        let pop = BTreeMap::from([(Zone::MidLane, 90u32), (Zone::EnemyJungle, 10)]);
        let ctx = Context { role_death_zones: HashMap::from([("MIDDLE".to_string(), pop)]), ..Default::default() };
        let games: Vec<Game> = (0..5)
            .map(|i| {
                let mut g = game(i, false, &[]);
                g.deaths = vec![death(600.0, Zone::EnemyJungle), death(900.0, Zone::MidLane)];
                g
            })
            .collect();
        let s = death_zones(&games, &ctx);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].id, "death_zone:enemy_jungle");
        assert_eq!(s[0].rate_pct, 50.0);
        assert!(s[0].detail.starts_with("50% of your deaths (5 in 5 games) were in the enemy jungle"));
        assert!(s[0].detail.contains("for mid laners in your games it's 10%"));
    }

    #[test]
    fn tilt_after_losses() {
        // 8 sessions of 3 games, 10 minutes between games: win, loss, loss.
        // Only the 3rd game follows a loss in the same session.
        let mut games = Vec::new();
        for s in 0..8 {
            for k in 0..3 {
                let mut g = game(s * 3 + k, k == 0, &[]);
                g.start_ms = s as i64 * 86_400_000 + k as i64 * 2_400_000;
                games.push(g);
            }
        }
        let s = session_effects(&games, 3_600_000);
        let tilt = s.iter().find(|s| s.kind == "after_loss").expect("tilt signal");
        // 8 games after a loss, none won; the other 16 split 8-8.
        assert_eq!(tilt.games_affected, 8);
        assert_eq!((tilt.win_rate_affected, tilt.win_rate_other), (Some(0.0), Some(50.0)));
        assert_eq!(
            tilt.detail,
            "You won 0% of 8 games queued right after a loss (same session), vs 50% of your other games."
        );
        // Sessions are 3 games long: no "late in session" games.
        assert!(s.iter().all(|s| s.kind != "long_sessions"));
        // A 5-minute gap splits every game into its own session.
        assert!(session_effects(&games, 300_000).is_empty());
    }
}
