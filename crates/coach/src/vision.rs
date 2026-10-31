//! Vision review: after a game, the frames captured around its key moments
//! (from Riot's timeline) go to a vision model with the moments' facts.
//! Runs only post-game. Other players are named by champion only, and any
//! Riot ID the model copies from a screenshot is replaced before storing.

use rc_ai::{AiClient, CompletionRequest, ImagePart, extract_json};
use rc_metrics::compute::{OBJECTIVE_MONSTERS, frame_before, lane_opponent};
use rc_metrics::detail::{deaths_of, objective_label};
use rc_store::Db;
use rc_store::reports::{self, NewReport, Report};
use rc_vision::{Anchor, AnchorKind, FrameStore, Moment};
use serde_json::{Value, json};

use crate::CoachError;
use crate::detail::{Loaded, load};
use crate::facts::clock;
use crate::validate::Validation;

pub const VISION: &str = "vision";
pub const VISION_PROMPT_VERSION: &str = "vision_review.v1";
const VISION_PROMPT: &str = include_str!("../../../prompts/vision_review.v1.md");

/// Fixed game times (minutes) worth one frame each.
const CHECKPOINT_MIN: [i64; 3] = [5, 10, 14];
/// Kills of the same objective type this close are one fight (voidgrubs).
const SAME_OBJECTIVE_MS: i64 = 90_000;
const CATEGORIES: [&str; 10] = [
    "positioning",
    "map_awareness",
    "vision",
    "wave_management",
    "trading",
    "objective_setup",
    "recall_timing",
    "teamfight",
    "macro",
    "other",
];

#[derive(Debug, Clone)]
pub struct VisionOptions {
    pub max_frames: u32,
    pub max_tokens: u32,
    pub monthly_token_limit: Option<u64>,
}

impl VisionOptions {
    pub fn from_settings(s: &rc_store::settings::AppSettings) -> Self {
        Self {
            max_frames: s.vision.max_frames,
            max_tokens: s.vision.max_tokens,
            monthly_token_limit: s.ai.monthly_token_limit,
        }
    }
}

/// The moments of a game to look at: the player's deaths, objective fights
/// and laning checkpoints.
pub fn anchors(l: &Loaded) -> Result<Vec<Anchor>, CoachError> {
    let me = l.subject()?;
    let Some(tl) = &l.timeline else { return Ok(vec![]) };
    let mut out = Vec::new();
    for (i, d) in deaths_of(&l.game, tl, me).iter().enumerate() {
        // Early deaths and deaths that gave an objective teach the most.
        let priority = u8::from(!(d.before_14 || d.enemy_objective.is_some()));
        out.push(Anchor {
            id: format!("death_{}", i + 1),
            kind: AnchorKind::Death,
            t_ms: (d.t_s * 1000.0).round() as i64,
            priority,
        });
    }
    let mut last: Vec<(&str, i64)> = Vec::new();
    let mut n = 0;
    for e in tl.info.frames.iter().flat_map(|f| f.events.iter()) {
        let Some(kind) =
            e.monster_type.as_deref().filter(|t| e.kind == "ELITE_MONSTER_KILL" && OBJECTIVE_MONSTERS.contains(t))
        else {
            continue;
        };
        if last.iter().any(|(k, t)| *k == kind && e.timestamp - t < SAME_OBJECTIVE_MS) {
            continue;
        }
        last.push((kind, e.timestamp));
        n += 1;
        out.push(Anchor { id: format!("objective_{n}"), kind: AnchorKind::Objective, t_ms: e.timestamp, priority: 2 });
    }
    let end_ms = l.summary.duration_s * 1000;
    for m in CHECKPOINT_MIN {
        if m * 60_000 < end_ms - 60_000 {
            out.push(Anchor {
                id: format!("checkpoint_{m}"),
                kind: AnchorKind::Checkpoint,
                t_ms: m * 60_000,
                priority: 3,
            });
        }
    }
    Ok(out)
}

