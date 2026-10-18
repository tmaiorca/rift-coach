//! Rule detectors: a per-game test ("died 2+ times before 14:00") that
//! becomes a signal when it recurs across enough of your games. The same
//! tests flag individual games on the match screen.

use serde::Serialize;

use crate::{
    Comparator, Death, Evidence, Game, GoalSuggestion, MAX_EVIDENCE, Polarity, Signal, pct, score, severity, win_gap,
    win_rates, win_sentence,
};

/// A rule needs this many considered games, and this many affected ones.
pub const MIN_CONSIDERED: usize = 5;
pub const MIN_AFFECTED: usize = 3;

pub struct Rule {
    pub kind: &'static str,
    pub title: &'static str,
    pub polarity: Polarity,
    /// Completes "<what> in 7 of 20 games".
    pub what: &'static str,
    /// Metric the rule is about (suppresses the generic percentile signal).
    pub metric: Option<&'static str>,
    /// Share of considered games (0–100) at which it becomes a signal.
    pub min_rate: f64,
    pub goal: Option<GoalSuggestion>,
    pub tip: &'static str,
    /// Whether the game counts at all (has the data, right role...).
    pub considered: fn(&Game) -> bool,
    /// Evidence if the game shows the pattern.
    pub hit: fn(&Game) -> Option<Vec<Evidence>>,
}

const fn goal(metric: &'static str, comparator: Comparator, target: f64) -> Option<GoalSuggestion> {
    Some(GoalSuggestion { metric, comparator, target })
}

fn has_timeline(g: &Game) -> bool {
    g.metrics.contains_key("deaths_pre14")
}

fn laner(g: &Game) -> bool {
    g.role.as_deref() != Some("UTILITY") && g.metrics.contains_key("cs_diff_10")
}

fn at_least(g: &Game, key: &str, v: f64) -> bool {
    g.metric(key).is_some_and(|x| x >= v)
}

fn killer(d: &Death) -> String {
    let who = d.killer_champion.as_deref().unwrap_or("tower/minions");
    match d.enemies {
        n if n > 1 => format!("killed by {who} + {}", n - 1),
        _ => format!("killed by {who}"),
    }
}

fn death_ev(g: &Game, d: &Death, note: String) -> Evidence {
    Evidence { match_id: g.match_id.clone(), t_s: Some(d.t_s), x: Some(d.x), y: Some(d.y), note }
}

fn game_ev(g: &Game, note: String) -> Evidence {
    Evidence { match_id: g.match_id.clone(), t_s: None, x: None, y: None, note }
}

fn deaths_where(g: &Game, keep: impl Fn(&Death) -> bool, note: impl Fn(&Death) -> String) -> Vec<Evidence> {
    g.deaths.iter().filter(|d| keep(d)).map(|d| death_ev(g, d, note(d))).collect()
}

