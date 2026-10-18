//! The facts object sent to the AI: structured metrics, comparisons, and
//! death context. Never raw API dumps, never other players' Riot IDs.

use rc_metrics::detail::DeathContext;
use rc_metrics::model::ItemCatalog;
use rc_metrics::percentile::Comparison;
use serde_json::{Value, json};

use crate::analysis::{MetricComparison, MetricView};
use crate::detail::Loaded;
use crate::goals::GoalView;
use crate::patterns::MatchPattern;
use crate::sessions::SessionDetail;

pub fn round_to(v: f64, decimals: u8) -> f64 {
    let f = 10f64.powi(i32::from(decimals));
    (v * f).round() / f
}

/// `754.2` s → `"12:34"`.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn cmp_json(c: &Option<Comparison>, decimals: u8) -> Value {
    match c {
        Some(c) => json!({
            "better_than_pct": c.better_than_pct.round(),
            "median": round_to(c.median, decimals),
            "n": c.n,
        }),
        None => Value::Null,
    }
}

fn metric_json(m: &MetricView) -> Value {
    let mut v = json!({
        "id": m.key,
        "label": m.label,
        "value": round_to(m.value, m.decimals),
        "unit": m.unit,
        "higher_is_better": m.higher_is_better,
    });
    if m.approximate {
        v["approximate"] = json!(true);
    }
    for (k, c) in [
        ("vs_role", &m.vs_role),
        ("vs_champion", &m.vs_champion),
        ("vs_personal", &m.vs_personal),
        ("vs_benchmark", &m.vs_benchmark),
        ("vs_benchmark_champion", &m.vs_benchmark_champion),
    ] {
        if c.is_some() {
            v[k] = cmp_json(c, m.decimals);
        }
    }
    v
}

fn round_metric(key: &str, v: f64) -> f64 {
    round_to(v, rc_metrics::defs::get(key).map_or(1, |d| d.decimals.max(1)))
}

/// This game's patterns, and how often each recurs in your recent games.
pub fn patterns_json(patterns: &[MatchPattern]) -> Vec<Value> {
    patterns
        .iter()
        .map(|p| {
            let mut v = json!({
                "id": format!("pattern:{}", p.flag.kind),
                "title": p.flag.title,
                "type": p.flag.polarity,
                "rule": p.flag.what,
                "in_this_game": p.flag.evidence.iter().map(|e| match e.t_s {
                    Some(t) => format!("{} {}", clock(t), e.note),
                    None => e.note.clone(),
                }).collect::<Vec<_>>(),
            });
            if let Some(r) = &p.recurring {
                v["recurring"] = json!({
                    "games_affected": r.games_affected,
                    "games_considered": r.games_considered,
                    "rate_pct": r.rate_pct,
                    "win_rate_when_it_happens": r.win_rate_affected,
                    "win_rate_otherwise": r.win_rate_other,
                });
            }
            v
        })
        .collect()
}

/// Active focus goals with progress (and this game's value, if it counts).
pub fn goals_json(goals: &[GoalView], match_id: Option<&str>) -> Vec<Value> {
    goals
        .iter()
        .map(|g| {
            let p = &g.progress;
            let mut v = json!({
                "id": format!("goal_{}", g.goal.id),
                "title": g.title,
                "metric": g.goal.metric,
                "comparator": g.goal.comparator,
                "target": g.goal.target,
                "games_total": p.games_total,
                "games_played": p.games_played,
                "games_met": p.games_met,
                "average_so_far": p.average.map(|a| round_metric(&g.goal.metric, a)),
                "baseline_before_goal": g.goal.baseline.map(|b| round_metric(&g.goal.metric, b)),
                "on_track": p.on_track,
            });
            if let Some(this) = match_id.and_then(|id| p.games.iter().find(|x| x.match_id == id)) {
                v["this_game"] = json!({ "value": round_metric(&g.goal.metric, this.value), "met": this.met });
            }
            v
        })
        .collect()
}