fn kind_label(k: AnchorKind) -> &'static str {
    match k {
        AnchorKind::Death => "death",
        AnchorKind::Objective => "objective",
        AnchorKind::Checkpoint => "checkpoint",
    }
}

fn ago(moment_ms: i64, frame_ms: i64) -> String {
    let d = (moment_ms - frame_ms) / 1000;
    match d {
        0 => "at the moment".into(),
        d if d > 0 => format!("{d} s before"),
        d => format!("{} s after", -d),
    }
}

/// Facts for the chosen moments (no images), plus each other player's
/// Riot ID name with their champion (to scrub from the reply).
fn moment_facts(l: &Loaded, moments: &[Moment]) -> Result<(Value, Vec<(String, String)>), CoachError> {
    let info = &l.game.info;
    let me = l.subject()?;
    let opp = lane_opponent(info, me);
    let champ_of = |pid: Option<i32>| {
        pid.filter(|k| (1..=10).contains(k))
            .and_then(|id| info.participants.iter().find(|p| p.participant_id == id))
            .map(|p| p.champion_name.clone())
    };
    let tl = l.timeline.as_ref();
    let deaths = tl.map(|tl| deaths_of(&l.game, tl, me)).unwrap_or_default();
    let objectives: Vec<(i64, &str, Option<i32>)> = tl
        .map(|tl| {
            tl.info
                .frames
                .iter()
                .flat_map(|f| f.events.iter())
                .filter(|e| e.kind == "ELITE_MONSTER_KILL")
                .filter_map(|e| Some((e.timestamp, e.monster_type.as_deref()?, e.killer_team_id)))
                .collect()
        })
        .unwrap_or_default();
    let list: Vec<Value> = moments
        .iter()
        .map(|m| {
            let a = &m.anchor;
            let context = match a.kind {
                AnchorKind::Death => {
                    let idx: usize = a.id.trim_start_matches("death_").parse().unwrap_or(0);
                    deaths.get(idx.wrapping_sub(1)).map_or(Value::Null, |d| {
                        json!({
                            "killer_champion": champ_of(d.killer_id),
                            "enemies_involved": d.enemies_involved,
                            "map_area": d.zone.label(),
                            "in_enemy_half": d.in_enemy_half,
                            "gold_vs_lane_opponent": d.gold_vs_opponent,
                            "unspent_gold": d.unspent_gold,
                            "enemy_objective_within_60s": d.enemy_objective,
                        })
                    })
                }
                AnchorKind::Objective => {
                    objectives.iter().find(|(t, _, _)| *t == a.t_ms).map_or(Value::Null, |(_, kind, team)| {
                        json!({
                            "objective": objective_label(kind),
                            "taken_by": if *team == Some(me.team_id) { "your team" } else { "the enemy team" },
                        })
                    })
                }
                AnchorKind::Checkpoint => {
                    let f = tl.and_then(|tl| frame_before(tl, a.t_ms + 1));
                    let mine = f.and_then(|f| f.participant(me.participant_id));
                    let theirs = opp.and_then(|o| f.and_then(|f| f.participant(o.participant_id)));
                    json!({
                        "cs": mine.map(|p| p.cs()),
                        "level": mine.map(|p| p.level),
                        "gold_vs_lane_opponent": mine.zip(theirs).map(|(a, b)| a.total_gold - b.total_gold),
                    })
                }
            };
            json!({
                "id": a.id,
                "kind": a.kind,
                "time": clock(a.t_ms as f64 / 1000.0),
                "frames": m.frames.iter().map(|&t| clock(t as f64 / 1000.0)).collect::<Vec<_>>(),
                "context": context,
            })
        })
        .collect();
    let s = &l.summary;
    let facts = json!({
        "game": {
            "match_id": s.match_id,
            "champion": s.champion_name,
            "role": s.team_position,
            "side": if me.team_id == 100 { "blue (base bottom-left)" } else { "red (base top-right)" },
            "lane_opponent_champion": opp.map(|o| o.champion_name.clone()),
            "result": if s.win { "win" } else { "loss" },
            "duration": clock(s.duration_s as f64),
            "kills": s.kills,
            "deaths": s.deaths,
            "assists": s.assists,
        },
        "moments": list,
        "notes": [
            "Frame times are estimated from the game clock and can be off by a second or two.",
            "Timeline positions are sampled once a minute; map areas are approximate.",
        ],
    });
    let names = info
        .participants
        .iter()
        .filter(|p| p.participant_id != me.participant_id)
        .filter_map(|p| Some((p.riot_id_game_name.clone()?, p.champion_name.clone())))
        .filter(|(n, _)| n.chars().count() >= 3)
        .collect();
    Ok((facts, names))
}

