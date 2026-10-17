//! Client behavior against a local mock server. No live API calls.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rc_common::Secret;
use rc_riot::client::RetryPolicy;
use rc_riot::{Platform, RateLimiter, Region, RiotClient, RiotError, RiotId};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "RGAPI-test-key-not-real";

fn client(server: &MockServer) -> RiotClient {
    RiotClient::new(Some(Secret::new(KEY)), Arc::new(RateLimiter::personal_key()))
        .expect("client")
        .with_base_url(server.uri().parse().expect("url"))
        .with_retry_policy(RetryPolicy { rate_limited: 3, server: 3, base_backoff: Duration::from_millis(10) })
}

#[tokio::test]
async fn sends_token_header_and_encodes_riot_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/riot/account/v1/accounts/by-riot-id/Some%20Name/NA1"))
        .and(header("X-Riot-Token", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "puuid": "abc", "gameName": "Some Name", "tagLine": "NA1"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let acct =
        client(&server).account_by_riot_id(Region::Americas, &RiotId::new("Some Name", "NA1")).await.expect("account");
    assert_eq!(acct.puuid, "abc");
    assert_eq!(acct.game_name.as_deref(), Some("Some Name"));
}

#[tokio::test]
async fn honors_retry_after_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(path("/lol/match/v5/matches/NA1_1"))
        .respond_with(
            ResponseTemplate::new(429).insert_header("Retry-After", "1").insert_header("X-Rate-Limit-Type", "method"),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(path("/lol/match/v5/matches/NA1_1"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .with_priority(2)
        .mount(&server)
        .await;

    let start = Instant::now();
    let body = client(&server).match_raw(Region::Americas, "NA1_1").await.expect("match");
    assert_eq!(&body[..], b"{}");
    assert!(start.elapsed() >= Duration::from_secs(1), "waited for Retry-After");
}

#[tokio::test]
async fn gives_up_after_repeated_429s() {
    let server = MockServer::start().await;
    Mock::given(path("/lol/match/v5/matches/NA1_1"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .mount(&server)
        .await;
    let err = client(&server).match_raw(Region::Americas, "NA1_1").await.expect_err("should fail");
    assert!(matches!(err, RiotError::RateLimited { attempts: 4 }), "{err:?}");
}

#[tokio::test]
async fn retries_server_errors() {
    let server = MockServer::start().await;
    Mock::given(path("/lol/match/v5/matches/NA1_1/timeline"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(path("/lol/match/v5/matches/NA1_1/timeline"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\":1}"))
        .with_priority(2)
        .mount(&server)
        .await;
    let body = client(&server).timeline_raw(Region::Americas, "NA1_1").await.expect("timeline");
    assert_eq!(&body[..], b"{\"ok\":1}");
}

#[tokio::test]
async fn maps_status_codes() {
    let server = MockServer::start().await;
    Mock::given(path("/lol/match/v5/matches/NA1_403")).respond_with(ResponseTemplate::new(403)).mount(&server).await;
    Mock::given(path("/lol/match/v5/matches/NA1_404")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    Mock::given(path("/lol/spectator/v5/active-games/by-summoner/p1"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path("/lol/match/v5/matches/NA1_400"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad thing"))
        .mount(&server)
        .await;

    let c = client(&server);
    let rejected = c.match_raw(Region::Americas, "NA1_403").await.expect_err("403");
    assert!(matches!(rejected, RiotError::KeyRejected { status: 403 }));
    assert!(rejected.is_key_problem());
    assert!(matches!(c.match_raw(Region::Americas, "NA1_404").await, Err(RiotError::NotFound)));
    assert!(c.active_game_raw(Platform::Na1, "p1").await.expect("ok").is_none());
    let bad = c.match_raw(Region::Americas, "NA1_400").await.expect_err("400");
    assert!(matches!(bad, RiotError::BadRequest { status: 400, ref body } if body == "bad thing"));
}

#[tokio::test]
async fn missing_key_makes_no_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
    let c = client(&server);
    c.set_key(None);
    assert!(matches!(c.match_raw(Region::Americas, "NA1_1").await, Err(RiotError::KeyMissing)));
}

#[tokio::test]
async fn learns_method_limits_from_headers() {
    let server = MockServer::start().await;
    Mock::given(path("/lol/match/v5/matches/by-puuid/p1/ids"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!(["NA1_1", "NA1_2"]))
                .insert_header("X-App-Rate-Limit", "20:1,100:120")
                .insert_header("X-App-Rate-Limit-Count", "1:1,1:120")
                .insert_header("X-Method-Rate-Limit", "500:10")
                .insert_header("X-Method-Rate-Limit-Count", "1:10"),
        )
        .mount(&server)
        .await;
    let c = client(&server);
    let ids = c
        .match_ids(Region::Americas, "p1", &rc_riot::client::MatchIdQuery { count: 100, ..Default::default() })
        .await
        .expect("ids");
    assert_eq!(ids, vec!["NA1_1", "NA1_2"]);
    let snap = c.limiter().snapshot();
    let method_bucket = snap.iter().find(|b| b.scope == "match-v5.ids-by-puuid").expect("method bucket");
    assert_eq!(method_bucket.windows[0].limit, 500);
    assert_eq!(method_bucket.windows[0].used, 1);
}

#[test]
fn errors_and_debug_output_never_contain_the_key() {
    let c = RiotClient::new(Some(Secret::new(KEY)), Arc::new(RateLimiter::personal_key())).expect("client");
    let dbg = format!("{c:?}");
    assert!(!dbg.contains(KEY), "client Debug leaked key: {dbg}");
    assert!(dbg.contains("[REDACTED]"));
}
