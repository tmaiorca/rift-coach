//! HTTP client for the Live Client Data API on `https://127.0.0.1:2999`.
//!
//! The game serves it with a certificate from Riot's own root CA that has no
//! usable subject name for `127.0.0.1`, so normal verification fails. This
//! client therefore skips certificate checks, and to keep that safe it only
//! ever talks to a loopback address and never sends credentials.

use std::time::Duration;

use url::Url;

use crate::model::{AllGameData, OwnSnapshot};

pub const DEFAULT_URL: &str = "https://127.0.0.1:2999";
/// Short: it's localhost, and a slow answer means the game is busy loading.
const TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("the live client URL must be a loopback address (got {0})")]
    NotLoopback(String),
    #[error("invalid live client URL: {0}")]
    BadUrl(#[from] url::ParseError),
    #[error("http client: {0}")]
    Http(#[from] reqwest::Error),
}

/// What one poll saw.
#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    /// Nothing listening: no game running.
    Absent,
    /// The game answers but has no player data yet (loading screen), or
    /// is a spectator/replay session without an active player.
    NotReady,
    Data(Box<OwnSnapshot>),
}

pub struct LiveClient {
    http: reqwest::Client,
    base: Url,
}

impl LiveClient {
    pub fn new(base: &str) -> Result<Self, LiveError> {
        let base = Url::parse(base)?;
        let loopback = match base.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
            None => false,
        };
        if !loopback {
            return Err(LiveError::NotLoopback(base.to_string()));
        }
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .pool_max_idle_per_host(1)
            .build()?;
        Ok(Self { http, base })
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    pub async fn poll(&self) -> Observation {
        let Ok(url) = self.base.join("/liveclientdata/allgamedata") else { return Observation::Absent };
        let resp = match self.http.get(url).send().await {
            Ok(r) => r,
            // Refused or timed out: no game (or it's starting up).
            Err(e) if e.is_connect() || e.is_timeout() => return Observation::Absent,
            Err(e) => {
                tracing::debug!(error = %e, "live client request failed");
                return Observation::Absent;
            }
        };
        if !resp.status().is_success() {
            return Observation::NotReady;
        }
        match resp.json::<AllGameData>().await {
            Ok(d) => d.own().map_or(Observation::NotReady, |s| Observation::Data(Box::new(s))),
            Err(e) => {
                tracing::debug!(error = %e, "unreadable live client data");
                Observation::NotReady
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn refuses_non_loopback() {
        assert!(matches!(LiveClient::new("https://example.com:2999"), Err(LiveError::NotLoopback(_))));
        assert!(LiveClient::new(DEFAULT_URL).is_ok());
        assert!(LiveClient::new("http://localhost:2998").is_ok());
    }

    #[tokio::test]
    async fn observes_absent_loading_and_data() {
        // Nothing on this port.
        let c = LiveClient::new("http://127.0.0.1:9").expect("client");
        assert_eq!(c.poll().await, Observation::Absent);

        let server = MockServer::start().await;
        let c = LiveClient::new(&server.uri()).expect("client");
        Mock::given(method("GET"))
            .and(path("/liveclientdata/allgamedata"))
            .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"errorCode":"RESOURCE_NOT_FOUND"}"#))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        assert_eq!(c.poll().await, Observation::NotReady);

        let game = crate::mock::MockGame::default();
        Mock::given(method("GET"))
            .and(path("/liveclientdata/allgamedata"))
            .respond_with(ResponseTemplate::new(200).set_body_json(game.all_game_data(600.0)))
            .mount(&server)
            .await;
        let Observation::Data(s) = c.poll().await else { panic!("expected data") };
        assert_eq!(s.riot_id, "TapRooted#BTC");
        assert_eq!(s.champion_ddragon_id.as_deref(), Some("Leblanc"));
        assert_eq!(s.game_time_s, 600.0);
        assert!(s.scores.creep_score > 50);
        assert!(s.game_end.is_none());
    }
}
