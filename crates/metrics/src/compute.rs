//! Per-participant metrics for one match. Everything is derived from the
//! match and (when present) timeline JSON, so it can be recomputed at will.

use std::collections::{BTreeMap, HashMap};

use crate::model::{Event, Frame, ItemCatalog, Match, MatchInfo, Participant, Timeline};

/// Bump to recompute every stored match (e.g. after fixing a metric).
pub const METRICS_VERSION: i64 = 2;

const MIN: i64 = 60_000;
/// Purchases before this are the starting items, not a back.
const FIRST_BACK_AFTER_MS: i64 = 90_000;
/// Purchases within this window of the first one count as the same back.
const SHOP_VISIT_MS: i64 = 30_000;
const LANING_PHASE_MS: i64 = 14 * MIN;

pub type MetricMap = BTreeMap<String, f64>;

#[derive(Debug, Clone)]
pub struct MatchMetrics {
    pub match_id: String,
    /// participant_id → metric key → value
    pub by_participant: BTreeMap<i32, MetricMap>,
}

#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    #[error("unparseable {what}: {source}")]
    Parse { what: &'static str, source: serde_json::Error },
}

pub fn parse_match(json: &[u8]) -> Result<Match, MetricsError> {
    serde_json::from_slice(json).map_err(|source| MetricsError::Parse { what: "match", source })
}

pub fn parse_timeline(json: &[u8]) -> Result<Timeline, MetricsError> {
    serde_json::from_slice(json).map_err(|source| MetricsError::Parse { what: "timeline", source })
}

/// Same role on the other team.
pub fn lane_opponent<'a>(info: &'a MatchInfo, p: &Participant) -> Option<&'a Participant> {
    let role = p.role()?;
    info.participants.iter().find(|o| o.team_id != p.team_id && o.role() == Some(role))
}

/// The frame taken at `minute`, if the game lasted that long.
pub fn frame_at(tl: &Timeline, minute: usize) -> Option<&Frame> {
    let f = tl.info.frames.get(minute)?;
    // The last frame is the game end, which can fall short of the minute.
    (f.timestamp >= minute as i64 * MIN - 1_000).then_some(f)
}

/// Last frame at or before `t_ms`.
pub fn frame_before(tl: &Timeline, t_ms: i64) -> Option<&Frame> {
    tl.info.frames.iter().take_while(|f| f.timestamp <= t_ms).last()
}

fn events(tl: &Timeline) -> impl Iterator<Item = &Event> {
    tl.info.frames.iter().flat_map(|f| f.events.iter())
}

fn pct(part: f64, whole: f64) -> Option<f64> {
    (whole > 0.0).then(|| part / whole * 100.0)
}