/// Replaces other players' names (case-insensitive) with their champion in
/// every string of the reply.
pub fn scrub_names(v: &mut Value, names: &[(String, String)]) {
    match v {
        Value::String(s) => {
            for (name, champ) in names {
                let lower = s.to_lowercase();
                let needle = name.to_lowercase();
                if needle.is_empty() || !lower.contains(&needle) {
                    continue;
                }
                let mut out = String::with_capacity(s.len());
                let mut rest = s.as_str();
                let mut rest_lower = lower.as_str();
                while let Some(i) = rest_lower.find(&needle) {
                    // Lowercasing can change byte lengths; fall back to an exact match then.
                    if rest.len() != rest_lower.len() {
                        out.push_str(&rest.replace(name.as_str(), champ));
                        rest = "";
                        break;
                    }
                    out.push_str(&rest[..i]);
                    out.push_str(champ);
                    rest = &rest[i + needle.len()..];
                    rest_lower = &rest_lower[i + needle.len()..];
                }
                out.push_str(rest);
                *s = out;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| scrub_names(x, names)),
        Value::Object(o) => o.values_mut().for_each(|x| scrub_names(x, names)),
        _ => {}
    }
}

/// Copies each reviewed moment's time and kind from the facts (the app
/// shows them; the model doesn't have to repeat them).
fn add_moment_times(report: &mut Value, facts: &Value) {
    let Some(fs) = facts["moments"].as_array() else { return };
    for m in report["moments"].as_array_mut().into_iter().flatten() {
        if let Some(f) = fs.iter().find(|f| f["id"] == m["id"]) {
            m["time"] = f["time"].clone();
            m["kind"] = f["kind"].clone();
        }
    }
}

fn text(v: &Value, key: &str) -> bool {
    v[key].as_str().is_some_and(|s| !s.trim().is_empty())
}

/// Checks the reply's shape and that every moment id exists. Unknown
/// categories become `other` (fixed in place).
pub fn validate_vision(report: &mut Value, facts: &Value) -> Validation {
    let mut v = Validation::default();
    let ids: Vec<&str> =
        facts["moments"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect()).unwrap_or_default();
    if !report.is_object() {
        v.structural.push("The reply must be one JSON object.".into());
        return v;
    }
    if !text(report, "summary") {
        v.structural.push("`summary` must be a non-empty string.".into());
    }
    let mut seen = Vec::new();
    match report["moments"].as_array_mut() {
        Some(ms) if !ms.is_empty() => {
            for (i, m) in ms.iter_mut().enumerate() {
                let id = m["id"].as_str().unwrap_or_default().to_owned();
                if !ids.contains(&id.as_str()) {
                    v.structural.push(format!("moments[{i}].id {id:?} is not a moment id from the facts."));
                } else if seen.contains(&id) {
                    v.structural.push(format!("moment {id} appears twice."));
                }
                seen.push(id);
                for key in ["title", "seen", "lesson"] {
                    if !text(m, key) {
                        v.structural.push(format!("moments[{i}].{key} must be a non-empty string."));
                    }
                }
                if !m["category"].as_str().is_some_and(|c| CATEGORIES.contains(&c)) {
                    m["category"] = json!("other");
                }
                if !m["severity"].as_str().is_some_and(|s| ["low", "medium", "high"].contains(&s)) {
                    m["severity"] = json!("medium");
                }
                if !m["clear"].is_boolean() {
                    m["clear"] = json!(true);
                }
            }
        }
        _ => v.structural.push("`moments` must list at least one reviewed moment.".into()),
    }
    if let Some(themes) = report["themes"].as_array() {
        for (i, t) in themes.iter().enumerate() {
            if !text(t, "title") {
                v.structural.push(format!("themes[{i}].title must be a non-empty string."));
            }
            for id in t["moments"].as_array().into_iter().flatten() {
                if !id.as_str().is_some_and(|s| ids.contains(&s)) {
                    v.structural.push(format!("themes[{i}] lists {id}, which is not a moment id."));
                }
            }
        }
    }
    if !report["confidence"].as_str().is_some_and(|c| ["low", "medium", "high"].contains(&c)) {
        v.structural.push("`confidence` must be low, medium or high.".into());
    }
    v
}

/// What a review of a game would send.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The moments' facts (sent as text).
    pub facts: Value,
    /// The frames picked for each moment.
    pub moments: Vec<Moment>,
    /// Other players' Riot ID names with their champion, to scrub.
    pub names: Vec<(String, String)>,
}

