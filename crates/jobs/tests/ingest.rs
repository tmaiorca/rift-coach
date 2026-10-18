//! End-to-end ingestion against mock Riot and Data Dragon servers.

use std::sync::Arc;
use std::time::Duration;

use rc_common::Secret;
use rc_ddragon::DdragonCache;
use rc_jobs::{Engine, EngineConfig, IdleOutcome, KeyStatus};
use rc_riot::client::RetryPolicy;
use rc_riot::{Platform, RateLimiter, RiotClient, RiotId};
use rc_store::Db;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn synthetic_match(match_id: &str, me_puuid: &str) -> Value {
    let participants: Vec<Value> = (1..=10)
        .map(|i| {
            let me = i == 4;
            json!({
                "participantId": i,
                "puuid": if me { me_puuid.to_owned() } else { format!("p{i}") },
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
            "gameCreation": 1_700_000_000_000i64, "gameStartTimestamp": 1_700_000_000_000i64,
            "gameEndTimestamp": 1_700_001_800_000i64, "gameDuration": 1800,
            "gameMode": "CLASSIC", "gameType": "MATCHED_GAME", "gameVersion": "16.20.712.3456",
            "mapId": 11, "queueId": 420, "platformId": "NA1", "participants": participants
        }
    })
}

fn synthetic_timeline(match_id: &str) -> Value {
    json!({"metadata": {"matchId": match_id}, "info": {"frameInterval": 60000, "frames": [{}, {}, {}]}})
}

struct Harness {
    riot: MockServer,
    _ddragon: MockServer,
    _dir: tempfile::TempDir,
    db: Db,
    engine: Arc<Engine>,
}

async fn harness(mount_account: bool) -> Harness {
    let riot = MockServer::start().await;
    if mount_account {
        mount_account_ok(&riot, 5).await;
    }
    Mock::given(path("/lol/match/v5/matches/by-puuid/p-me/ids"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(["NA1_3", "NA1_2", "NA1_1"])))
        .mount(&riot)
        .await;
    for id in ["NA1_1", "NA1_2", "NA1_3"] {
        Mock::given(path(format!("/lol/match/v5/matches/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(synthetic_match(id, "p-me")))
            .expect(1)
            .mount(&riot)
            .await;
        Mock::given(path(format!("/lol/match/v5/matches/{id}/timeline")))
            .respond_with(ResponseTemplate::new(200).set_body_json(synthetic_timeline(id)))
            .expect(1)
            .mount(&riot)
            .await;
    }

    Mock::given(path_regex(r"^/lol/league/v4/entries/by-puuid/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"queueType": "RANKED_SOLO_5x5", "tier": "GOLD", "rank": "II", "leaguePoints": 40, "wins": 10, "losses": 8}
        ])))
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

    let dir = tempfile::tempdir().expect("tmp");
    let db = Db::open(&dir.path().join("t.db")).expect("db");
    let client = RiotClient::new(Some(Secret::new("RGAPI-test")), Arc::new(RateLimiter::personal_key()))
        .expect("client")
        .with_base_url(riot.uri().parse().expect("url"))
        .with_retry_policy(RetryPolicy { rate_limited: 2, server: 1, base_backoff: Duration::from_millis(5) });
    let cache = DdragonCache::with_base_url(dir.path().join("dd"), db.clone(), &ddragon.uri()).expect("dd");
    let config = EngineConfig {
        workers: 3,
        idle_poll: Duration::from_millis(200),
        base_backoff: Duration::from_millis(50),
        max_backoff: Duration::from_millis(200),
    };
    let engine = Engine::new(db.clone(), Arc::new(client), Arc::new(cache), config);
    engine.start().await.expect("start");
    Harness { riot, _ddragon: ddragon, _dir: dir, db, engine }
}

async fn mount_account_ok(riot: &MockServer, priority: u8) {
    Mock::given(path_regex(r"(?i)^/riot/account/v1/accounts/by-riot-id/me/na1$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"puuid": "p-me", "gameName": "Me", "tagLine": "NA1"})),
        )
        .with_priority(priority)
        .mount(riot)
        .await;
}

async fn wait(engine: &Engine) -> IdleOutcome {
    tokio::time::timeout(Duration::from_secs(20), engine.wait_idle()).await.expect("timed out").expect("store")
}

#[tokio::test]
async fn ingests_matches_and_timelines_once() {
    let h = harness(true).await;
    // Lowercase on purpose: Riot's casing is stored after resolving.
    let me = h.engine.track_and_ingest(&RiotId::new("me", "na1"), Platform::Na1, 100).await.expect("track");
    assert_eq!(wait(&h.engine).await, IdleOutcome::Drained);

    let (counts, list, account) =
        h.db.with(|c| {
            Ok((
                rc_store::matches::account_match_counts(c, me)?,
                rc_store::matches::list_account_matches(c, me, 10, 0)?,
                rc_store::accounts::get_account(c, me)?,
            ))
        })
        .expect("q");
    assert_eq!((counts.matches, counts.with_timeline), (3, 3));
    assert_eq!(list[0].cs, 192, "cs = lane minions + neutral");
    assert!(list.iter().all(|m| m.win && m.has_timeline && m.patch == "16.20"));
    assert_eq!(account.map(|a| (a.game_name, a.tag_line)), Some(("Me".into(), "NA1".into())));
    let status = h.engine.status().await.expect("status");
    assert!(status.recent_failures.is_empty(), "{:?}", status.recent_failures);
    assert!(h.db.with(|c| rc_store::ddragon::is_complete(c, "16.20.1")).expect("q"));
    assert_eq!(h.engine.key_status(), KeyStatus::Ok);

    // Re-running fetches no match or timeline again (mocks expect(1)).
    h.engine.track_and_ingest(&RiotId::new("Me", "NA1"), Platform::Na1, 100).await.expect("track");
    assert_eq!(wait(&h.engine).await, IdleOutcome::Drained);
    let status = h.engine.status().await.expect("status");
    assert_eq!(status.jobs.failed, 0);
    h.riot.verify().await;
}

#[tokio::test]
async fn retries_a_match_that_is_not_published_yet() {
    let riot_404_first = harness(true).await;
    Mock::given(method("GET"))
        .and(path("/lol/match/v5/matches/NA1_2"))
        .respond_with(ResponseTemplate::new(404))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&riot_404_first.riot)
        .await;
    let me = riot_404_first.engine.track_and_ingest(&RiotId::new("Me", "NA1"), Platform::Na1, 3).await.expect("track");
    assert_eq!(wait(&riot_404_first.engine).await, IdleOutcome::Drained);
    let counts = riot_404_first.db.with(|c| rc_store::matches::account_match_counts(c, me)).expect("q");
    assert_eq!(counts.matches, 3);
}

#[tokio::test]
async fn pauses_on_rejected_key_and_resumes_with_a_new_one() {
    let h = harness(false).await;
    Mock::given(path("/riot/account/v1/accounts/by-riot-id/Me/NA1"))
        .respond_with(ResponseTemplate::new(403))
        .with_priority(5)
        .mount(&h.riot)
        .await;
    let me = h.engine.track_and_ingest(&RiotId::new("Me", "NA1"), Platform::Na1, 3).await.expect("track");
    assert_eq!(wait(&h.engine).await, IdleOutcome::KeyProblem(KeyStatus::Rejected));
    let status = h.engine.status().await.expect("status");
    assert_eq!(status.jobs.pending(), 1, "job kept, not failed");
    assert_eq!(status.jobs.failed, 0);

    // User fixes the key; the account endpoint now succeeds.
    Mock::given(path("/riot/account/v1/accounts/by-riot-id/Me/NA1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"puuid": "p-me", "gameName": "Me", "tagLine": "NA1"})),
        )
        .with_priority(1)
        .mount(&h.riot)
        .await;
    h.engine.set_key(Some(Secret::new("RGAPI-new")));
    assert_eq!(wait(&h.engine).await, IdleOutcome::Drained);
    let counts = h.db.with(|c| rc_store::matches::account_match_counts(c, me)).expect("q");
    assert_eq!(counts.matches, 3);
}

/// Replays the recorded (scrubbed) real responses through the whole pipeline.
#[tokio::test]
async fn ingests_recorded_fixtures() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/riot");
    let read = |rel: &str| std::fs::read(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let ids: Vec<String> = serde_json::from_slice(&read("match-ids.json")).expect("ids");

    let riot = MockServer::start().await;
    Mock::given(path("/riot/account/v1/accounts/by-riot-id/TapRooted/BTC"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(read("account.json")))
        .mount(&riot)
        .await;
    Mock::given(path("/lol/match/v5/matches/by-puuid/FIXTURE-PUUID-SELF/ids"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(read("match-ids.json")))
        .mount(&riot)
        .await;
    for id in &ids {
        Mock::given(path(format!("/lol/match/v5/matches/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(read(&format!("matches/{id}.json"))))
            .mount(&riot)
            .await;
        Mock::given(path(format!("/lol/match/v5/matches/{id}/timeline")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(read(&format!("timelines/{id}.json"))))
            .mount(&riot)
            .await;
    }
    Mock::given(path("/lol/league/v4/entries/by-puuid/FIXTURE-PUUID-SELF"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(read("league-entries.json")))
        .mount(&riot)
        .await;
    let ddragon = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&ddragon).await;

    let tmp = tempfile::tempdir().expect("tmp");
    let db = Db::open(&tmp.path().join("t.db")).expect("db");
    let client = RiotClient::new(Some(Secret::new("RGAPI-test")), Arc::new(RateLimiter::personal_key()))
        .expect("client")
        .with_base_url(riot.uri().parse().expect("url"));
    let cache = DdragonCache::with_base_url(tmp.path().join("dd"), db.clone(), &ddragon.uri()).expect("dd");
    let config = EngineConfig {
        workers: 2,
        idle_poll: Duration::from_millis(200),
        // Data Dragon is down in this test; let its job fail fast.
        base_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(5),
    };
    let engine = Engine::new(db.clone(), Arc::new(client), Arc::new(cache), config);
    engine.start().await.expect("start");

    let me = engine.track_and_ingest(&RiotId::new("TapRooted", "BTC"), Platform::Na1, 20).await.expect("track");
    assert_eq!(wait(&engine).await, IdleOutcome::Drained);

    let (counts, list) = db
        .with(|c| {
            Ok((
                rc_store::matches::account_match_counts(c, me)?,
                rc_store::matches::list_account_matches(c, me, 50, 0)?,
            ))
        })
        .expect("q");
    assert_eq!(counts.matches as usize, ids.len());
    assert_eq!(counts.with_timeline as usize, ids.len());
    let listed: Vec<_> = list.iter().map(|m| m.match_id.clone()).collect();
    assert_eq!(listed, ids, "newest first, same order Riot returned");
    // Stored raw JSON round-trips byte-for-byte.
    let raw = db
        .with(|c| rc_store::matches::get_raw(c, &ids[0], rc_store::matches::RawKind::Timeline))
        .expect("q")
        .expect("raw");
    assert_eq!(raw, read(&format!("timelines/{}.json", ids[0])));
    // Only the Data Dragon job (mock is down) failed.
    let failures = engine.status().await.expect("status").recent_failures;
    assert!(failures.iter().all(|f| f.kind == "ddragon_sync"), "{failures:?}");
    // The rank came along with the sync.
    let ranks = db.with(|c| rc_store::bench::ranks(c, 1)).expect("ranks");
    assert!(ranks.iter().any(|r| r.queue_type == "RANKED_SOLO_5x5" && r.tier.is_some()), "{ranks:?}");

    // Metrics were still computed for all ten players of every match, but
    // stored as partial (version 0) so a later start completes them.
    let (stale, mine) = db
        .with(|c| {
            let (_, pid) = rc_store::matches::account_match(c, me, &ids[0])?.expect("row");
            Ok((
                rc_store::metrics::stale_matches(c, rc_metrics::METRICS_VERSION)?,
                rc_store::metrics::participant_metrics(c, &ids[0], pid)?,
            ))
        })
        .expect("q");
    assert_eq!(stale.len(), ids.len());
    assert!(mine.contains_key("cs_10") && mine.contains_key("kp"));
    assert!(!mine.contains_key("first_back_gold"), "needs item prices");

    // On-demand AI review against a mock provider: the first reply is
    // unusable, so the coach asks again with feedback.
    let ai = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "Sorry, here is some prose instead of JSON."}],
            "usage": {"input_tokens": 1000, "output_tokens": 10}
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&ai)
        .await;
    let good = json!({
        "summary": "You traded evenly early but fell behind after repeated laning deaths.",
        "went_well": [{"title": "Fight presence", "detail": "You joined most of your team's kills.", "evidence": ["kp"]}],
        "improve": [{"title": "Early deaths", "detail": "Deaths before 14:00 cost lane control.", "evidence": ["deaths_pre14", "death_1"]}],
        "priority_fixes": [{"title": "Respect jungle timers", "why": "Early deaths gave tempo away.", "how": "Ward river before pushing.", "evidence": ["deaths_pre14"]}],
        "next_game_focus": {"goal": "At most 1 death before 14:00", "metric": "deaths_pre14"},
        "confidence": "medium",
        "confidence_reason": "One game of data."
    });
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": good.to_string()}],
            "usage": {"input_tokens": 1100, "output_tokens": 300}
        })))
        .with_priority(2)
        .mount(&ai)
        .await;
    let client = rc_ai::AiClient::new(
        rc_ai::ProviderConfig { kind: rc_ai::ProviderKind::Anthropic, model: "m".into(), base_url: Some(ai.uri()) },
        Some(Secret::new("sk-test")),
    )
    .expect("ai");
    // The newest game has deaths; pick one with at least one.
    let target = ids
        .iter()
        .find(|id| db.with(|c| Ok(rc_store::matches::account_match(c, me, id)?.expect("row").0.deaths > 0)).expect("q"))
        .expect("a game with deaths");
    let opts = rc_coach::review::ReviewOptions {
        baseline_games: 30,
        session_gap_min: 60,
        max_tokens: 2000,
        monthly_token_limit: None,
        force: false,
    };
    let report = rc_coach::review::post_game_review(&db, &client, me, target, &opts, None).await.expect("review");
    assert_eq!(report.validation, "ok", "{:?}", report.notes);
    assert_eq!(report.output_tokens, Some(310));
    let requests = ai.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 2);
    let body = String::from_utf8_lossy(&requests[0].body);
    assert!(body.contains("deaths_pre14") && body.contains("vs_role"), "facts sent");
    assert!(!body.contains("FIXTURE-PUUID") && !body.contains("Player0"), "no other players' identities");

    // Cached: asking again doesn't call the provider.
    let again = rc_coach::review::post_game_review(&db, &client, me, target, &opts, None).await.expect("cached");
    assert_eq!(again.id, report.id);
    assert_eq!(ai.received_requests().await.unwrap_or_default().len(), 2);

    // Death events are stored for every death of every player.
    let (summary, pid) = db.with(|c| Ok(rc_store::matches::account_match(c, me, target)?.expect("row"))).expect("q");
    let deaths = db.with(|c| rc_store::metrics::participant_deaths(c, target, pid)).expect("q");
    assert_eq!(deaths.len() as i32, summary.deaths);

    // A goal started before these games counts all three, and completes.
    db.with(|c| {
        rc_store::goals::insert(
            c,
            &rc_store::goals::NewGoal {
                account_id: me,
                metric: "deaths_pre14",
                comparator: "at_most",
                target: 1.0,
                games: 3,
                role: None,
                champion_name: None,
                baseline: Some(2.0),
                source: "user",
                note: None,
            },
            Some(0),
        )
    })
    .expect("goal");
    let goals = db.with(|c| Ok(rc_coach::goals::list(c, me, true))).expect("q").expect("goals");
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].progress.games_played, 3);
    assert_eq!(goals[0].goal.status, "completed", "all games played");
    assert!(goals[0].goal.achieved.is_some());
    // Completed goals don't show among active ones.
    assert!(db.with(|c| Ok(rc_coach::goals::list(c, me, false))).expect("q").expect("goals").is_empty());

    // The three recorded games are one session.
    let sessions = db.with(|c| rc_coach::sessions::list(c, me, 60, 10)).expect("sessions");
    assert_eq!(sessions.len(), 1);
    let scope = sessions[0].scope.clone();
    assert_eq!((sessions[0].games, sessions[0].rift_games), (3, 3));
    let detail = db.with(|c| Ok(rc_coach::sessions::detail(c, me, &scope, 60, 30))).expect("q").expect("detail");
    assert_eq!(detail.games.len(), 3);
    assert!(detail.aggregates.iter().any(|a| a.metric == "deaths_pre14"));

    // Session review against a mock provider.
    let ai2 = MockServer::start().await;
    let session_report = json!({
        "summary": "A tough session where laning deaths kept showing up.",
        "went_well": [{"title": "Warding", "detail": "Your vision held up across the session.", "evidence": ["vision_pm"]}],
        "improve": [{"title": "Laning deaths", "detail": "Early deaths repeated from game to game.", "evidence": ["deaths_pre14", "game_1"]}],
        "priority_fixes": [{"title": "Stop after two losses", "why": "Results dropped as the session went on.", "how": "Take a break after two losses in a row.", "evidence": ["session"]}],
        "next_game_focus": {"goal": "At most 1 death before 14:00 per game", "metric": "deaths_pre14", "comparator": "at_most", "target": 1},
        "confidence": "low",
        "confidence_reason": "Only a few games."
    });
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": session_report.to_string()}],
            "usage": {"input_tokens": 2000, "output_tokens": 400}
        })))
        .mount(&ai2)
        .await;
    let client2 = rc_ai::AiClient::new(
        rc_ai::ProviderConfig { kind: rc_ai::ProviderKind::Anthropic, model: "m".into(), base_url: Some(ai2.uri()) },
        Some(Secret::new("sk-test")),
    )
    .expect("ai");
    let r = rc_coach::review::session_review(&db, &client2, me, &scope, &opts).await.expect("session review");
    assert_eq!(r.kind, "session");
    assert_eq!(r.scope.as_deref(), Some(scope.as_str()));
    assert_eq!(r.validation, "ok", "{:?}", r.notes);
    let body = String::from_utf8_lossy(&ai2.received_requests().await.unwrap_or_default()[0].body).to_string();
    assert!(body.contains("game_3") && body.contains("aggregates"), "session facts sent");
    assert!(!body.contains("Player0"), "no other players' identities");
    let cached = rc_coach::review::session_review(&db, &client2, me, &scope, &opts).await.expect("cached");
    assert_eq!(cached.id, r.id);

    // The token cap blocks new reports once reached (2,710 tokens used so far).
    let capped = rc_coach::review::ReviewOptions { monthly_token_limit: Some(1000), force: true, ..opts.clone() };
    let err = rc_coach::review::session_review(&db, &client2, me, &scope, &capped).await.expect_err("capped");
    assert!(matches!(err, rc_coach::CoachError::Budget(_)), "{err}");
    assert_eq!(ai2.received_requests().await.unwrap_or_default().len(), 1, "no call once capped");
}

#[tokio::test]
async fn missing_key_blocks_without_calling_riot() {
    let h = harness(true).await;
    h.engine.set_key(None);
    h.engine.track_and_ingest(&RiotId::new("Me", "NA1"), Platform::Na1, 3).await.expect("track");
    assert_eq!(wait(&h.engine).await, IdleOutcome::KeyProblem(KeyStatus::Missing));
    assert!(h.riot.received_requests().await.unwrap_or_default().is_empty());
    // Drop the harness's expect(1) mocks; nothing was supposed to be fetched.
    h.riot.reset().await;
}
