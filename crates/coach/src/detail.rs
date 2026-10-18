//! Everything the match detail screen shows, assembled from stored data.

use rc_common::queues::{is_standard_rift, queue_name};
use rc_metrics::detail::{DeathContext, MapEvent, Series, deaths_of, map_events, series};
use rc_metrics::model::{Match, Participant, Timeline};
use rc_metrics::{parse_match, parse_timeline};
use rc_store::matches::{self, MatchSummary, RawKind};
use rc_store::reports::{self, Report};
use rc_store::rusqlite::Connection;
use serde::Serialize;

use crate::CoachError;
use crate::analysis::{MetricComparison, compare_metrics};
use crate::patterns::{MatchPattern, match_patterns};

pub const POST_GAME: &str = "post_game";

#[derive(Debug, Clone, Serialize)]
pub struct ScoreRow {
    pub participant_id: i32,
    pub team_id: i32,
    pub riot_id: Option<String>,
    pub champion_id: i32,
    pub champion_name: String,
    pub team_position: Option<String>,
    pub win: bool,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub level: i32,
    pub cs: i32,
    pub gold: i32,
    pub damage: i64,
    pub vision_score: i32,
    pub items: [i32; 7],
    pub summoner_spells: [i32; 2],
    pub is_subject: bool,
}

fn score_row(p: &Participant, subject: i32) -> ScoreRow {
    let riot_id = match (&p.riot_id_game_name, &p.riot_id_tagline) {
        (Some(n), Some(t)) if !n.is_empty() => Some(format!("{n}#{t}")),
        _ => None,
    };
    ScoreRow {
        participant_id: p.participant_id,
        team_id: p.team_id,
        riot_id,
        champion_id: p.champion_id,
        champion_name: p.champion_name.clone(),
        team_position: p.role().map(str::to_owned),
        win: p.win,
        kills: p.kills,
        deaths: p.deaths,
        assists: p.assists,
        level: p.champ_level,
        cs: p.cs(),
        gold: p.gold_earned,
        damage: p.total_damage_dealt_to_champions,
        vision_score: p.vision_score,
        items: p.items(),
        summoner_spells: [p.summoner1_id, p.summoner2_id],
        is_subject: p.participant_id == subject,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchDetail {
    pub summary: MatchSummary,
    pub queue_name: &'static str,
    pub is_standard_rift: bool,
    pub participant_id: i32,
    pub opponent_id: Option<i32>,
    pub scoreboard: Vec<ScoreRow>,
    pub comparison: MetricComparison,
    pub series: Option<Series>,
    pub events: Vec<MapEvent>,
    pub deaths: Vec<DeathContext>,
    /// Rule patterns in this game, marked when they recur in your games.
    pub patterns: Vec<MatchPattern>,
    pub report: Option<Report>,
}

/// Parsed raw data for a match the account played.
pub struct Loaded {
    pub summary: MatchSummary,
    pub participant_id: i32,
    pub game: Match,
    pub timeline: Option<Timeline>,
}

impl Loaded {
    pub fn subject(&self) -> Result<&Participant, CoachError> {
        self.game
            .info
            .participants
            .iter()
            .find(|p| p.participant_id == self.participant_id)
            .ok_or_else(|| CoachError::Invalid("player not found in match data".into()))
    }
}

pub fn load(conn: &Connection, account_id: i64, match_id: &str) -> Result<Loaded, CoachError> {
    let (summary, participant_id) = matches::account_match(conn, account_id, match_id)?
        .ok_or_else(|| CoachError::NotFound(format!("match {match_id} for this account")))?;
    let raw = matches::get_raw(conn, match_id, RawKind::Match)?
        .ok_or_else(|| CoachError::NotFound(format!("raw data for {match_id}")))?;
    let game = parse_match(&raw)?;
    let timeline = matches::get_raw(conn, match_id, RawKind::Timeline)?.map(|t| parse_timeline(&t)).transpose()?;
    Ok(Loaded { summary, participant_id, game, timeline })
}

pub fn match_detail(
    conn: &Connection,
    account_id: i64,
    match_id: &str,
    baseline_games: u32,
    session_gap_min: u32,
) -> Result<MatchDetail, CoachError> {
    let l = load(conn, account_id, match_id)?;
    let me = l.subject()?;
    let opponent_id = rc_metrics::compute::lane_opponent(&l.game.info, me).map(|o| o.participant_id);
    let rift = is_standard_rift(l.summary.queue_id, l.summary.map_id) && !l.summary.is_remake;
    let comparison = compare_metrics(
        conn,
        account_id,
        match_id,
        l.participant_id,
        if rift { me.role() } else { None },
        &me.champion_name,
        l.summary.game_start_ms,
        baseline_games,
    )?;
    let (series, events, deaths) = match &l.timeline {
        Some(tl) => (Some(series(tl)), map_events(&l.game, tl), deaths_of(&l.game, tl, me)),
        None => (None, vec![], vec![]),
    };
    let patterns = if rift {
        match_patterns(conn, account_id, match_id, l.summary.game_start_ms, baseline_games, session_gap_min)?
    } else {
        vec![]
    };
    Ok(MatchDetail {
        queue_name: queue_name(l.summary.queue_id),
        is_standard_rift: rift,
        participant_id: l.participant_id,
        opponent_id,
        scoreboard: l.game.info.participants.iter().map(|p| score_row(p, l.participant_id)).collect(),
        comparison,
        series,
        events,
        deaths,
        patterns,
        report: reports::latest_for_match(conn, POST_GAME, account_id, match_id)?,
        summary: l.summary,
    })
}