/// The moments a review would use, with their facts.
pub fn prepare(
    conn: &rc_store::rusqlite::Connection,
    account_id: i64,
    match_id: &str,
    frames: &[i64],
    max_frames: u32,
) -> Result<Prepared, CoachError> {
    let l = load(conn, account_id, match_id)?;
    if l.timeline.is_none() {
        return Err(CoachError::Invalid("the game's timeline isn't downloaded yet".into()));
    }
    let moments = rc_vision::select(frames, &anchors(&l)?, max_frames as usize);
    let (facts, names) = moment_facts(&l, &moments)?;
    Ok(Prepared { facts, moments, names })
}

/// Writes the vision review for a game whose frames are in `store` under
/// `live_game_id`. The caller deletes the frames afterwards.
pub async fn vision_review(
    db: &Db,
    ai: &AiClient,
    store: &FrameStore,
    account_id: i64,
    match_id: &str,
    live_game_id: i64,
    opts: &VisionOptions,
) -> Result<Report, CoachError> {
    crate::review::check_budget(db, opts.monthly_token_limit).await?;
    let frames = store.list(live_game_id);
    if frames.is_empty() {
        return Err(CoachError::NotFound("screenshots for this game (they're deleted after a review)".into()));
    }
    let (id, max) = (match_id.to_owned(), opts.max_frames);
    let Prepared { facts, moments, names } = db.call(move |c| Ok(prepare(c, account_id, &id, &frames, max))).await??;
    if moments.is_empty() {
        return Err(CoachError::Invalid("no screenshots were taken near this game's key moments".into()));
    }
    let mut images = Vec::new();
    for m in &moments {
        let n = m.frames.len();
        for (i, &t) in m.frames.iter().enumerate() {
            let data = store.read(live_game_id, t).map_err(|e| CoachError::Invalid(e.to_string()))?;
            images.push(ImagePart {
                caption: format!(
                    "Moment {} ({} at {}), frame {} of {n}, game time {} ({}):",
                    m.anchor.id,
                    kind_label(m.anchor.kind),
                    clock(m.anchor.t_ms as f64 / 1000.0),
                    i + 1,
                    clock(t as f64 / 1000.0),
                    ago(m.anchor.t_ms, t),
                ),
                media_type: "image/jpeg",
                data,
            });
        }
    }
    let facts_text = serde_json::to_string_pretty(&facts).map_err(|e| CoachError::Invalid(e.to_string()))?;
    let mut request = CompletionRequest {
        system: VISION_PROMPT.to_owned(),
        user: format!(
            "Facts for the game to review:\n{facts_text}\n\nThe {} screenshots follow, each after its caption.",
            images.len()
        ),
        max_tokens: opts.max_tokens,
        json: true,
        images,
    };
    let (mut tokens_in, mut tokens_out) = (0i64, 0i64);
    let mut last: Option<(Value, Validation)> = None;
    for attempt in 1..=2 {
        let completion = ai.complete(&request).await?;
        tokens_in += completion.input_tokens.unwrap_or(0);
        tokens_out += completion.output_tokens.unwrap_or(0);
        let mut parsed = extract_json(&completion.text).unwrap_or(Value::Null);
        let v = validate_vision(&mut parsed, &facts);
        tracing::info!(attempt, structural = v.structural.len(), "vision review validated");
        let clean = v.structural.is_empty();
        if !clean {
            request.user = format!(
                "{}\n\nYour previous reply had problems:\n{}\nReply again with the corrected JSON object only.",
                request.user,
                v.feedback()
            );
        }
        last = Some((parsed, v));
        if clean {
            break;
        }
    }
    let Some((mut report, v)) = last else { return Err(CoachError::Invalid("no AI reply".into())) };
    if !v.structural.is_empty() {
        return Err(CoachError::Invalid(format!(
            "the AI reply didn't match the vision review format after 2 tries: {}",
            v.structural.join(" ")
        )));
    }
    add_moment_times(&mut report, &facts);
    scrub_names(&mut report, &names);
    let frames_sent: usize = moments.iter().map(|m| m.frames.len()).sum();
    let input_json = json!({"facts": facts, "frames_sent": frames_sent}).to_string();
    let (provider, model, output) = (ai.config().kind.as_str(), ai.config().model.clone(), report.to_string());
    let id = match_id.to_owned();
    let report_id = db
        .call(move |c| {
            reports::insert(
                c,
                &NewReport {
                    kind: VISION,
                    account_id,
                    match_id: Some(&id),
                    scope: None,
                    provider,
                    model: &model,
                    prompt_version: VISION_PROMPT_VERSION,
                    input_json: &input_json,
                    output_json: &output,
                    validation: "ok",
                    notes: None,
                    input_tokens: Some(tokens_in),
                    output_tokens: Some(tokens_out),
                },
            )
        })
        .await?;
    db.call(move |c| reports::get(c, report_id))
        .await?
        .ok_or_else(|| CoachError::NotFound("the report just saved".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_names_case_insensitively() {
        let mut v = json!({"a": "TheBigBoss ganked you; thebigboss again", "b": ["Bob was there"]});
        scrub_names(&mut v, &[("TheBigBoss".into(), "Lee Sin".into()), ("Bob".into(), "Ahri".into())]);
        assert_eq!(v["a"], "Lee Sin ganked you; Lee Sin again");
        assert_eq!(v["b"][0], "Ahri was there");
    }

    #[test]
    fn validates_and_repairs() {
        let facts = json!({"moments": [{"id": "death_1"}, {"id": "checkpoint_5"}]});
        let mut good = json!({
            "summary": "s",
            "moments": [{"id": "death_1", "title": "t", "seen": "x", "lesson": "y", "category": "weird"}],
            "themes": [{"title": "t", "moments": ["death_1"]}],
            "confidence": "high",
        });
        let v = validate_vision(&mut good, &facts);
        assert!(v.structural.is_empty(), "{:?}", v.structural);
        assert_eq!(good["moments"][0]["category"], "other");
        assert_eq!(good["moments"][0]["severity"], "medium");

        let mut bad = json!({
            "summary": "",
            "moments": [{"id": "death_9", "title": "t", "seen": "x", "lesson": ""}],
            "themes": [{"title": "t", "moments": ["nope"]}],
        });
        let v = validate_vision(&mut bad, &facts);
        assert_eq!(v.structural.len(), 5, "{:?}", v.structural);
        assert!(!validate_vision(&mut json!([]), &facts).structural.is_empty());
    }

    #[test]
    fn frame_offsets_read_naturally() {
        assert_eq!(ago(60_000, 54_000), "6 s before");
        assert_eq!(ago(60_000, 60_000), "at the moment");
        assert_eq!(ago(60_000, 61_000), "1 s after");
    }
}
