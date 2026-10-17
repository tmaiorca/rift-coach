//! Cache behavior against a mock Data Dragon. No network.

use rc_ddragon::DdragonCache;
use rc_store::Db;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CHAMPIONS: &str = r#"{"data": {
    "Ahri": {"id": "Ahri", "key": "103", "name": "Ahri", "image": {"full": "Ahri.png"}},
    "MonkeyKing": {"id": "MonkeyKing", "key": "62", "name": "Wukong", "image": {"full": "MonkeyKing.png"}}
}}"#;

async fn mock_ddragon() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/api/versions.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"["16.20.1","16.19.1","lolpatch_3.7"]"#))
        .mount(&server)
        .await;
    Mock::given(path("/cdn/16.20.1/data/en_US/champion.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(CHAMPIONS))
        .expect(1)
        .mount(&server)
        .await;
    for f in ["item.json", "runesReforged.json", "summoner.json"] {
        Mock::given(path(format!("/cdn/16.20.1/data/en_US/{f}")))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/cdn/16.20.1/img/champion/MonkeyKing.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x89, b'P', b'N', b'G']))
        .expect(1)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn caches_versions_data_and_icons() {
    let server = mock_ddragon().await;
    let dir = tempfile::tempdir().expect("tmp");
    let db = Db::open_in_memory().expect("db");
    let cache = DdragonCache::with_base_url(dir.path(), db.clone(), &server.uri()).expect("cache");

    assert_eq!(*cache.versions().await.expect("versions"), vec!["16.20.1", "16.19.1"]);
    let v = cache.version_for_patch("16.20").await.expect("version");
    assert_eq!(v, "16.20.1");

    cache.ensure_version(&v).await.expect("ensure");
    cache.ensure_version(&v).await.expect("second ensure hits cache");
    assert!(db.with(|c| rc_store::ddragon::is_complete(c, "16.20.1")).expect("q"));

    let champs = cache.champions(&v).await.expect("champions");
    assert_eq!(champs.len(), 2);
    assert_eq!(champs.get(62).map(|c| c.name.as_str()), Some("Wukong"));

    let icon = cache.champion_icon(&v, 62).await.expect("icon");
    assert!(icon.ends_with("MonkeyKing.png"));
    assert_eq!(std::fs::read(&icon).expect("read")[1], b'P');
    cache.champion_icon(&v, 62).await.expect("icon from disk");

    assert!(cache.champion_icon(&v, 9999).await.is_err());
    assert!(cache.ensure_version("../../evil").await.is_err());
    // Mock `.expect(1)`s verify each file was downloaded exactly once.
}

/// Regression: concurrent first calls used to race on one temp file.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_use_does_not_race() {
    let server = mock_ddragon().await;
    let dir = tempfile::tempdir().expect("tmp");
    let db = Db::open_in_memory().expect("db");
    let cache = std::sync::Arc::new(DdragonCache::with_base_url(dir.path(), db, &server.uri()).expect("cache"));
    let tasks: Vec<_> = (0..16)
        .map(|_| {
            let cache = cache.clone();
            tokio::spawn(async move { cache.version_for_patch("16.20").await })
        })
        .collect();
    for t in tasks {
        assert_eq!(t.await.expect("join").expect("version"), "16.20.1");
    }
    // This test doesn't download data files or icons.
    server.reset().await;
}

#[tokio::test]
async fn falls_back_to_disk_versions_when_offline() {
    let dir = tempfile::tempdir().expect("tmp");
    let db = Db::open_in_memory().expect("db");
    {
        let server = MockServer::start().await;
        Mock::given(path("/api/versions.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"["16.20.1"]"#))
            .mount(&server)
            .await;
        let cache = DdragonCache::with_base_url(dir.path(), db.clone(), &server.uri()).expect("cache");
        cache.versions().await.expect("online");
        drop(server);
    }
    let down = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&down).await;
    let cache = DdragonCache::with_base_url(dir.path(), db, &down.uri()).expect("cache");
    assert_eq!(cache.versions().await.expect("offline")[0], "16.20.1");
}