pub const RULES: &[Rule] = &[
    Rule {
        kind: "early_deaths",
        title: "Dying in the laning phase",
        polarity: Polarity::Weakness,
        what: "You died 2 or more times before 14:00",
        metric: Some("deaths_pre14"),
        min_rate: 35.0,
        goal: goal("deaths_pre14", Comparator::AtMost, 1.0),
        tip: "Before trading, check the enemy jungler's last sighting and your wave state; back when you can't hold the wave safely.",
        considered: has_timeline,
        hit: |g| at_least(g, "deaths_pre14", 2.0).then(|| deaths_where(g, |d| d.before_14, killer)),
    },
    Rule {
        kind: "solo_deaths",
        title: "Losing 1v1s",
        polarity: Polarity::Weakness,
        what: "You were solo killed 2 or more times",
        metric: Some("solo_deaths"),
        min_rate: 30.0,
        goal: goal("solo_deaths", Comparator::AtMost, 1.0),
        tip: "Track the enemy's key cooldowns and level spikes; take extended trades only when you know you win them.",
        considered: has_timeline,
        hit: |g| at_least(g, "solo_deaths", 2.0).then(|| deaths_where(g, |d| d.solo, killer)),
    },
    Rule {
        kind: "deaths_while_ahead",
        title: "Dying while ahead in lane",
        polarity: Polarity::Weakness,
        what: "You died while at least 500 gold ahead of your lane opponent",
        metric: Some("deaths_ahead"),
        min_rate: 30.0,
        goal: goal("deaths_ahead", Comparator::AtMost, 0.0),
        tip: "When ahead, your bounty grows: convert the lead into plates and objectives instead of chasing kills.",
        considered: |g| g.metrics.contains_key("deaths_ahead"),
        hit: |g| {
            at_least(g, "deaths_ahead", 1.0).then(|| {
                deaths_where(
                    g,
                    |d| d.gold_vs_opponent.is_some_and(|x| x >= rc_metrics::detail::AHEAD_GOLD),
                    |d| format!("{} gold ahead of your lane opponent, {}", d.gold_vs_opponent.unwrap_or(0), killer(d)),
                )
            })
        },
    },
    Rule {
        kind: "objective_deaths",
        title: "Deaths that give objectives",
        polarity: Polarity::Weakness,
        what: "A death of yours was followed by an enemy dragon, voidgrubs, Herald, Baron or Atakhan within 60 seconds",
        metric: Some("deaths_before_obj"),
        min_rate: 35.0,
        goal: goal("deaths_before_obj", Comparator::AtMost, 0.0),
        tip: "In the minute before an objective spawns, play safe and get to the pit first instead of catching a side wave.",
        considered: |g| g.metrics.contains_key("deaths_before_obj"),
        hit: |g| {
            at_least(g, "deaths_before_obj", 1.0).then(|| {
                deaths_where(
                    g,
                    |d| d.enemy_objective.is_some(),
                    |d| {
                        format!(
                            "enemy took {} after, {}",
                            d.enemy_objective.as_deref().unwrap_or("an objective"),
                            killer(d)
                        )
                    },
                )
            })
        },
    },
    Rule {
        kind: "unspent_gold",
        title: "Dying with gold in your pocket",
        polarity: Polarity::Weakness,
        what: "You died holding 1,500 or more unspent gold",
        metric: Some("unspent_gold_death"),
        min_rate: 30.0,
        goal: goal("unspent_gold_death", Comparator::AtMost, 1000.0),
        tip: "Back when you can complete a component; unspent gold doesn't fight for you.",
        considered: has_timeline,
        hit: |g| {
            let ev = deaths_where(
                g,
                |d| d.unspent_gold.is_some_and(|x| x >= 1500),
                |d| format!("{} gold unspent (approx.)", d.unspent_gold.unwrap_or(0)),
            );
            (!ev.is_empty()).then_some(ev)
        },
    },
    Rule {
        kind: "shutdowns",
        title: "Giving away shutdowns",
        polarity: Polarity::Weakness,
        what: "You gave up 700 or more shutdown gold",
        metric: Some("shutdown_given"),
        min_rate: 25.0,
        goal: goal("shutdown_given", Comparator::AtMost, 300.0),
        tip: "With a bounty on your head, stay with your team and let the enemy come to you.",
        considered: |g| g.metrics.contains_key("shutdown_given"),
        hit: |g| {
            at_least(g, "shutdown_given", 700.0).then(|| {
                deaths_where(g, |d| d.shutdown_bounty > 0, |d| format!("gave {} gold shutdown", d.shutdown_bounty))
            })
        },
    },
    Rule {
        kind: "thrown_leads",
        title: "Losing games your team led",
        polarity: Polarity::Weakness,
        what: "Your team led by 3,000 or more gold and lost",
        metric: None,
        min_rate: 30.0,
        goal: None,
        tip: "With a lead, group for the next objective and keep vision; most throws start with a pick in an unwarded side lane.",
        considered: |g| at_least(g, "max_team_lead", 3000.0),
        hit: |g| {
            (!g.win).then(|| {
                vec![game_ev(g, format!("team led by up to {} gold", g.metric("max_team_lead").unwrap_or(0.0)))]
            })
        },
    },
    Rule {
        kind: "lane_deficit",
        title: "Falling behind in CS early",
        polarity: Polarity::Weakness,
        what: "You were 10 or more CS behind your lane opponent at 10:00",
        metric: Some("cs_diff_10"),
        min_rate: 45.0,
        goal: goal("cs_diff_10", Comparator::AtLeast, 0.0),
        tip: "Prioritise last hits over trades for the first waves, and use your waveclear to avoid losing minions under tower.",
        considered: laner,
        hit: |g| {
            g.metric("cs_diff_10")
                .filter(|v| *v <= -10.0)
                .map(|v| vec![game_ev(g, format!("{v} CS vs lane opponent at 10:00"))])
        },
    },
    Rule {
        kind: "lane_lead",
        title: "Winning lane on CS",
        polarity: Polarity::Strength,
        what: "You were 10 or more CS ahead of your lane opponent at 10:00",
        metric: Some("cs_diff_10"),
        min_rate: 50.0,
        goal: None,
        tip: "Keep it up: turn the CS lead into plates and early item spikes.",
        considered: laner,
        hit: |g| {
            g.metric("cs_diff_10")
                .filter(|v| *v >= 10.0)
                .map(|v| vec![game_ev(g, format!("+{v} CS vs lane opponent at 10:00"))])
        },
    },
    Rule {
        kind: "clean_laning",
        title: "Clean laning phase",
        polarity: Polarity::Strength,
        what: "You didn't die before 14:00",
        metric: Some("deaths_pre14"),
        min_rate: 60.0,
        goal: None,
        tip: "Safe laning is a strong base; look for windows to pressure when the enemy jungler is shown elsewhere.",
        considered: has_timeline,
        hit: |g| g.metric("deaths_pre14").filter(|v| *v == 0.0).map(|_| vec![]),
    },
];