pub fn build(
    l: &Loaded,
    cmp: &MetricComparison,
    deaths: &[DeathContext],
    items: Option<&ItemCatalog>,
    patterns: &[MatchPattern],
    goals: &[GoalView],
) -> Value {
    let info = &l.game.info;
    let champ_of = |pid: Option<i32>| {
        pid.and_then(|id| info.participants.iter().find(|p| p.participant_id == id)).map(|p| p.champion_name.clone())
    };
    let me = l.subject().ok();
    let opponent = me.and_then(|m| rc_metrics::compute::lane_opponent(info, m));
    let s = &l.summary;
    let final_items: Vec<String> = me
        .map(|m| {
            m.items()
                .iter()
                .filter(|&&id| id != 0)
                .map(|&id| items.and_then(|c| c.get(id)).map_or_else(|| format!("item {id}"), |i| i.name.clone()))
                .collect()
        })
        .unwrap_or_default();

    let mut notes = vec![
        "Positions come from Riot's timeline, sampled about once per minute; locations are approximate.".to_string(),
        "Populations are other players in your own stored games, not a global ladder.".to_string(),
    ];
    if l.timeline.is_none() {
        notes.push(
            "No timeline is stored for this game: laning, vision-per-minute and death details are missing.".into(),
        );
    }

    json!({
        "match": {
            "match_id": s.match_id,
            "champion": s.champion_name,
            "role": s.team_position,
            "lane_opponent_champion": opponent.map(|o| o.champion_name.clone()),
            "result": if s.win { "win" } else { "loss" },
            "duration": clock(s.duration_s as f64),
            "queue": rc_common::queues::queue_name(s.queue_id),
            "patch": s.patch,
            "kills": s.kills,
            "deaths": s.deaths,
            "assists": s.assists,
            "final_items": final_items,
        },
        "populations": {
            "vs_role": cmp.populations.role_label,
            "vs_champion": cmp.populations.champion_label,
            "vs_personal": cmp.populations.personal_label,
            "vs_benchmark": cmp.populations.benchmark_label,
            "vs_benchmark_champion": cmp.populations.benchmark_champion_label,
        },
        "metrics": cmp.metrics.iter().map(metric_json).collect::<Vec<_>>(),
        "deaths": deaths.iter().enumerate().map(|(i, d)| json!({
            "id": format!("death_{}", i + 1),
            "time": clock(d.t_s),
            "killer_champion": champ_of(d.killer_id.filter(|k| (1..=10).contains(k))),
            "enemies_involved": d.enemies_involved,
            "before_14": d.before_14,
            "map_area": d.zone.label(),
            "in_enemy_half": d.in_enemy_half,
            "gold_vs_lane_opponent": d.gold_vs_opponent,
            "unspent_gold": d.unspent_gold,
            "shutdown_bounty": d.shutdown_bounty,
            "enemy_objective_within_60s": d.enemy_objective,
        })).collect::<Vec<_>>(),
        "patterns": patterns_json(patterns),
        "goals": goals_json(goals, Some(&s.match_id)),
        "notes": notes,
    })
}