pub fn compute(m: &Match, tl: Option<&Timeline>, items: &ItemCatalog) -> MatchMetrics {
    let info = &m.info;
    let minutes = (info.duration_s() / 60.0).max(1.0);

    let mut team_kills: HashMap<i32, f64> = HashMap::new();
    let mut team_dmg: HashMap<i32, f64> = HashMap::new();
    let mut team_gold: HashMap<i32, f64> = HashMap::new();
    let mut team_taken: HashMap<i32, f64> = HashMap::new();
    for p in &info.participants {
        *team_kills.entry(p.team_id).or_default() += f64::from(p.kills);
        *team_dmg.entry(p.team_id).or_default() += p.total_damage_dealt_to_champions as f64;
        *team_gold.entry(p.team_id).or_default() += f64::from(p.gold_earned);
        *team_taken.entry(p.team_id).or_default() += p.total_damage_taken as f64;
    }

    let timeline_stats = tl.map(|tl| TimelineStats::collect(info, tl, items));

    let mut by_participant = BTreeMap::new();
    for p in &info.participants {
        let mut out = MetricMap::new();
        let mut put = |k: &str, v: Option<f64>| {
            if let Some(v) = v.filter(|v| v.is_finite()) {
                out.insert(k.to_owned(), v);
            }
        };

        // From the match alone.
        put("kills", Some(f64::from(p.kills)));
        put("deaths", Some(f64::from(p.deaths)));
        put("assists", Some(f64::from(p.assists)));
        put("kda", Some(f64::from(p.kills + p.assists) / f64::from(p.deaths.max(1))));
        put("cspm", Some(f64::from(p.cs()) / minutes));
        put("gpm", Some(f64::from(p.gold_earned) / minutes));
        put("dpm", Some(p.total_damage_dealt_to_champions as f64 / minutes));
        put("vision_pm", Some(f64::from(p.vision_score) / minutes));
        put("control_wards", Some(f64::from(p.detector_wards_placed)));
        put("time_dead_pct", pct(p.total_time_spent_dead as f64, info.duration_s()));
        put("kp", pct(f64::from(p.kills + p.assists), team_kills[&p.team_id]).map(|v| v.min(100.0)));
        let dmg_share = pct(p.total_damage_dealt_to_champions as f64, team_dmg[&p.team_id]);
        let gold_share = pct(f64::from(p.gold_earned), team_gold[&p.team_id]);
        put("dmg_share", dmg_share);
        put("gold_share", gold_share);
        if let (Some(d), Some(g)) = (dmg_share, gold_share) {
            put("dmg_per_gold_share", (g > 0.0).then(|| d / g));
        }
        put("dmg_taken_share", pct(p.total_damage_taken as f64, team_taken[&p.team_id]));

        // From the timeline.
        if let (Some(tl), Some(ts)) = (tl, &timeline_stats) {
            let opp = lane_opponent(info, p);
            for minute in [10usize, 15] {
                let Some(frame) = frame_at(tl, minute) else { continue };
                let Some(me) = frame.participant(p.participant_id) else { continue };
                put(&format!("cs_{minute}"), Some(f64::from(me.cs())));
                put(&format!("gold_{minute}"), Some(f64::from(me.total_gold)));
                put(&format!("xp_{minute}"), Some(f64::from(me.xp)));
                if let Some(o) = opp.and_then(|o| frame.participant(o.participant_id)) {
                    put(&format!("cs_diff_{minute}"), Some(f64::from(me.cs() - o.cs())));
                    put(&format!("gold_diff_{minute}"), Some(f64::from(me.total_gold - o.total_gold)));
                    put(&format!("xp_diff_{minute}"), Some(f64::from(me.xp - o.xp)));
                }
            }
            let pid = p.participant_id;
            let get = |m: &HashMap<i32, f64>| m.get(&pid).copied().unwrap_or(0.0);
            put("deaths_pre14", Some(get(&ts.deaths_pre14)));
            put("solo_kills", Some(get(&ts.solo_kills)));
            put("solo_deaths", Some(get(&ts.solo_deaths)));
            put("shutdown_given", Some(get(&ts.shutdown_given)));
            put("wards_placed_pm", Some(get(&ts.wards_placed) / minutes));
            put("wards_killed_pm", Some(get(&ts.wards_killed) / minutes));
            put("obj_part", pct(get(&ts.objectives), ts.team_objectives.get(&p.team_id).copied().unwrap_or(0.0)));
            if let Some(deaths) = ts.unspent_at_death.get(&pid).filter(|v| !v.is_empty()) {
                put("unspent_gold_death", Some(deaths.iter().sum::<f64>() / deaths.len() as f64));
            }
            if let Some(back) = ts.first_back.get(&pid) {
                put("first_back_min", Some(back.at_ms as f64 / MIN as f64));
                put("first_back_gold", back.gold_spent.map(f64::from));
            }
            put("first_item_min", ts.first_item_ms.get(&pid).map(|t| *t as f64 / MIN as f64));
            put("max_team_lead", ts.max_team_lead.get(&p.team_id).map(|v| f64::from(*v)));
        }
        by_participant.insert(p.participant_id, out);
    }
    MatchMetrics { match_id: m.metadata.match_id.clone(), by_participant }
}

#[derive(Debug, Clone, Copy)]
struct FirstBack {
    at_ms: i64,
    gold_spent: Option<i32>,
}

/// Event-derived tallies, computed once per match.
#[derive(Default)]
struct TimelineStats {
    deaths_pre14: HashMap<i32, f64>,
    solo_kills: HashMap<i32, f64>,
    solo_deaths: HashMap<i32, f64>,
    shutdown_given: HashMap<i32, f64>,
    wards_placed: HashMap<i32, f64>,
    wards_killed: HashMap<i32, f64>,
    objectives: HashMap<i32, f64>,
    team_objectives: HashMap<i32, f64>,
    unspent_at_death: HashMap<i32, Vec<f64>>,
    first_back: HashMap<i32, FirstBack>,
    first_item_ms: HashMap<i32, i64>,
    max_team_lead: HashMap<i32, i32>,
}

fn is_champion(id: Option<i32>) -> Option<i32> {
    id.filter(|i| (1..=10).contains(i))
}

const OBJECTIVE_MONSTERS: [&str; 5] = ["DRAGON", "HORDE", "RIFTHERALD", "BARON_NASHOR", "ATAKHAN"];

