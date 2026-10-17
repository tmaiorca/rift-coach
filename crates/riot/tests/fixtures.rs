//! Recorded (scrubbed) real responses parse into our DTOs.

use std::path::PathBuf;

use rc_riot::dto::{AccountDto, ChampionMasteryDto, LeagueEntryDto, MatchDto, TimelineDto, patch_of};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/riot")
}

fn read(rel: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(rel)).unwrap_or_else(|e| panic!("fixture {rel}: {e}"))
}

fn match_ids() -> Vec<String> {
    serde_json::from_slice(&read("match-ids.json")).expect("match-ids.json")
}

#[test]
fn recorded_matches_parse() {
    let ids = match_ids();
    assert!(!ids.is_empty());
    for id in ids {
        let m: MatchDto = serde_json::from_slice(&read(&format!("matches/{id}.json"))).expect("match");
        assert_eq!(m.metadata.match_id, id);
        assert_eq!(m.info.participants.len(), 10, "{id}");
        assert!(m.info.duration_seconds() > 0 && m.info.duration_seconds() < 4 * 3600, "{id}: duration in seconds");
        assert!(patch_of(&m.info.game_version).split('.').count() == 2, "{id}: patch");
        let me: Vec<_> =
            m.info.participants.iter().filter(|p| p.riot_id_game_name.as_deref() == Some("TapRooted")).collect();
        assert_eq!(me.len(), 1, "{id}: exactly one self participant");
        assert_eq!(me[0].puuid, "FIXTURE-PUUID-SELF");
    }
}

#[test]
fn recorded_timelines_parse() {
    for id in match_ids() {
        let t: TimelineDto = serde_json::from_slice(&read(&format!("timelines/{id}.json"))).expect("timeline");
        assert_eq!(t.metadata.match_id, id);
        assert_eq!(t.info.frame_interval, 60_000, "{id}: one-minute frames");
        assert!(t.info.frames.len() > 10, "{id}: frames");
    }
}

#[test]
fn recorded_account_league_and_mastery_parse() {
    let a: AccountDto = serde_json::from_slice(&read("account.json")).expect("account");
    assert_eq!(a.puuid, "FIXTURE-PUUID-SELF");
    let _: Vec<LeagueEntryDto> = serde_json::from_slice(&read("league-entries.json")).expect("league");
    let m: Vec<ChampionMasteryDto> = serde_json::from_slice(&read("mastery-top.json")).expect("mastery");
    assert!(!m.is_empty());
}