/// Facts for a session review: each game, session averages vs your usual,
/// patterns that showed up, and goal progress.
pub fn session(d: &SessionDetail, goals: &[GoalView]) -> Value {
    let s = &d.summary;
    let rift: Vec<_> = d.games.iter().filter(|g| g.is_standard_rift).collect();
    let mut notes = vec![
        "Averages and patterns cover Summoner's Rift games only (no ARAM/Arena, no remakes).".to_string(),
        format!("\"baseline\" values are your averages over the {} games before this session.", d.baseline_games),
        "Positions come from Riot's timeline, sampled about once per minute; locations are approximate.".to_string(),
    ];
    if d.baseline_games < 5 {
        notes.push("Few games before this session: baseline comparisons are weak.".into());
    }
    let play_minutes: i64 = d.games.iter().map(|g| g.summary.duration_s).sum::<i64>() / 60;
    json!({
        "session": {
            "games": s.games,
            "summoners_rift_games": s.rift_games,
            "wins": s.wins,
            "losses": s.losses,
            "results_in_order": d.games.iter().filter(|g| !g.summary.is_remake)
                .map(|g| if g.summary.win { "W" } else { "L" }).collect::<String>(),
            "minutes_played": play_minutes,
        },
        "games": rift.iter().enumerate().map(|(i, g)| {
            let m = &g.summary;
            let pick = |k: &str| g.metrics.get(k).map(|v| round_metric(k, *v));
            json!({
                "id": format!("game_{}", i + 1),
                "match_id": m.match_id,
                "champion": m.champion_name,
                "role": m.team_position,
                "result": if m.win { "win" } else { "loss" },
                "duration": clock(m.duration_s as f64),
                "kills": m.kills,
                "deaths": m.deaths,
                "assists": m.assists,
                "cs_10": pick("cs_10"),
                "cs_diff_10": pick("cs_diff_10"),
                "gold_diff_15": pick("gold_diff_15"),
                "deaths_pre14": pick("deaths_pre14"),
                "kp": pick("kp"),
                "dmg_share": pick("dmg_share"),
                "patterns": g.flags.iter().map(|f| f.kind).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "aggregates": d.aggregates.iter().map(|a| json!({
            "id": a.metric,
            "label": a.label,
            "unit": a.unit,
            "higher_is_better": a.higher_is_better,
            "session_average": round_to(a.session, a.decimals.max(1)),
            "baseline": a.baseline.map(|b| round_to(b, a.decimals.max(1))),
            "better_than_baseline": a.better,
            "games": a.games,
        })).collect::<Vec<_>>(),
        "patterns": d.patterns.iter().map(|p| json!({
            "id": format!("pattern:{}", p.kind),
            "title": p.title,
            "type": p.polarity,
            "rule": p.what,
            "session_games_affected": p.session_games_affected,
            "session_games": p.session_games,
            "baseline_rate_pct": p.baseline_rate_pct,
        })).collect::<Vec<_>>(),
        "goals": goals_json(goals, None),
        "notes": notes,
    })
}

fn gaps_json(g: &crate::benchmarks::GapsView) -> Value {
    json!({
        "reference": g.reference,
        "your_games": g.games,
        "metrics": g.gaps.iter().map(|x| json!({
            "id": format!("bench:{}", x.metric),
            "label": x.label,
            "yours": round_to(x.yours, x.decimals.max(1)),
            "benchmark_median": round_to(x.benchmark_median, x.decimals.max(1)),
            "better_than_pct_of_benchmark": x.better_than_pct.round(),
            "early_game": x.early,
        })).collect::<Vec<_>>(),
    })
}

/// Facts for a weekly report: the week vs the four weeks before, champions,
/// patterns, the weekly trend, benchmark gaps, and goals.
pub fn weekly(d: &crate::weekly::WeekDetail, goals: &[GoalView]) -> Value {
    let s = &d.summary;
    let mut notes = vec![
        "Averages and patterns cover Summoner's Rift games only (no ARAM/Arena, no remakes).".to_string(),
        format!("\"baseline\" values are your averages over the {} games in the 4 weeks before.", d.baseline_games),
    ];
    if d.baseline_games < 5 {
        notes.push("Few games in the weeks before: baseline comparisons are weak.".into());
    }
    if d.benchmark.is_none() {
        notes.push("No benchmark comparison yet (higher-rank games are still being collected).".into());
    }
    json!({
        "week": {
            "starts": s.week,
            "games": s.games,
            "summoners_rift_games": s.rift_games,
            "wins": s.wins,
            "losses": s.losses,
        },
        "champions": d.champions.iter().map(|c| json!({
            "id": format!("champ:{}", c.champion),
            "champion": c.champion,
            "games": c.games,
            "wins": c.wins,
        })).collect::<Vec<_>>(),
        "aggregates": d.aggregates.iter().map(|a| json!({
            "id": a.metric,
            "label": a.label,
            "unit": a.unit,
            "higher_is_better": a.higher_is_better,
            "week_average": round_to(a.session, a.decimals.max(1)),
            "baseline": a.baseline.map(|b| round_to(b, a.decimals.max(1))),
            "better_than_baseline": a.better,
            "games": a.games,
        })).collect::<Vec<_>>(),
        "patterns": d.patterns.iter().map(|p| json!({
            "id": format!("pattern:{}", p.kind),
            "title": p.title,
            "type": p.polarity,
            "rule": p.what,
            "week_games_affected": p.session_games_affected,
            "week_games": p.session_games,
            "baseline_rate_pct": p.baseline_rate_pct,
        })).collect::<Vec<_>>(),
        "previous_weeks": d.trend.iter().map(|t| json!({
            "id": format!("week:{}", t.week),
            "games": t.games,
            "win_rate_pct": t.win_rate_pct,
            "averages": t.metrics.iter().map(|(k, v)| (k.clone(), json!(round_metric(k, *v)))).collect::<serde_json::Map<_, _>>(),
        })).collect::<Vec<_>>(),
        "benchmark_gaps": d.benchmark.as_ref().map(gaps_json),
        "goals": goals_json(goals, None),
        "notes": notes,
    })
}

/// Valid evidence ids in a facts object.
pub fn evidence_ids(facts: &Value) -> Vec<String> {
    let mut ids = ["match", "session", "week", "matchup", "record"].map(str::to_owned).to_vec();
    for bench in ["benchmark_gaps", "benchmark_targets"] {
        if let Some(a) = facts[bench]["metrics"].as_array() {
            ids.extend(a.iter().filter_map(|m| m["id"].as_str().map(str::to_owned)));
        }
    }
    for list in
        ["metrics", "deaths", "patterns", "goals", "games", "aggregates", "champions", "previous_weeks", "enemies"]
    {
        if let Some(a) = facts[list].as_array() {
            ids.extend(a.iter().filter_map(|m| m["id"].as_str().map(str::to_owned)));
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        assert_eq!(clock(754.2), "12:34");
        assert_eq!(clock(5.0), "0:05");
        assert_eq!(round_to(1.23456, 2), 1.23);
        assert_eq!(round_to(-10.6, 0), -11.0);
    }
}