/// Recurring rule signals across `games`.
pub fn detect(games: &[Game]) -> Vec<Signal> {
    let mut newest: Vec<&Game> = games.iter().collect();
    newest.sort_by_key(|g| std::cmp::Reverse(g.start_ms));
    RULES.iter().filter_map(|r| detect_one(r, &newest)).collect()
}

fn detect_one(rule: &Rule, games: &[&Game]) -> Option<Signal> {
    let considered: Vec<&Game> = games.iter().copied().filter(|g| (rule.considered)(g)).collect();
    let hits: Vec<(&Game, Vec<Evidence>)> =
        considered.iter().filter_map(|g| (rule.hit)(g).map(|ev| (*g, ev))).collect();
    let (n, a) = (considered.len(), hits.len());
    let rate = pct(a, n);
    if n < MIN_CONSIDERED || a < MIN_AFFECTED || rate < rule.min_rate {
        return None;
    }
    let affected = |g: &&Game| hits.iter().any(|(h, _)| h.match_id == g.match_id);
    let wr =
        win_rates(considered.iter().copied().filter(affected), considered.iter().copied().filter(|g| !affected(g)));
    let gap = win_gap(rule.polarity, wr);
    let rate_r = rate.round();
    Some(Signal {
        id: rule.kind.to_owned(),
        kind: rule.kind,
        polarity: rule.polarity,
        severity: severity(rate, gap),
        score: score(rate / 100.0, a, gap),
        title: rule.title.to_owned(),
        detail: format!("{} in {a} of {n} games ({rate_r}%).{}", rule.what, win_sentence(wr)),
        tip: rule.tip,
        games_considered: n,
        games_affected: a,
        rate_pct: rate_r,
        metric: rule.metric,
        win_rate_affected: wr.0,
        win_rate_other: wr.1,
        evidence: hits.into_iter().flat_map(|(_, ev)| ev).take(MAX_EVIDENCE).collect(),
        goal: rule.goal.clone(),
    })
}

/// A rule that fired in one game.
#[derive(Debug, Clone, Serialize)]
pub struct GameFlag {
    pub kind: &'static str,
    pub title: &'static str,
    pub polarity: Polarity,
    pub what: &'static str,
    pub evidence: Vec<Evidence>,
}

