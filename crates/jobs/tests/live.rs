//! The live poller end to end: a mock Live Client game (loading → in game →
//! GameEnd) against mock Riot and Data Dragon servers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rc_common::Secret;
use rc_common::time::now_ms;
use rc_ddragon::DdragonCache;
use rc_jobs::{Engine, EngineConfig, EngineEvent, Job, LiveConfig, LiveState, spawn_live};
use rc_live::Phase;
use rc_live::mock::MockGame;
use rc_riot::client::RetryPolicy;
use rc_riot::{RateLimiter, RiotClient};
use rc_store::Db;
use serde_json::{Value, json};
use tokio::sync::watch;
use wiremock::matchers::{path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn synthetic_match(match_id: &str, start_ms: i64) -> Value {
    let participants: Vec<Value> = (1..=10)
        .map(|i| {
            let me = i == 4;
            json!({
                "participantId": i,
                "puuid": if me { "p-me".to_owned() } else { format!("p{i}") },
                "riotIdGameName": if me { "Me".to_owned() } else { format!("Player{i}") },
                "riotIdTagline": "NA1",
                "teamId": if i <= 5 { 100 } else { 200 },
                "championId": 103, "championName": "Ahri",
                "teamPosition": "MIDDLE", "individualPosition": "MIDDLE",
                "win": i <= 5, "kills": 4, "deaths": 3, "assists": 9, "champLevel": 15,
                "goldEarned": 11000, "totalMinionsKilled": 180, "neutralMinionsKilled": 12, "visionScore": 22
            })
        })
        .collect();
    json!({
        "metadata": {"matchId": match_id, "participants": []},
        "info": {
            "gameCreation": start_ms - 120_000, "gameStartTimestamp": start_ms,
            "gameEndTimestamp": start_ms + 1_680_000, "gameDuration": 1680,
            "gameMode": "CLASSIC", "gameType": "MATCHED_GAME", "gameVersion": "16.20.712.3456",
            "mapId": 11, "queueId": 420, "platformId": "NA1", "participants": participants
        }
    })
}

/// What the mock game client answers right now.
#[derive(Clone, Copy)]
enum Script {
    Loading,
    At(f64),
}

struct LiveResponder {
    script: Arc<Mutex<Script>>,
    game: MockGame,
}

impl Respond for LiveResponder {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        match *self.script.lock().expect("lock") {
            Script::Loading => ResponseTemplate::new(404).set_body_string(r#"{"errorCode":"RESOURCE_NOT_FOUND"}"#),
            Script::At(t) => ResponseTemplate::new(200).set_body_json(self.game.all_game_data(t)),
        }
    }
}

async fn wait_state(rx: &mut watch::Receiver<LiveState>, what: &str, f: impl FnMut(&LiveState) -> bool) -> LiveState {
    tokio::time::timeout(Duration::from_secs(10), rx.wait_for(f))
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
        .expect("poller alive")
        .clone()
}

#[tokio::test]
async fn live_game_holds_work_and_links_the_finished_match() {
    // The live game is first seen 60 s in, so it started about a minute ago.
    let start_ms = now_ms() - 60_000;

    let riot = MockServer::start().await;
    Mock::given(path_regex(r"(?i)^/riot/account/v1/accounts/by-riot-id/me/na1$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"puuid": "p-me", "gameName": "Me", "tagLine": "NA1"})),
        )
        .mount(&riot)
        .await;
    // Not published on the first look, then it is.
    Mock::given(path("/lol/match/v5/matches/by-puuid/p-me/ids"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&riot)
        .await;
    Mock::given(path("/lol/match/v5/matches/by-puuid/p-me/ids"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(["NA1_9"])))
        .mount(&riot)
        .await;
    Mock::given(path("/lol/match/v5/matches/NA1_9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(synthetic_match("NA1_9", start_ms)))
        .expect(1)
        .mount(&riot)
        .await;
    Mock::given(path("/lol/match/v5/matches/NA1_9/timeline"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"metadata": {"matchId": "NA1_9"}, "info": {"frameInterval": 60000, "frames": [{}, {}, {}]}}),
        ))
        .mount(&riot)
        .await;
    // The loading-screen brief reads champions and teams from Spectator-V5.
    let participants: Vec<Value> = (1..=10)
        .map(|i| {
            json!({
                "puuid": if i == 4 { "p-me".to_owned() } else { format!("p{i}") },
                "riotId": format!("Someone{i}#NA1"),
                "championId": if i == 4 { 103 } else { i * 10 },
                "teamId": if i <= 5 { 100 } else { 200 },
            })
        })
        .collect();
    Mock::given(path("/lol/spectator/v5/active-games/by-summoner/p-me"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"gameId": 777, "gameQueueConfigId": 420, "mapId": 11, "participants": participants}),
            ),
        )
        .mount(&riot)
        .await;
    Mock::given(path_regex(r"^/lol/league/v4/entries/by-puuid/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&riot)
        .await;
    let ddragon = MockServer::start().await;
    Mock::given(path("/api/versions.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(["16.20.1"])))
        .mount(&ddragon)
        .await;
    Mock::given(path_regex(r"^/cdn/16\.20\.1/data/en_US/.*\.json$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":{}}"#))
        .mount(&ddragon)
        .await;

    let script = Arc::new(Mutex::new(Script::Loading));
    let game = MockGame { riot_id: "Me#NA1".into(), champion: ("Ahri".into(), "Ahri".into()), ..Default::default() };
    let live = MockServer::start().await;
    Mock::given(path("/liveclientdata/allgamedata"))
        .respond_with(LiveResponder { script: script.clone(), game: game.clone() })
        .mount(&live)
        .await;

    let dir = tempfile::tempdir().expect("tmp");
    let db = Db::open(&dir.path().join("t.db")).expect("db");
    let client = RiotClient::new(Some(Secret::new("RGAPI-test")), Arc::new(RateLimiter::personal_key()))
        .expect("client")
        .with_base_url(riot.uri().parse().expect("url"))
        .with_retry_policy(RetryPolicy { rate_limited: 2, server: 1, base_backoff: Duration::from_millis(5) });
    let cache = DdragonCache::with_base_url(dir.path().join("dd"), db.clone(), &ddragon.uri()).expect("dd");
    let config = EngineConfig {
        workers: 2,
        idle_poll: Duration::from_millis(100),
        base_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
    };
    let engine = Engine::new(db.clone(), Arc::new(client), Arc::new(cache), config);
    engine.start().await.expect("start");
    let me = db.with(|c| rc_store::accounts::upsert_account(c, "Me", "NA1", "na1", "americas", true)).expect("account");
    engine.enqueue(Job::ResolveAccount { account_id: me, then_sync: None }).await.expect("enqueue");
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle()).await.expect("idle").expect("store");

    let mut events = engine.subscribe();
    let handle = spawn_live(
        engine.clone(),
        LiveConfig {
            url: live.uri(),
            max_poll: Some(Duration::from_millis(30)),
            footprint_every: Duration::from_millis(10),
            post_game_delay: Duration::ZERO,
        },
    )
    .expect("live");
    let mut rx = handle.state.clone();

    // Loading screen: work is already held.
    wait_state(&mut rx, "loading", |s| s.phase == Phase::Loading).await;
    assert!(engine.in_game());
    engine.enqueue(Job::DdragonSync { patch: "16.20".into() }).await.expect("enqueue");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let s = wait_state(&mut rx, "brief", |s| s.brief.as_ref().is_some_and(|b| !b.loading)).await;
    let b = s.brief.expect("brief state");
    let brief = b.brief.expect("brief");
    assert_eq!((brief.game_id, brief.champion_id), (777, 103));
    assert_eq!(brief.enemies.len(), 5);
    assert_eq!(b.plan_status, rc_jobs::PlanStatus::Off, "AI game plans are off by default");
    let status = engine.status().await.expect("status");
    assert!(status.in_game);
    assert_eq!(status.jobs.ready, 1, "queued work waits for the game to end");

    *script.lock().expect("lock") = Script::At(60.0);
    let s = wait_state(&mut rx, "in game", |s| s.phase == Phase::InGame && s.live_game_id.is_some()).await;
    assert!(s.tracked);

    *script.lock().expect("lock") = Script::At(900.0);
    let s = wait_state(&mut rx, "15:00", |s| s.snapshot.as_ref().is_some_and(|x| x.game_time_s == 900.0)).await;
    let snap = s.snapshot.expect("snapshot");
    assert_eq!((snap.scores.deaths, snap.scores.creep_score), (2, game.cs_at(900.0)));
    assert_eq!(s.pacing.expect("pacing").cs, game.cs_at(900.0));
    assert!(s.goals.is_empty());

    *script.lock().expect("lock") = Script::At(game.duration_s + 1.0);
    wait_state(&mut rx, "end", |s| s.phase == Phase::Ended).await;
    assert!(!engine.in_game(), "work resumes on the end screen");

    let ready = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(EngineEvent::PostGameReady { match_id, live_game_id, review_queued }) = events.recv().await {
                return (match_id, live_game_id, review_queued);
            }
        }
    })
    .await;
    let ready = match ready {
        Ok(r) => r,
        Err(_) => {
            let st = engine.status().await.expect("status");
            let rows = db.with(|c| rc_store::live::recent(c, 5)).expect("q");
            let acts = db.with(|c| rc_store::activity::recent(c, 50)).expect("q");
            panic!(
                "no post-game: {:?}
{:?}
{:?}
{:#?}",
                st.jobs, st.recent_failures, rows, acts
            );
        }
    };
    assert_eq!(ready.0, "NA1_9");
    assert!(!ready.2, "auto review is off by default");

    let (row, snapshots) = db
        .with(|c| Ok((rc_store::live::get(c, ready.1)?.expect("row"), rc_store::live::snapshots(c, ready.1)?)))
        .expect("q");
    assert_eq!(row.match_id.as_deref(), Some("NA1_9"));
    assert_eq!(row.account_id, Some(me));
    assert_eq!(row.result.as_deref(), Some("Win"));
    assert_eq!(row.end_reason.as_deref(), Some("game_end"));
    assert!(row.analyzed_at.is_some());
    assert!(row.footprint_json.is_some(), "footprint measured during the game");
    assert!(snapshots.len() >= 2, "{}", snapshots.len());
    assert!(!row.snapshots_pruned, "kept for 7 days by default");
    let brief_json: Option<String> = db
        .with(|c| Ok(c.query_row("SELECT brief_json FROM live_games WHERE id = ?1", [ready.1], |r| r.get(0))?))
        .expect("q");
    assert!(brief_json.is_some_and(|j| j.contains("777") && !j.contains("Someone")), "brief stored, no names");
}

/// Real screen capture through the poller (not in CI: needs a window). Run with
/// `$env:RIFT_COACH_VISION_WINDOW = "<part of a window title>"` and
/// `cargo test -p rc-jobs --test live -- --ignored vision`.
#[tokio::test]
#[ignore = "captures a real window; set RIFT_COACH_VISION_WINDOW"]
async fn vision_capture_during_a_game() {
    assert!(std::env::var(rc_vision::WINDOW_ENV).is_ok(), "set {}", rc_vision::WINDOW_ENV);
    let riot = MockServer::start().await;
    let ddragon = MockServer::start().await;
    let script = Arc::new(Mutex::new(Script::Loading));
    let game = MockGame { riot_id: "Me#NA1".into(), ..Default::default() };
    let live = MockServer::start().await;
    Mock::given(path("/liveclientdata/allgamedata"))
        .respond_with(LiveResponder { script: script.clone(), game: game.clone() })
        .mount(&live)
        .await;
    let dir = tempfile::tempdir().expect("tmp");
    let db = Db::open(&dir.path().join("t.db")).expect("db");
    let client = RiotClient::new(Some(Secret::new("RGAPI-test")), Arc::new(RateLimiter::personal_key()))
        .expect("client")
        .with_base_url(riot.uri().parse().expect("url"));
    let cache = DdragonCache::with_base_url(dir.path().join("dd"), db.clone(), &ddragon.uri()).expect("dd");
    let engine = Engine::new(db.clone(), Arc::new(client), Arc::new(cache), EngineConfig::default());
    let store = rc_vision::FrameStore::new(dir.path().join("vision"));
    engine.set_frame_store(store.clone());
    db.with(|c| {
        rc_store::accounts::upsert_account(c, "Me", "NA1", "na1", "americas", true)?;
        let mut s = rc_store::settings::app(c)?;
        s.vision.enabled = true;
        s.vision.interval_s = 3;
        rc_store::settings::set(c, rc_store::settings::APP, &s)
    })
    .expect("setup");
    let handle = spawn_live(
        engine.clone(),
        LiveConfig {
            url: live.uri(),
            max_poll: Some(Duration::from_millis(200)),
            footprint_every: Duration::from_secs(1),
            post_game_delay: Duration::from_secs(3600),
        },
    )
    .expect("live");
    let mut rx = handle.state.clone();
    *script.lock().expect("lock") = Script::At(60.0);
    let s = wait_state(&mut rx, "in game", |s| s.phase == Phase::InGame && s.live_game_id.is_some()).await;
    let id = s.live_game_id.expect("id");
    // 15 s of game, the clock moving with real time.
    for i in 1..=15 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        *script.lock().expect("lock") = Script::At(60.0 + f64::from(i));
    }
    let s = rx.borrow().clone();
    let v = s.vision.expect("vision status in the live state");
    assert!(v.running, "{v:?}");
    *script.lock().expect("lock") = Script::At(game.duration_s + 1.0);
    wait_state(&mut rx, "end", |s| s.phase == Phase::Ended).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let row = db.with(|c| rc_store::live::get(c, id)).expect("q").expect("row");
    assert_eq!(row.vision_state.as_deref(), Some("captured"));
    let stats: rc_vision::CaptureStats =
        serde_json::from_str(row.vision_json.as_deref().expect("stats")).expect("json");
    println!("{stats:#?}");
    let times = store.list(id);
    assert_eq!(times.len() as u32, stats.frames);
    assert!(stats.frames >= 3, "{stats:?}");
    assert!(times.windows(2).all(|w| w[1] - w[0] >= 2_500), "about 3 s apart: {times:?}");
    assert!(times[0] >= 60_000 && times[0] < 80_000, "stamped with game time: {times:?}");
}
