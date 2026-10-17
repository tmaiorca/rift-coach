//! Metrics on a recorded, scrubbed match (TapRooted on LeBlanc mid,
//! participant 3, vs Naafiri, participant 8). Expected values were checked
//! independently against the raw JSON.

use rc_metrics::detail::{deaths_of, map_events, series};
use rc_metrics::model::ItemCatalog;
use rc_metrics::{compute, parse_match, parse_timeline};

const MATCH: &str = "NA1_5659312018";

fn load(kind: &str) -> Vec<u8> {
    let path = format!("{}/../../tests/fixtures/riot/{kind}/{MATCH}.json", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(path).expect("fixture")
}

/// A tiny item.json with just the items bought on the first back.
fn catalog() -> ItemCatalog {
    ItemCatalog::from_ddragon_json(
        br#"{"data": {
            "3145": {"name": "Hextech Alternator", "into": ["4645"], "tags": [], "gold": {"total": 1050}},
            "1027": {"name": "Sapphire Crystal", "into": ["3070"], "tags": [], "gold": {"total": 250}},
            "2055": {"name": "Control Ward", "tags": ["Consumable"], "gold": {"total": 75}},
            "6655": {"name": "Luden's Echo", "tags": [], "gold": {"total": 2750}}
        }}"#,
    )
    .expect("catalog")
}

#[test]
fn computes_laning_and_macro_metrics_for_the_player() {
    let m = parse_match(&load("matches")).expect("match");
    let tl = parse_timeline(&load("timelines")).expect("timeline");
    let metrics = compute(&m, Some(&tl), &catalog());
    assert_eq!(metrics.by_participant.len(), 10);
    let me = &metrics.by_participant[&3];

    assert_eq!(me["cs_10"], 49.0);
    assert_eq!(me["cs_diff_10"], -11.0);
    assert_eq!(me["gold_diff_10"], 519.0);
    assert_eq!(me["kp"], 50.0);
    assert_eq!(me["deaths"], 7.0);
    assert_eq!(me["deaths_pre14"], 3.0);
    assert!((me["first_back_min"] - 308.617 / 60.0).abs() < 1e-9);
    assert_eq!(me["first_back_gold"], 1050.0 + 250.0 + 75.0);
    assert!((me["first_item_min"] - 640.237 / 60.0).abs() < 1e-9);
    assert!(me["dmg_share"] > 0.0 && me["dmg_share"] <= 100.0);
    assert!(me["obj_part"] >= 0.0 && me["obj_part"] <= 100.0);
    // Game is 27:45; 15-minute metrics exist.
    assert!(me.contains_key("xp_diff_15"));
}

#[test]
fn match_only_metrics_without_timeline() {
    let m = parse_match(&load("matches")).expect("match");
    let metrics = compute(&m, None, &ItemCatalog::default());
    let me = &metrics.by_participant[&3];
    assert_eq!(me["kp"], 50.0);
    assert!(!me.contains_key("cs_10"));
    assert!(!me.contains_key("first_back_gold"));
}

#[test]
fn detail_views() {
    let m = parse_match(&load("matches")).expect("match");
    let tl = parse_timeline(&load("timelines")).expect("timeline");
    let s = series(&tl);
    assert_eq!(s.minutes.len(), 29);
    assert_eq!(s.by_participant[&3].cs[10], 49);

    let events = map_events(&m, &tl);
    let kills = events.iter().filter(|e| matches!(e.kind, rc_metrics::detail::MapEventKind::Kill)).count();
    assert_eq!(kills, 50);

    let me = m.info.participants.iter().find(|p| p.participant_id == 3).expect("me");
    let deaths = deaths_of(&m, &tl, me);
    assert_eq!(deaths.len(), 7);
    assert_eq!(deaths.iter().filter(|d| d.before_14).count(), 3);
}