impl TimelineStats {
    fn collect(info: &MatchInfo, tl: &Timeline, items: &ItemCatalog) -> Self {
        let team_of: HashMap<i32, i32> = info.participants.iter().map(|p| (p.participant_id, p.team_id)).collect();
        let mut s = Self::default();
        let mut purchases: HashMap<i32, Vec<&Event>> = HashMap::new();
        let mut destroyed: HashMap<i32, Vec<&Event>> = HashMap::new();

        for e in events(tl) {
            match e.kind.as_str() {
                "CHAMPION_KILL" => {
                    let killer = is_champion(e.killer_id);
                    let solo = e.assisting_participant_ids.is_empty();
                    if let Some(victim) = is_champion(e.victim_id) {
                        if e.timestamp < LANING_PHASE_MS {
                            *s.deaths_pre14.entry(victim).or_default() += 1.0;
                        }
                        if solo && killer.is_some() {
                            *s.solo_deaths.entry(victim).or_default() += 1.0;
                        }
                        *s.shutdown_given.entry(victim).or_default() += f64::from(e.shutdown_bounty.unwrap_or(0));
                        let gold = frame_before(tl, e.timestamp)
                            .and_then(|f| f.participant(victim))
                            .map(|pf| f64::from(pf.current_gold));
                        s.unspent_at_death.entry(victim).or_default().extend(gold);
                    }
                    if let (Some(k), true) = (killer, solo) {
                        *s.solo_kills.entry(k).or_default() += 1.0;
                    }
                }
                "WARD_PLACED" if e.ward_type.as_deref() != Some("UNDEFINED") => {
                    if let Some(c) = is_champion(e.creator_id) {
                        *s.wards_placed.entry(c).or_default() += 1.0;
                    }
                }
                "WARD_KILL" => {
                    if let Some(k) = is_champion(e.killer_id) {
                        *s.wards_killed.entry(k).or_default() += 1.0;
                    }
                }
                "ELITE_MONSTER_KILL" if e.monster_type.as_deref().is_some_and(|t| OBJECTIVE_MONSTERS.contains(&t)) => {
                    let team = e.killer_team_id.or_else(|| e.killer_id.and_then(|k| team_of.get(&k).copied()));
                    if let Some(team) = team {
                        s.credit_objective(team, e, &team_of);
                    }
                }
                "BUILDING_KILL" if e.building_type.as_deref() == Some("TOWER_BUILDING") => {
                    // `teamId` is the team that *lost* the tower.
                    if let Some(lost) = e.team_id {
                        let team = if lost == 100 { 200 } else { 100 };
                        s.credit_objective(team, e, &team_of);
                    }
                }
                "ITEM_PURCHASED" => {
                    if let Some(p) = is_champion(e.participant_id) {
                        purchases.entry(p).or_default().push(e);
                    }
                }
                "ITEM_DESTROYED" => {
                    if let Some(p) = is_champion(e.participant_id) {
                        destroyed.entry(p).or_default().push(e);
                    }
                }
                _ => {}
            }
        }

        for (pid, buys) in &purchases {
            if let Some(first) = buys.iter().find(|e| e.timestamp > FIRST_BACK_AFTER_MS) {
                let start = first.timestamp;
                let in_visit = |e: &&&Event| e.timestamp >= start && e.timestamp <= start + SHOP_VISIT_MS;
                let gold_spent = (!items.is_empty()).then(|| {
                    let price = |e: &&Event| e.item_id.and_then(|i| items.get(i)).map_or(0, |i| i.total_gold);
                    let bought: i32 = buys.iter().filter(in_visit).map(price).sum();
                    let used: i32 = destroyed.get(pid).map_or(0, |d| d.iter().filter(in_visit).map(price).sum());
                    (bought - used).max(0)
                });
                s.first_back.insert(*pid, FirstBack { at_ms: start, gold_spent });
            }
            if let Some(e) = buys.iter().find(|e| e.item_id.and_then(|i| items.get(i)).is_some_and(|i| i.completed)) {
                s.first_item_ms.insert(*pid, e.timestamp);
            }
        }

        // Frame 0 (everyone at starting gold) would make "never led" read 0.
        for f in tl.info.frames.iter().skip(1) {
            let mut gold: HashMap<i32, i32> = HashMap::new();
            for (pid, team) in &team_of {
                if let Some(pf) = f.participant(*pid) {
                    *gold.entry(*team).or_default() += pf.total_gold;
                }
            }
            let (blue, red) = (gold.get(&100).copied().unwrap_or(0), gold.get(&200).copied().unwrap_or(0));
            for (team, lead) in [(100, blue - red), (200, red - blue)] {
                let best = s.max_team_lead.entry(team).or_insert(lead);
                *best = (*best).max(lead);
            }
        }
        s
    }

    fn credit_objective(&mut self, team: i32, e: &Event, team_of: &HashMap<i32, i32>) {
        *self.team_objectives.entry(team).or_default() += 1.0;
        let mut involved: Vec<i32> = e.assisting_participant_ids.clone();
        involved.extend(is_champion(e.killer_id));
        involved.sort_unstable();
        involved.dedup();
        for p in involved {
            if team_of.get(&p) == Some(&team) {
                *self.objectives.entry(p).or_default() += 1.0;
            }
        }
    }
}
