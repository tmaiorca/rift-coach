use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Platform routing value (`na1.api.riotgames.com`). Used by League-V4,
/// Spectator-V5, Champion-Mastery-V4, Summoner-V4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Br1,
    Eun1,
    Euw1,
    Jp1,
    Kr,
    La1,
    La2,
    Me1,
    Na1,
    Oc1,
    Ru,
    Sg2,
    Tr1,
    Tw2,
    Vn2,
}

/// Regional routing value (`americas.api.riotgames.com`). Used by Match-V5 and
/// Account-V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Americas,
    Europe,
    Asia,
    Sea,
}

impl Platform {
    pub const ALL: [Platform; 15] = [
        Self::Br1,
        Self::Eun1,
        Self::Euw1,
        Self::Jp1,
        Self::Kr,
        Self::La1,
        Self::La2,
        Self::Me1,
        Self::Na1,
        Self::Oc1,
        Self::Ru,
        Self::Sg2,
        Self::Tr1,
        Self::Tw2,
        Self::Vn2,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Br1 => "br1",
            Self::Eun1 => "eun1",
            Self::Euw1 => "euw1",
            Self::Jp1 => "jp1",
            Self::Kr => "kr",
            Self::La1 => "la1",
            Self::La2 => "la2",
            Self::Me1 => "me1",
            Self::Na1 => "na1",
            Self::Oc1 => "oc1",
            Self::Ru => "ru",
            Self::Sg2 => "sg2",
            Self::Tr1 => "tr1",
            Self::Tw2 => "tw2",
            Self::Vn2 => "vn2",
        }
    }

    /// Region used for Match-V5.
    pub fn match_region(self) -> Region {
        match self {
            Self::Na1 | Self::Br1 | Self::La1 | Self::La2 => Region::Americas,
            Self::Euw1 | Self::Eun1 | Self::Tr1 | Self::Ru | Self::Me1 => Region::Europe,
            Self::Kr | Self::Jp1 => Region::Asia,
            Self::Oc1 | Self::Sg2 | Self::Tw2 | Self::Vn2 => Region::Sea,
        }
    }

    /// Region used for Account-V1 (which has no SEA cluster; any region works,
    /// the nearest is fastest).
    pub fn account_region(self) -> Region {
        match self.match_region() {
            Region::Sea => Region::Asia,
            r => r,
        }
    }

    /// Platform from a match ID prefix, e.g. `NA1_5123456789`.
    pub fn from_match_id(match_id: &str) -> Option<Self> {
        match_id.split_once('_').and_then(|(p, _)| p.parse().ok())
    }
}

impl Region {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Americas => "americas",
            Self::Europe => "europe",
            Self::Asia => "asia",
            Self::Sea => "sea",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        Self::ALL.into_iter().find(|p| p.as_str() == lower).ok_or_else(|| format!("unknown platform '{s}'"))
    }
}

/// A Riot ID (`gameName#tagLine`). Comparison is case-insensitive, as in-game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiotId {
    pub game_name: String,
    pub tag_line: String,
}

impl RiotId {
    pub fn new(game_name: impl Into<String>, tag_line: impl Into<String>) -> Self {
        Self { game_name: game_name.into(), tag_line: tag_line.into() }
    }

    pub fn matches(&self, game_name: &str, tag_line: &str) -> bool {
        self.game_name.to_lowercase() == game_name.to_lowercase()
            && self.tag_line.to_lowercase() == tag_line.to_lowercase()
    }
}

impl FromStr for RiotId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, tag) = s.rsplit_once('#').ok_or_else(|| format!("'{s}' is not a Riot ID (expected Name#TAG)"))?;
        let (name, tag) = (name.trim(), tag.trim());
        if name.is_empty() || tag.is_empty() {
            return Err(format!("'{s}' is not a Riot ID (expected Name#TAG)"));
        }
        Ok(Self::new(name, tag))
    }
}

impl fmt::Display for RiotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.game_name, self.tag_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_platforms_and_match_ids() {
        assert_eq!("NA1".parse::<Platform>(), Ok(Platform::Na1));
        assert_eq!(Platform::from_match_id("NA1_5123456789"), Some(Platform::Na1));
        assert_eq!(Platform::from_match_id("garbage"), None);
        assert_eq!(Platform::Na1.match_region(), Region::Americas);
        assert_eq!(Platform::Oc1.match_region(), Region::Sea);
        assert_eq!(Platform::Oc1.account_region(), Region::Asia);
    }

    #[test]
    fn parses_riot_ids() {
        let id: RiotId = "TapRooted#BTC".parse().expect("valid");
        assert_eq!(id.game_name, "TapRooted");
        assert_eq!(id.tag_line, "BTC");
        assert!(id.matches("taprooted", "btc"));
        // Game names may contain spaces; the tag is after the last '#'.
        let spaced: RiotId = "Some Name #NA1".parse().expect("valid");
        assert_eq!(spaced.game_name, "Some Name");
        assert!("NoTag".parse::<RiotId>().is_err());
        assert!("#TAG".parse::<RiotId>().is_err());
    }
}
