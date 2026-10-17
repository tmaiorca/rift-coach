//! The parts of Match-V5 match and timeline JSON the metrics need. Separate
//! from `rc-riot`'s ingestion DTOs: those stay minimal, these are richer.
//! Unknown fields are ignored and missing ones default.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Match {
    pub metadata: Metadata,
    pub info: MatchInfo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub match_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchInfo {
    #[serde(default)]
    pub game_duration: i64,
    #[serde(default)]
    pub game_end_timestamp: Option<i64>,
    #[serde(default)]
    pub queue_id: i32,
    #[serde(default)]
    pub map_id: i32,
    #[serde(default)]
    pub participants: Vec<Participant>,
}

impl MatchInfo {
    pub fn duration_s(&self) -> f64 {
        let d = if self.game_end_timestamp.is_some() { self.game_duration } else { self.game_duration / 1000 };
        d as f64
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub participant_id: i32,
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
    pub total_damage_dealt_to_champions: i64,
    #[serde(default)]
    pub total_damage_taken: i64,
    #[serde(default)]
    pub vision_score: i32,
    #[serde(default)]
    pub wards_placed: i32,
    #[serde(default)]
    pub wards_killed: i32,
    #[serde(default)]
    pub detector_wards_placed: i32,
    #[serde(default)]
    pub total_time_spent_dead: i64,
    #[serde(default)]
    pub item0: i32,
    #[serde(default)]
    pub item1: i32,
    #[serde(default)]
    pub item2: i32,
    #[serde(default)]
    pub item3: i32,
    #[serde(default)]
    pub item4: i32,
    #[serde(default)]
    pub item5: i32,
    #[serde(default)]
    pub item6: i32,
    #[serde(default)]
    pub summoner1_id: i32,
    #[serde(default)]
    pub summoner2_id: i32,
}

impl Participant {
    pub fn cs(&self) -> i32 {
        self.total_minions_killed + self.neutral_minions_killed
    }

    pub fn items(&self) -> [i32; 7] {
        [self.item0, self.item1, self.item2, self.item3, self.item4, self.item5, self.item6]
    }

    pub fn role(&self) -> Option<&str> {
        (!self.team_position.is_empty()).then_some(self.team_position.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Timeline {
    pub info: TimelineInfo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineInfo {
    #[serde(default)]
    pub frame_interval: i64,
    #[serde(default)]
    pub frames: Vec<Frame>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    #[serde(default)]
    pub timestamp: i64,
    /// Keyed "1".."10".
    #[serde(default)]
    pub participant_frames: HashMap<String, ParticipantFrame>,
    #[serde(default)]
    pub events: Vec<Event>,
}

impl Frame {
    pub fn participant(&self, id: i32) -> Option<&ParticipantFrame> {
        self.participant_frames.get(&id.to_string())
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, serde::Serialize, PartialEq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantFrame {
    #[serde(default)]
    pub current_gold: i32,
    #[serde(default)]
    pub total_gold: i32,
    #[serde(default)]
    pub xp: i32,
    #[serde(default)]
    pub level: i32,
    #[serde(default)]
    pub minions_killed: i32,
    #[serde(default)]
    pub jungle_minions_killed: i32,
    #[serde(default)]
    pub position: Option<Position>,
}

impl ParticipantFrame {
    pub fn cs(&self) -> i32 {
        self.minions_killed + self.jungle_minions_killed
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub participant_id: Option<i32>,
    #[serde(default)]
    pub killer_id: Option<i32>,
    #[serde(default)]
    pub victim_id: Option<i32>,
    #[serde(default)]
    pub creator_id: Option<i32>,
    #[serde(default)]
    pub assisting_participant_ids: Vec<i32>,
    #[serde(default)]
    pub position: Option<Position>,
    #[serde(default)]
    pub item_id: Option<i32>,
    #[serde(default)]
    pub before_id: Option<i32>,
    #[serde(default)]
    pub after_id: Option<i32>,
    #[serde(default)]
    pub ward_type: Option<String>,
    #[serde(default)]
    pub monster_type: Option<String>,
    #[serde(default)]
    pub monster_sub_type: Option<String>,
    #[serde(default)]
    pub killer_team_id: Option<i32>,
    #[serde(default)]
    pub team_id: Option<i32>,
    #[serde(default)]
    pub building_type: Option<String>,
    #[serde(default)]
    pub tower_type: Option<String>,
    #[serde(default)]
    pub lane_type: Option<String>,
    #[serde(default)]
    pub bounty: Option<i32>,
    #[serde(default)]
    pub shutdown_bounty: Option<i32>,
}

/// Item prices and categories from Data Dragon `item.json`.
#[derive(Debug, Clone, Default)]
pub struct ItemCatalog {
    items: HashMap<i32, ItemInfo>,
}

#[derive(Debug, Clone)]
pub struct ItemInfo {
    pub name: String,
    pub total_gold: i32,
    /// A finished, non-boot, non-consumable item (builds into nothing and
    /// costs at least [`ItemCatalog::FINISHED_MIN_GOLD`]).
    pub completed: bool,
}

impl ItemCatalog {
    pub const FINISHED_MIN_GOLD: i32 = 2200;

    pub fn get(&self, id: i32) -> Option<&ItemInfo> {
        self.items.get(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn from_ddragon_json(json: &[u8]) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct File {
            data: HashMap<String, Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            #[serde(default)]
            name: String,
            #[serde(default)]
            into: Vec<String>,
            #[serde(default)]
            tags: Vec<String>,
            gold: Gold,
        }
        #[derive(Deserialize)]
        struct Gold {
            #[serde(default)]
            total: i32,
        }
        let file: File = serde_json::from_slice(json)?;
        let items = file
            .data
            .into_iter()
            .filter_map(|(id, it)| {
                let id = id.parse().ok()?;
                let excluded = it.tags.iter().any(|t| matches!(t.as_str(), "Boots" | "Consumable" | "Trinket"));
                let completed = it.into.is_empty() && !excluded && it.gold.total >= Self::FINISHED_MIN_GOLD;
                Some((id, ItemInfo { name: it.name, total_gold: it.gold.total, completed }))
            })
            .collect();
        Ok(Self { items })
    }
}
