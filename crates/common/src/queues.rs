//! Queue metadata. Source: https://static.developer.riotgames.com/docs/lol/queues.json

pub const SUMMONERS_RIFT_MAP: i32 = 11;

/// Standard 5v5 Summoner's Rift queues (draft, ranked, blind, swiftplay,
/// quickplay, clash).
pub const STANDARD_RIFT_QUEUES: [i32; 7] = [400, 420, 430, 440, 480, 490, 700];

/// Human-readable queue name for UI display.
pub fn queue_name(queue_id: i32) -> &'static str {
    match queue_id {
        400 => "Normal Draft",
        420 => "Ranked Solo/Duo",
        430 => "Normal Blind",
        440 => "Ranked Flex",
        450 => "ARAM",
        480 => "Swiftplay",
        490 => "Quickplay",
        700 => "Clash",
        720 => "ARAM Clash",
        900 | 1900 => "URF",
        1700 | 1710 => "Arena",
        0 => "Custom",
        _ => "Other",
    }
}

/// Standard 5v5 Summoner's Rift games, which are the only ones laning and macro
/// metrics apply to. Everything else is stored but excluded from analysis.
pub fn is_standard_rift(queue_id: i32, map_id: i32) -> bool {
    map_id == SUMMONERS_RIFT_MAP && STANDARD_RIFT_QUEUES.contains(&queue_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_queues() {
        assert!(is_standard_rift(420, 11));
        assert!(is_standard_rift(440, 11));
        assert!(!is_standard_rift(450, 12));
        assert!(!is_standard_rift(1700, 30));
        assert!(!is_standard_rift(900, 11));
        assert_eq!(queue_name(420), "Ranked Solo/Duo");
    }
}
