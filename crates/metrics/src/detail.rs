//! Views for the match detail screen: per-minute series for the charts,
//! positioned events for the minimap, and context for each death.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::compute::{frame_before, lane_opponent};
use crate::model::{Match, Participant, Timeline};

/// Summoner's Rift coordinate bounds, for projecting onto the minimap.
pub const MAP_MIN: i32 = -120;
pub const MAP_MAX_X: i32 = 14870;
pub const MAP_MAX_Y: i32 = 14980;

#[derive(Debug, Clone, Serialize, Default)]
pub struct ParticipantSeries {
    pub gold: Vec<i32>,
    pub xp: Vec<i32>,
    pub cs: Vec<i32>,
    pub level: Vec<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Series {
    /// Game time of each frame, in minutes.
    pub minutes: Vec<f64>,
    pub by_participant: BTreeMap<i32, ParticipantSeries>,
}

pub fn series(tl: &Timeline) -> Series {
    let minutes = tl.info.frames.iter().map(|f| f.timestamp as f64 / 60_000.0).collect();
    let mut by_participant: BTreeMap<i32, ParticipantSeries> = BTreeMap::new();
    for f in &tl.info.frames {
        for pid in 1..=10 {
            let s = by_participant.entry(pid).or_default();
            let pf = f.participant(pid);
            s.gold.push(pf.map_or(0, |p| p.total_gold));
            s.xp.push(pf.map_or(0, |p| p.xp));
            s.cs.push(pf.map_or(0, |p| p.cs()));
            s.level.push(pf.map_or(0, |p| p.level));
        }
    }
    Series { minutes, by_participant }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MapEventKind {
    Kill,
    Objective,
    Building,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapEvent {
    pub kind: MapEventKind,
    pub t_s: f64,
    pub x: i32,
    pub y: i32,
    /// For kills; 0 means a turret/minion/monster got the kill.
    pub killer_id: Option<i32>,
    pub victim_id: Option<i32>,
    pub assist_ids: Vec<i32>,
    /// Team credited with the event (kills: killer's team).
    pub team_id: Option<i32>,
    /// e.g. "DRAGON / CHEMTECH_DRAGON", "OUTER_TURRET MID_LANE".
    pub label: Option<String>,
    pub shutdown_bounty: i32,
}

pub fn map_events(m: &Match, tl: &Timeline) -> Vec<MapEvent> {
    let team_of = |pid: i32| m.info.participants.iter().find(|p| p.participant_id == pid).map(|p| p.team_id);
    let mut out = Vec::new();
    for e in tl.info.frames.iter().flat_map(|f| f.events.iter()) {
        let Some(pos) = e.position else { continue };
        let t_s = e.timestamp as f64 / 1000.0;
        match e.kind.as_str() {
            "CHAMPION_KILL" => {
                let team = e
                    .killer_id
                    .and_then(team_of)
                    .or_else(|| e.victim_id.and_then(team_of).map(|t| if t == 100 { 200 } else { 100 }));
                out.push(MapEvent {
                    kind: MapEventKind::Kill,
                    t_s,
                    x: pos.x,
                    y: pos.y,
                    killer_id: e.killer_id,
                    victim_id: e.victim_id,
                    assist_ids: e.assisting_participant_ids.clone(),
                    team_id: team,
                    label: None,
                    shutdown_bounty: e.shutdown_bounty.unwrap_or(0),
                });
            }
            "ELITE_MONSTER_KILL" => out.push(MapEvent {
                kind: MapEventKind::Objective,
                t_s,
                x: pos.x,
                y: pos.y,
                killer_id: e.killer_id,
                victim_id: None,
                assist_ids: e.assisting_participant_ids.clone(),
                team_id: e.killer_team_id,
                label: Some(match (&e.monster_type, &e.monster_sub_type) {
                    (Some(t), Some(s)) => format!("{t} / {s}"),
                    (Some(t), None) => t.clone(),
                    _ => "MONSTER".into(),
                }),
                shutdown_bounty: 0,
            }),
            "BUILDING_KILL" => out.push(MapEvent {
                kind: MapEventKind::Building,
                t_s,
                x: pos.x,
                y: pos.y,
                killer_id: e.killer_id,
                victim_id: None,
                assist_ids: e.assisting_participant_ids.clone(),
                team_id: e.team_id.map(|lost| if lost == 100 { 200 } else { 100 }),
                label: Some(
                    [e.tower_type.as_deref(), e.building_type.as_deref(), e.lane_type.as_deref()]
                        .into_iter()
                        .flatten()
                        .filter(|s| *s != "TOWER_BUILDING")
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                shutdown_bounty: 0,
            }),
            _ => {}
        }
    }
    out
}

/// Context for one of the subject's deaths, for the death list and AI facts.
#[derive(Debug, Clone, Serialize)]
pub struct DeathContext {
    pub t_s: f64,
    pub x: i32,
    pub y: i32,
    pub killer_id: Option<i32>,
    pub assist_ids: Vec<i32>,
    /// Enemy champions credited (killer + assists).
    pub enemies_involved: usize,
    pub before_14: bool,
    /// Past the river diagonal, on the enemy's side of the map (approximate).
    pub in_enemy_half: bool,
    /// Gold lead over the lane opponent at the last minute snapshot.
    pub gold_vs_opponent: Option<i32>,
    /// Gold in pocket at the last minute snapshot (approximate).
    pub unspent_gold: Option<i32>,
    pub shutdown_bounty: i32,
}

/// Rough river diagonal: blue base is bottom-left, red top-right.
fn in_enemy_half(team_id: i32, x: i32, y: i32) -> bool {
    let s = x + y;
    if team_id == 100 { s > MAP_MAX_X + 300 } else { s < MAP_MAX_X - 300 }
}

pub fn deaths_of(m: &Match, tl: &Timeline, subject: &Participant) -> Vec<DeathContext> {
    let opp = lane_opponent(&m.info, subject);
    let mut out = Vec::new();
    for e in tl.info.frames.iter().flat_map(|f| f.events.iter()) {
        if e.kind != "CHAMPION_KILL" || e.victim_id != Some(subject.participant_id) {
            continue;
        }
        let pos = e.position.unwrap_or_default();
        let frame = frame_before(tl, e.timestamp);
        let me = frame.and_then(|f| f.participant(subject.participant_id));
        let them = opp.and_then(|o| frame.and_then(|f| f.participant(o.participant_id)));
        let killer = e.killer_id.filter(|k| (1..=10).contains(k));
        out.push(DeathContext {
            t_s: e.timestamp as f64 / 1000.0,
            x: pos.x,
            y: pos.y,
            killer_id: e.killer_id,
            assist_ids: e.assisting_participant_ids.clone(),
            enemies_involved: usize::from(killer.is_some()) + e.assisting_participant_ids.len(),
            before_14: e.timestamp < 14 * 60_000,
            in_enemy_half: in_enemy_half(subject.team_id, pos.x, pos.y),
            gold_vs_opponent: me.zip(them).map(|(a, b)| a.total_gold - b.total_gold),
            unspent_gold: me.map(|p| p.current_gold),
            shutdown_bounty: e.shutdown_bounty.unwrap_or(0),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn river_side() {
        // Blue jungle (bottom-left) is home for blue, enemy for red.
        assert!(!in_enemy_half(100, 3800, 7900));
        assert!(in_enemy_half(200, 3800, 7900));
        // Red jungle.
        assert!(in_enemy_half(100, 11000, 7000));
        assert!(!in_enemy_half(200, 11000, 7000));
    }
}
