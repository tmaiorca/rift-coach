//! Partial Live Client Data API types (`/liveclientdata/allgamedata`).
//! Every field defaults, so a client update that drops or renames one
//! degrades a number instead of breaking the poller.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AllGameData {
    pub active_player: ActivePlayer,
    pub all_players: Vec<Player>,
    pub events: Events,
    pub game_data: GameData,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ActivePlayer {
    pub current_gold: f64,
    pub level: i32,
    /// `Name#TAG` (newer clients).
    pub riot_id: String,
    /// Older clients put the Riot ID or summoner name here.
    pub summoner_name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Player {
    pub champion_name: String,
    /// `game_character_displayname_<DataDragonId>`: language-independent.
    pub raw_champion_name: String,
    pub riot_id: String,
    pub summoner_name: String,
    /// `TOP`, `JUNGLE`, `MIDDLE`, `BOTTOM`, `UTILITY`, or empty outside ranked/draft.
    pub position: String,
    pub level: i32,
    pub items: Vec<LiveItem>,
    pub scores: Scores,
    pub is_dead: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LiveItem {
    #[serde(rename = "itemID")]
    pub item_id: i32,
    pub count: i32,
    pub slot: i32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Scores {
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub creep_score: i32,
    pub ward_score: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Events {
    #[serde(rename = "Events")]
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Event {
    #[serde(rename = "EventName")]
    pub name: String,
    #[serde(rename = "EventTime")]
    pub time: f64,
    /// `Win` / `Lose` on `GameEnd`.
    #[serde(rename = "Result")]
    pub result: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GameData {
    pub game_mode: String,
    pub game_time: f64,
    pub map_number: i32,
}

/// The player's own numbers from one poll. Nothing about other players is
/// kept: they're dropped while parsing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OwnSnapshot {
    pub riot_id: String,
    /// Display name as the client shows it (may be localized).
    pub champion_name: String,
    /// Data Dragon id (e.g. `Leblanc`), when the client reports it.
    pub champion_ddragon_id: Option<String>,
    pub position: String,
    pub game_mode: String,
    pub map_number: i32,
    pub game_time_s: f64,
    pub level: i32,
    pub current_gold: f64,
    pub scores: Scores,
    pub items: Vec<LiveItem>,
    pub is_dead: bool,
    /// `Some("Win" | "Lose")` once the `GameEnd` event is in the feed.
    pub game_end: Option<String>,
}

impl AllGameData {
    /// The player's own entry, matched on Riot ID (falling back to the
    /// summoner name for older clients).
    pub fn own(&self) -> Option<OwnSnapshot> {
        let me = &self.active_player;
        let id = if me.riot_id.is_empty() { &me.summoner_name } else { &me.riot_id };
        if id.is_empty() {
            return None;
        }
        let p = self.all_players.iter().find(|p| {
            (!p.riot_id.is_empty() && p.riot_id.eq_ignore_ascii_case(id))
                || (!p.summoner_name.is_empty() && p.summoner_name.eq_ignore_ascii_case(id))
        })?;
        let game_end =
            self.events.events.iter().find(|e| e.name == "GameEnd").map(|e| e.result.clone().unwrap_or_default());
        Some(OwnSnapshot {
            riot_id: id.clone(),
            champion_name: p.champion_name.clone(),
            champion_ddragon_id: p
                .raw_champion_name
                .strip_prefix("game_character_displayname_")
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            position: p.position.clone(),
            game_mode: self.game_data.game_mode.clone(),
            map_number: self.game_data.map_number,
            game_time_s: self.game_data.game_time,
            level: if me.level > 0 { me.level } else { p.level },
            current_gold: me.current_gold,
            scores: p.scores.clone(),
            items: p.items.clone(),
            is_dead: p.is_dead,
            game_end,
        })
    }
}
