//! Rough Summoner's Rift regions, relative to the player's team. Built from
//! straight-line bands over the map coordinates (blue base bottom-left, red
//! top-right), so every label is approximate.

use serde::Serialize;

use crate::detail::MAP_MAX_X;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    OwnBase,
    EnemyBase,
    TopLane,
    MidLane,
    BotLane,
    TopRiver,
    BotRiver,
    OwnJungle,
    EnemyJungle,
}

impl Zone {
    pub const ALL: [Zone; 9] = [
        Zone::OwnBase,
        Zone::EnemyBase,
        Zone::TopLane,
        Zone::MidLane,
        Zone::BotLane,
        Zone::TopRiver,
        Zone::BotRiver,
        Zone::OwnJungle,
        Zone::EnemyJungle,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Zone::OwnBase => "own_base",
            Zone::EnemyBase => "enemy_base",
            Zone::TopLane => "top_lane",
            Zone::MidLane => "mid_lane",
            Zone::BotLane => "bot_lane",
            Zone::TopRiver => "top_river",
            Zone::BotRiver => "bot_river",
            Zone::OwnJungle => "own_jungle",
            Zone::EnemyJungle => "enemy_jungle",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Zone::OwnBase => "your base",
            Zone::EnemyBase => "the enemy base",
            Zone::TopLane => "top lane",
            Zone::MidLane => "mid lane",
            Zone::BotLane => "bot lane",
            Zone::TopRiver => "top-side river",
            Zone::BotRiver => "bot-side river",
            Zone::OwnJungle => "your jungle",
            Zone::EnemyJungle => "the enemy jungle",
        }
    }

    pub fn parse(s: &str) -> Option<Zone> {
        Zone::ALL.into_iter().find(|z| z.as_str() == s)
    }
}

/// Lanes run along the map edges and the main diagonal.
const LANE_EDGE: i32 = 2000;
const MID_HALF_WIDTH: i32 = 1100;
const RIVER_HALF_WIDTH: i32 = 1000;
const BASE: i32 = 4300;
/// Bases are a corner square cut by a diagonal (covers the inhibitors).
const BASE_DIAGONAL: i32 = 7000;

pub fn zone_of(team_id: i32, x: i32, y: i32) -> Zone {
    let far = MAP_MAX_X;
    let blue_base = x < BASE && y < BASE && x + y < BASE_DIAGONAL;
    let red_base = x > far - BASE && y > far - BASE && x + y > 2 * far - BASE_DIAGONAL;
    if blue_base || red_base {
        return if blue_base == (team_id == 100) { Zone::OwnBase } else { Zone::EnemyBase };
    }
    // Top lane hugs the left and top edges, bot lane the bottom and right.
    if (x < LANE_EDGE || y > far - LANE_EDGE) && y > x {
        return Zone::TopLane;
    }
    if (y < LANE_EDGE || x > far - LANE_EDGE) && x > y {
        return Zone::BotLane;
    }
    if (x - y).abs() < MID_HALF_WIDTH {
        return Zone::MidLane;
    }
    let s = x + y;
    if (s - far).abs() < RIVER_HALF_WIDTH {
        return if y > x { Zone::TopRiver } else { Zone::BotRiver };
    }
    let blue_side = s < far;
    if blue_side == (team_id == 100) { Zone::OwnJungle } else { Zone::EnemyJungle }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landmarks() {
        // Blue fountain / red fountain.
        assert_eq!(zone_of(100, 600, 600), Zone::OwnBase);
        assert_eq!(zone_of(200, 600, 600), Zone::EnemyBase);
        assert_eq!(zone_of(100, 14200, 14200), Zone::EnemyBase);
        // Mid inhibitors are inside the bases.
        assert_eq!(zone_of(100, 3203, 3208), Zone::OwnBase);
        assert_eq!(zone_of(100, 11598, 11667), Zone::EnemyBase);
        // Outer turrets.
        assert_eq!(zone_of(100, 981, 10441), Zone::TopLane);
        assert_eq!(zone_of(100, 4318, 13875), Zone::TopLane);
        assert_eq!(zone_of(100, 10504, 1029), Zone::BotLane);
        assert_eq!(zone_of(100, 13866, 4505), Zone::BotLane);
        assert_eq!(zone_of(100, 5846, 6396), Zone::MidLane);
        assert_eq!(zone_of(100, 8955, 8510), Zone::MidLane);
        // Dragon pit and baron pit.
        assert_eq!(zone_of(100, 9866, 4414), Zone::BotRiver);
        assert_eq!(zone_of(100, 5007, 10471), Zone::TopRiver);
        // Blue buff (blue side) and red side's blue buff.
        assert_eq!(zone_of(100, 3800, 7900), Zone::OwnJungle);
        assert_eq!(zone_of(200, 3800, 7900), Zone::EnemyJungle);
        assert_eq!(zone_of(100, 11000, 7000), Zone::EnemyJungle);
    }
}
