//! The facts object sent to the AI: structured metrics, comparisons, and
//! death context. Never raw API dumps, never other players' Riot IDs.

use rc_metrics::detail::DeathContext;
use rc_metrics::model::ItemCatalog;
use rc_metrics::percentile::Comparison;
use serde_json::{Value, json};

use crate::analysis::{MetricComparison, MetricView};
use crate::detail::Loaded;

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
    for (k, c) in [("vs_role", &m.vs_role), ("vs_champion", &m.vs_champion), ("vs_personal", &m.vs_personal)] {
        if c.is_some() {
            v[k] = cmp_json(c, m.decimals);
        }
    }
    v
}

pub fn build(l: &Loaded, cmp: &MetricComparison, deaths: &[DeathContext], items: Option<&ItemCatalog>) -> Value {
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
        },
        "metrics": cmp.metrics.iter().map(metric_json).collect::<Vec<_>>(),
        "deaths": deaths.iter().enumerate().map(|(i, d)| json!({
            "id": format!("death_{}", i + 1),
            "time": clock(d.t_s),
            "killer_champion": champ_of(d.killer_id.filter(|k| (1..=10).contains(k))),
            "enemies_involved": d.enemies_involved,
            "before_14": d.before_14,
            "in_enemy_half": d.in_enemy_half,
            "gold_vs_lane_opponent": d.gold_vs_opponent,
            "unspent_gold": d.unspent_gold,
            "shutdown_bounty": d.shutdown_bounty,
        })).collect::<Vec<_>>(),
        "notes": notes,
    })
}

/// Valid evidence ids in a facts object.
pub fn evidence_ids(facts: &Value) -> Vec<String> {
    let mut ids = vec!["match".to_string()];
    for list in ["metrics", "deaths"] {
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
