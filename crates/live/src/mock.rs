//! A scripted game for tests and the `rc-cli mock-live` server: produces
//! `allgamedata` JSON for any game time, so the poller and live view can be
//! exercised without launching League.

use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct MockGame {
    pub riot_id: String,
    /// Display name and Data Dragon id.
    pub champion: (String, String),
    pub position: String,
    /// `CLASSIC`, `PRACTICETOOL`, `ARAM`, ...
    pub game_mode: String,
    /// The `GameEnd` event appears at this game time.
    pub duration_s: f64,
    pub win: bool,
    pub cs_per_min: f64,
}

impl Default for MockGame {
    fn default() -> Self {
        Self {
            riot_id: "TapRooted#BTC".into(),
            champion: ("LeBlanc".into(), "Leblanc".into()),
            position: "MIDDLE".into(),
            game_mode: "CLASSIC".into(),
            duration_s: 28.0 * 60.0,
            win: true,
            cs_per_min: 7.0,
        }
    }
}

/// From game time, inventory as (item id, count), gold spent by then.
type BuildStep = (f64, &'static [(i32, i32)], f64);

const BUILD: [BuildStep; 6] = [
    (0.0, &[(1056, 1), (2003, 2), (3340, 1)], 500.0),
    (400.0, &[(1056, 1), (1052, 1), (1001, 1), (3340, 1)], 1235.0),
    (720.0, &[(1056, 1), (6655, 1), (1001, 1), (3340, 1)], 3700.0),
    (960.0, &[(1056, 1), (6655, 1), (3020, 1), (3340, 1)], 4500.0),
    (1320.0, &[(6655, 1), (3020, 1), (4645, 1), (3340, 1)], 7400.0),
    (1680.0, &[(6655, 1), (3020, 1), (4645, 1), (3089, 1), (3340, 1)], 11_000.0),
];
const DEATHS: [f64; 3] = [300.0, 780.0, 1500.0];
const KILLS: [f64; 4] = [450.0, 900.0, 1200.0, 1400.0];

fn count_before(times: &[f64], t: f64) -> i32 {
    i32::try_from(times.iter().filter(|x| **x <= t).count()).unwrap_or(0)
}

impl MockGame {
    pub fn cs_at(&self, t: f64) -> i32 {
        // Minions arrive in lane around 1:30.
        (((t - 90.0).max(0.0) / 60.0) * self.cs_per_min).floor() as i32
    }

    pub fn all_game_data(&self, t: f64) -> Value {
        let t = t.max(0.0);
        let (_, inventory, spent) = BUILD.iter().rev().find(|(from, _, _)| *from <= t).copied().unwrap_or(BUILD[0]);
        let earned = 500.0 + t / 60.0 * 380.0;
        let gold = (earned - spent).max(0.0);
        let level = (1 + (t / 95.0) as i32).min(18);
        let assists = if t > 600.0 { ((t - 600.0) / 240.0) as i32 } else { 0 };
        let items: Vec<Value> = inventory
            .iter()
            .enumerate()
            .map(|(slot, (id, count))| json!({"itemID": id, "count": count, "slot": slot, "displayName": format!("Item {id}"), "price": 0}))
            .collect();
        let me = json!({
            "championName": self.champion.0,
            "rawChampionName": format!("game_character_displayname_{}", self.champion.1),
            "riotId": self.riot_id,
            "summonerName": self.riot_id,
            "position": self.position,
            "level": level,
            "items": items,
            "isDead": DEATHS.iter().any(|d| t >= *d && t < d + 20.0),
            "team": "ORDER",
            "scores": {
                "kills": count_before(&KILLS, t),
                "deaths": count_before(&DEATHS, t),
                "assists": assists,
                "creepScore": self.cs_at(t),
                "wardScore": (t / 60.0 * 0.9).floor(),
            },
        });
        // Other players exist in the real feed; the parser must ignore them.
        let mut players = vec![me];
        for i in 2..=10 {
            players.push(json!({
                "championName": "Annie",
                "rawChampionName": "game_character_displayname_Annie",
                "riotId": format!("Player{i}#NA1"),
                "summonerName": format!("Player{i}#NA1"),
                "position": "",
                "level": level,
                "items": [],
                "team": if i <= 5 { "ORDER" } else { "CHAOS" },
                "scores": {"kills": 1, "deaths": 1, "assists": 1, "creepScore": 40, "wardScore": 3.0},
            }));
        }
        let mut events = vec![json!({"EventID": 0, "EventName": "GameStart", "EventTime": 0.0})];
        if t >= self.duration_s {
            events.push(json!({
                "EventID": 1,
                "EventName": "GameEnd",
                "EventTime": self.duration_s,
                "Result": if self.win { "Win" } else { "Lose" },
            }));
        }
        json!({
            "activePlayer": {
                "currentGold": gold,
                "level": level,
                "riotId": self.riot_id,
                "summonerName": self.riot_id,
                "championStats": {},
                "fullRunes": {},
            },
            "allPlayers": players,
            "events": {"Events": events},
            "gameData": {
                "gameMode": self.game_mode,
                "gameTime": t.min(self.duration_s),
                "mapName": "Map11",
                "mapNumber": 11,
                "mapTerrain": "Default",
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AllGameData;

    #[test]
    fn parses_own_numbers_only() {
        let g = MockGame::default();
        let data: AllGameData = serde_json::from_value(g.all_game_data(800.0)).expect("parse");
        let s = data.own().expect("own");
        assert_eq!(s.champion_name, "LeBlanc");
        assert_eq!(s.position, "MIDDLE");
        assert_eq!(s.scores.deaths, 2);
        assert_eq!(s.scores.kills, 1);
        assert_eq!(s.scores.creep_score, g.cs_at(800.0));
        assert!(s.items.iter().any(|i| i.item_id == 6655));
        assert!(s.game_end.is_none());
        let end: AllGameData = serde_json::from_value(g.all_game_data(g.duration_s + 5.0)).expect("parse");
        assert_eq!(end.own().expect("own").game_end.as_deref(), Some("Win"));
    }
}
