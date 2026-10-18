//! The benchmark collector end to end against mock Riot and Data Dragon
//! servers: seed the ladder, find focus-champion players, list and fetch
//! their games, and build a benchmark population kept apart from "your
//! lobbies".

use std::sync::Arc;
use std::time::Duration;

use rc_common::Secret;
use rc_common::time::now_ms;
use rc_ddragon::DdragonCache;
use rc_jobs::{BenchConfig, BenchPhase, Engine, EngineConfig, Job, spawn_benchmarks};
use rc_riot::client::RetryPolicy;
use rc_riot::{RateLimiter, RiotClient};
use rc_store::Db;
use serde_json::{Value, json};
use wiremock::matchers::{path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn synthetic_match(match_id: &str, me_puuid: Option<&str>, start_ms: i64) -> Value {
    let participants: Vec<Value> = (1..=10)
        .map(|i| {
            let me = i == 4 && me_puuid.is_some();
            json!({
                "participantId": i,
                "puuid": if me { me_puuid.unwrap_or_default().to_owned() } else { format!("{match_id}-p{i}") },
                "riotIdGameName": if me { "Me".to_owned() } else { format!("Stranger{i}") },
                "riotIdTagline": "NA1",
                "summonerName": format!("Stranger{i}"),
                "teamId": if i <= 5 { 100 } else { 200 },
                "championId": 103, "championName": "Ahri",
                "teamPosition": "MIDDLE", "individualPosition": "MIDDLE",
                "win": i <= 5, "kills": 4, "deaths": 3, "assists": 9, "champLevel": 15,
                "goldEarned": 11000, "totalMinionsKilled": 180 + i, "neutralMinionsKilled": 12, "visionScore": 22
            })
        })
        .collect();
    json!({
        "metadata": {"matchId": match_id, "participants": []},
        "info": {
            "gameCreation": start_ms, "gameStartTimestamp": start_ms,
            "gameEndTimestamp": start_ms + 1_800_000, "gameDuration": 1800,
            "gameMode": "CLASSIC", "gameType": "MATCHED_GAME", "gameVersion": "16.20.712.3456",
            "mapId": 11, "queueId": 420, "platformId": "NA1", "participants": participants
        }
    })
}

fn timeline(match_id: &str) -> Value {
    json!({"metadata": {"matchId": match_id}, "info": {"frameInterval": 60000, "frames": [{}, {}, {}]}})
}

async fn mount(riot: &MockServer, p: &str, body: Value) {
    Mock::given(path(p)).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(riot).await;
}

#[tokio::test]
async fn collects_benchmark_games_apart_from_your_lobbies() {
    let start = now_ms() - 3_600_000;
    let riot = MockServer::start().await;
    Mock::given(path_regex(r"(?i)^/riot/account/v1/accounts/by-riot-id/me/na1$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"puuid": "p-me", "gameName": "Me", "tagLine": "NA1"})),
        )
        .mount(&riot)
        .await;
    mount(&riot, "/lol/match/v5/matches/by-puuid/p-me/ids", json!(["NA1_1"])).await;
    mount(&riot, "/lol/match/v5/matches/NA1_1", synthetic_match("NA1_1", Some("p-me"), start)).await;
    mount(&riot, "/lol/match/v5/matches/NA1_1/timeline", timeline("NA1_1")).await;
    mount(&riot, "/lol/league/v4/entries/by-puuid/p-me", json!([])).await;
    // The Diamond ladder: two players.
    Mock::given(path("/lol/league/v4/entries/RANKED_SOLO_5x5/DIAMOND/I"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"queueType": "RANKED_SOLO_5x5", "tier": "DIAMOND", "rank": "I", "leaguePoints": 50, "puuid": "b-main"},
            {"queueType": "RANKED_SOLO_5x5", "tier": "DIAMOND", "rank": "I", "leaguePoints": 20, "puuid": "b-other"}
        ])))
        .mount(&riot)
        .await;
    mount(&riot, "/lol/league/v4/entries/RANKED_SOLO_5x5/DIAMOND/II", json!([])).await;
    // b-main mains Ahri (103), b-other doesn't.
    mount(&riot, "/lol/champion-mastery/v4/champion-masteries/by-puuid/b-main/top", json!([{"championId": 103}])).await;
    mount(&riot, "/lol/champion-mastery/v4/champion-masteries/by-puuid/b-other/top", json!([{"championId": 1}])).await;
    mount(&riot, "/lol/match/v5/matches/by-puuid/b-main/ids", json!(["NA1_B1"])).await;
    mount(&riot, "/lol/match/v5/matches/by-puuid/b-other/ids", json!(["NA1_B2"])).await;
    for id in ["NA1_B1", "NA1_B2"] {
        mount(&riot, &format!("/lol/match/v5/matches/{id}"), synthetic_match(id, None, start)).await;
        mount(&riot, &format!("/lol/match/v5/matches/{id}/timeline"), timeline(id)).await;
    }
    let ddragon = MockServer::start().await;
    mount(&ddragon, "/api/versions.json", json!(["16.20.1"])).await;
    Mock::given(path_regex(r"^/cdn/16\.20\.1/data/en_US/.*\.json$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":{}}"#))
        .mount(&ddragon)
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
    let me = db.with(|c| rc_store::accounts::upsert_account(c, "Me", "NA1", "na1", "americas", true)).expect("acct");
    db.with(|c| {
        let mut s = rc_store::settings::app(c)?;
        s.benchmarks.champions = vec!["Ahri".into()];
        rc_store::settings::set(c, rc_store::settings::APP, &s)
    })
    .expect("settings");
    engine.enqueue(Job::ResolveAccount { account_id: me, then_sync: Some(1) }).await.expect("enqueue");
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle()).await.expect("idle").expect("store");

    // Held while a game runs.
    engine.set_in_game(true);
    let mut rx = spawn_benchmarks(engine.clone(), BenchConfig { fast: Some(Duration::from_millis(20)) });
    tokio::time::timeout(Duration::from_secs(5), rx.wait_for(|s| s.phase == BenchPhase::Waiting))
        .await
        .expect("waits during a game")
        .expect("alive");
    let stored = |db: &Db| {
        db.with(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM matches m WHERE benchmark_tier = 'DIAMOND'
                   AND EXISTS (SELECT 1 FROM match_metrics_state s WHERE s.match_id = m.match_id)",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .expect("q")
    };
    assert_eq!(stored(&db), 0);
    engine.set_in_game(false);

    tokio::time::timeout(Duration::from_secs(20), async {
        while stored(&db) < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("benchmark games not collected: {:?}", rx.borrow().clone()));

    db.with(|c| {
        // The focus player's game was queued with priority.
        let prio: i64 = c.query_row("SELECT priority FROM bench_queue WHERE match_id = 'NA1_B1'", [], |r| r.get(0))?;
        assert_eq!(prio, 1);
        // Other players' names are gone from benchmark games.
        let raw = rc_store::matches::get_raw(c, "NA1_B1", rc_store::matches::RawKind::Match)?.expect("raw");
        assert!(!String::from_utf8_lossy(&raw).contains("Stranger"));
        let named: i64 = c.query_row(
            "SELECT COUNT(*) FROM match_participants WHERE match_id = 'NA1_B1' AND riot_id_name IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(named, 0);
        // A benchmark population exists; your lobby population doesn't include it.
        assert_eq!(rc_store::bench::sample_count(c, "DIAMOND", "16.20", "MIDDLE", None)?, 20);
        let bench = rc_store::bench::population(c, "DIAMOND", "16.20", "MIDDLE", Some("Ahri"))?;
        assert_eq!(bench.get("cspm").map(Vec::len), Some(20));
        let lobby = rc_store::metrics::role_population(c, me, "MIDDLE", None)?;
        assert_eq!(lobby.get("cspm").map(Vec::len), Some(9), "only the other 9 players of your own game");
        Ok(())
    })
    .expect("checks");
}