pub fn game_flags(g: &Game) -> Vec<GameFlag> {
    RULES
        .iter()
        .filter(|r| (r.considered)(g))
        .filter_map(|r| {
            (r.hit)(g).map(|evidence| GameFlag {
                kind: r.kind,
                title: r.title,
                polarity: r.polarity,
                what: r.what,
                evidence,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rc_metrics::zones::Zone;

    use super::*;
    use crate::test_support::{death, game};

    fn laning(i: usize, deaths_pre14: f64, win: bool) -> Game {
        let mut g = game(i, win, &[("deaths_pre14", deaths_pre14), ("cs_diff_10", 0.0)]);
        g.deaths = (0..deaths_pre14 as usize).map(|k| death(200.0 + k as f64 * 100.0, Zone::MidLane)).collect();
        g
    }

    #[test]
    fn early_deaths_recurring_with_win_rates() {
        // 4 of 10 games with 2+ early deaths, all losses; the rest all wins.
        let games: Vec<Game> =
            (0..10).map(|i| if i < 4 { laning(i, 2.0, false) } else { laning(i, 0.0, true) }).collect();
        let signals = detect(&games);
        let s = signals.iter().find(|s| s.kind == "early_deaths").expect("early deaths signal");
        assert_eq!((s.games_considered, s.games_affected, s.rate_pct), (10, 4, 40.0));
        assert_eq!((s.win_rate_affected, s.win_rate_other), (Some(0.0), Some(100.0)));
        assert_eq!(s.evidence.len(), 8, "two deaths per affected game");
        assert_eq!(s.evidence[0].match_id, "NA1_3", "newest first");
        assert!(s.detail.starts_with("You died 2 or more times before 14:00 in 4 of 10 games (40%)."));
        // 40% and a 100-point win-rate gap.
        assert_eq!(s.severity, crate::Severity::High);
        // 6 of 10 deathless laning phases: a strength.
        assert!(signals.iter().any(|s| s.kind == "clean_laning" && s.polarity == Polarity::Strength));
    }

    #[test]
    fn below_thresholds_nothing() {
        // 2 affected games is below MIN_AFFECTED; 4 considered is below MIN_CONSIDERED.
        let games: Vec<Game> = (0..10).map(|i| laning(i, if i < 2 { 3.0 } else { 0.0 }, true)).collect();
        assert!(detect(&games).iter().all(|s| s.kind != "early_deaths"));
        let few: Vec<Game> = (0..4).map(|i| laning(i, 3.0, true)).collect();
        assert!(detect(&few).is_empty());
    }

    #[test]
    fn thrown_leads_only_counts_games_with_a_lead() {
        let mut games = Vec::new();
        for i in 0..6 {
            // Led by 4k: 3 lost, 3 won.
            games.push(game(i, i % 2 == 0, &[("max_team_lead", 4000.0)]));
        }
        for i in 6..20 {
            games.push(game(i, false, &[("max_team_lead", 500.0)]));
        }
        let s = detect(&games).into_iter().find(|s| s.kind == "thrown_leads").expect("signal");
        assert_eq!((s.games_considered, s.games_affected), (6, 3));
        assert_eq!(s.evidence[0].note, "team led by up to 4000 gold");
    }

    #[test]
    fn supports_are_not_judged_on_cs() {
        let games: Vec<Game> = (0..10)
            .map(|i| {
                let mut g = game(i, true, &[("cs_diff_10", -40.0)]);
                g.role = Some("UTILITY".into());
                g
            })
            .collect();
        assert!(detect(&games).iter().all(|s| s.kind != "lane_deficit"));
    }

    #[test]
    fn flags_one_game() {
        let mut g = laning(1, 3.0, false);
        g.deaths[0].enemy_objective = Some("dragon".into());
        g.metrics.insert("deaths_before_obj".into(), 1.0);
        let flags = game_flags(&g);
        let kinds: Vec<_> = flags.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, vec!["early_deaths", "objective_deaths"]);
        assert_eq!(flags[1].evidence[0].note, "enemy took dragon after, killed by Ahri + 1");
    }
}
