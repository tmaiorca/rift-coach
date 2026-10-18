//! Typed views of Riot responses. Match and timeline DTOs are deliberately
//! partial: the raw JSON is stored verbatim, and these structs only cover the
//! fields we index. Unknown fields are ignored and missing ones default, so
//! Riot adding or dropping fields doesn't break ingestion.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    pub puuid: String,
    pub game_name: Option<String>,
    pub tag_line: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeagueEntryDto {
    pub queue_type: String,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub rank: Option<String>,
    #[serde(default)]
    pub league_points: i32,
    #[serde(default)]
    pub wins: i32,
    #[serde(default)]
    pub losses: i32,
    #[serde(default)]
    pub puuid: Option<String>,
}

/// An apex ladder (Master and up).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeagueListDto {
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub entries: Vec<LeagueItemDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeagueItemDto {
    #[serde(default)]
    pub puuid: Option<String>,
    #[serde(default)]
    pub league_points: i32,
}

/// Spectator-V5 current game, reduced to what the pre-game brief may use:
/// champions and teams. Other players' identities are not read.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveGameDto {
    #[serde(default)]
    pub game_id: i64,
    #[serde(default)]
    pub game_queue_config_id: i32,
    #[serde(default)]
    pub map_id: i32,
    #[serde(default)]
    pub participants: Vec<ActiveParticipantDto>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveParticipantDto {
    #[serde(default)]
    pub champion_id: i32,
    #[serde(default)]
    pub team_id: i32,
    /// Only compared with the player's own PUUID to find their team.
    #[serde(default)]
    pub puuid: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChampionMasteryDto {
    pub champion_id: i64,
    #[serde(default)]
    pub champion_level: i32,
    #[serde(default)]
    pub champion_points: i64,
    #[serde(default)]
    pub last_play_time: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatchDto {
    pub metadata: MatchMetadataDto,
    pub info: MatchInfoDto,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchMetadataDto {
    pub match_id: String,
    #[serde(default)]
    pub participants: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchInfoDto {
    #[serde(default)]
    pub game_creation: i64,
    #[serde(default)]
    pub game_start_timestamp: Option<i64>,
    #[serde(default)]
    pub game_end_timestamp: Option<i64>,
    /// Seconds when `game_end_timestamp` is present, milliseconds otherwise
    /// (pre-11.20 matches). Use [`MatchInfoDto::duration_seconds`].
    #[serde(default)]
    pub game_duration: i64,
    #[serde(default)]
    pub game_mode: String,
    #[serde(default)]
    pub game_type: Option<String>,
    #[serde(default)]
    pub game_version: String,
    #[serde(default)]
    pub map_id: i32,
    #[serde(default)]
    pub queue_id: i32,
    #[serde(default)]
    pub platform_id: String,
    #[serde(default)]
    pub end_of_game_result: Option<String>,
    #[serde(default)]
    pub participants: Vec<ParticipantDto>,
}

impl MatchInfoDto {
    pub fn duration_seconds(&self) -> i64 {
        if self.game_end_timestamp.is_some() { self.game_duration } else { self.game_duration / 1000 }
    }

    pub fn start_ms(&self) -> i64 {
        self.game_start_timestamp.unwrap_or(self.game_creation)
    }

    /// Ended by an early-surrender vote (remake).
    pub fn is_remake(&self) -> bool {
        self.participants.iter().any(|p| p.game_ended_in_early_surrender)
    }
}

/// `"16.20.712.3456"` → `"16.20"`.
pub fn patch_of(game_version: &str) -> String {
    let mut parts = game_version.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) if !major.is_empty() && !minor.is_empty() => format!("{major}.{minor}"),
        _ => game_version.to_owned(),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantDto {
    pub participant_id: i32,
    pub puuid: String,
    #[serde(default)]
    pub riot_id_game_name: Option<String>,
    #[serde(default)]
    pub riot_id_tagline: Option<String>,
    #[serde(default)]
    pub team_id: i32,
    #[serde(default)]
    pub champion_id: i32,
    #[serde(default)]
    pub champion_name: String,
    #[serde(default)]
    pub team_position: String,
    #[serde(default)]
    pub individual_position: String,
    #[serde(default)]
    pub win: bool,
    #[serde(default)]
    pub kills: i32,
    #[serde(default)]
    pub deaths: i32,
    #[serde(default)]
    pub assists: i32,
    #[serde(default)]
    pub champ_level: i32,
    #[serde(default)]
    pub gold_earned: i32,
    #[serde(default)]
    pub total_minions_killed: i32,
    #[serde(default)]
    pub neutral_minions_killed: i32,
    #[serde(default)]
    pub vision_score: i32,
    #[serde(default)]
    pub game_ended_in_early_surrender: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimelineDto {
    pub metadata: MatchMetadataDto,
    pub info: TimelineInfoDto,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineInfoDto {
    #[serde(default)]
    pub frame_interval: i64,
    /// Kept opaque here; the metrics crate parses frames in full.
    #[serde(default)]
    pub frames: Vec<serde::de::IgnoredAny>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_extraction() {
        assert_eq!(patch_of("16.20.712.3456"), "16.20");
        assert_eq!(patch_of("14.1.555.1"), "14.1");
        assert_eq!(patch_of("weird"), "weird");
    }

    #[test]
    fn parses_minimal_match_and_handles_duration_units() {
        let json = r#"{
            "metadata": {"matchId": "NA1_1", "participants": ["p1"]},
            "info": {
                "gameCreation": 1000, "gameDuration": 1800, "gameEndTimestamp": 5,
                "gameMode": "CLASSIC", "gameVersion": "16.20.1.2", "mapId": 11, "queueId": 420,
                "someNewField": {"x": 1},
                "participants": [{"participantId": 1, "puuid": "p1", "win": true, "kills": 3}]
            }
        }"#;
        let m: MatchDto = serde_json::from_str(json).expect("parse");
        assert_eq!(m.info.duration_seconds(), 1800);
        assert_eq!(m.info.start_ms(), 1000);
        assert_eq!(m.info.participants[0].kills, 3);
        assert!(!m.info.is_remake());

        let old = r#"{"metadata": {"matchId": "NA1_2"}, "info": {"gameDuration": 1800000}}"#;
        let m: MatchDto = serde_json::from_str(old).expect("parse");
        assert_eq!(m.info.duration_seconds(), 1800);
    }
}
