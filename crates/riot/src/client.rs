use std::sync::{Arc, RwLock};
use std::time::Duration;

use bytes::Bytes;
use rc_common::Secret;
use reqwest::header::HeaderValue;
use tokio::time::Instant;
use url::Url;

use crate::dto::{AccountDto, ChampionMasteryDto, LeagueEntryDto, LeagueListDto};
use crate::error::RiotError;
use crate::limiter::{LimitScope, RateLimiter};
use crate::routing::{Platform, Region, RiotId};

const USER_AGENT: &str = concat!("RiftCoach/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Each endpoint is its own method bucket for rate limiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    AccountByRiotId,
    AccountByPuuid,
    MatchIdsByPuuid,
    Match,
    Timeline,
    LeagueEntriesByPuuid,
    MasteryTopByPuuid,
    ActiveGameByPuuid,
    /// Master, Grandmaster and Challenger lists.
    LeagueApex,
    LeagueEntriesByTier,
}

impl Endpoint {
    pub fn method_id(self) -> &'static str {
        match self {
            Self::AccountByRiotId => "account-v1.by-riot-id",
            Self::AccountByPuuid => "account-v1.by-puuid",
            Self::MatchIdsByPuuid => "match-v5.ids-by-puuid",
            Self::Match => "match-v5.match",
            Self::Timeline => "match-v5.timeline",
            Self::LeagueEntriesByPuuid => "league-v4.entries-by-puuid",
            Self::MasteryTopByPuuid => "champion-mastery-v4.top-by-puuid",
            Self::ActiveGameByPuuid => "spectator-v5.active-game",
            Self::LeagueApex => "league-v4.apex",
            Self::LeagueEntriesByTier => "league-v4.entries-by-tier",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Retries after 429s (each waits `Retry-After`).
    pub rate_limited: u32,
    /// Retries after 5xx and network errors (exponential backoff).
    pub server: u32,
    pub base_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { rate_limited: 5, server: 3, base_backoff: Duration::from_secs(1) }
    }
}

/// Paging and filters for Match-V5 `ids` (count is capped at 100 by Riot).
#[derive(Debug, Clone, Default)]
pub struct MatchIdQuery {
    pub start: u32,
    pub count: u32,
    pub queue: Option<i32>,
    pub start_time_s: Option<i64>,
    pub end_time_s: Option<i64>,
}

#[derive(Debug)]
pub struct RiotClient {
    http: reqwest::Client,
    key: RwLock<Option<Secret>>,
    limiter: Arc<RateLimiter>,
    base_override: Option<Url>,
    retry: RetryPolicy,
}

impl RiotClient {
    pub fn new(key: Option<Secret>, limiter: Arc<RateLimiter>) -> Result<Self, RiotError> {
        let http = reqwest::Client::builder().user_agent(USER_AGENT).timeout(REQUEST_TIMEOUT).build()?;
        Ok(Self { http, key: RwLock::new(key), limiter, base_override: None, retry: RetryPolicy::default() })
    }

    /// Send every request to `base` instead of `https://{routing}.api.riotgames.com`
    /// (for tests against a mock server). Routing still selects the rate-limit bucket.
    pub fn with_base_url(mut self, base: Url) -> Self {
        self.base_override = Some(base);
        self
    }

    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn set_key(&self, key: Option<Secret>) {
        *self.key.write().unwrap_or_else(std::sync::PoisonError::into_inner) = key;
    }

    pub fn has_key(&self) -> bool {
        self.current_key().is_some()
    }

    pub fn limiter(&self) -> &Arc<RateLimiter> {
        &self.limiter
    }

    fn current_key(&self) -> Option<Secret> {
        self.key.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    fn url(&self, routing: &str, segments: &[&str], query: &[(&str, String)]) -> Result<Url, RiotError> {
        let base = match &self.base_override {
            Some(u) => u.clone(),
            None => Url::parse(&format!("https://{routing}.api.riotgames.com/"))
                .map_err(|e| RiotError::BadRequest { status: 0, body: e.to_string() })?,
        };
        let mut url = base;
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        Ok(url)
    }

    /// GET with rate limiting and retries. Path segments are percent-encoded.
    pub async fn get(
        &self,
        routing: &str,
        endpoint: Endpoint,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<Bytes, RiotError> {
        let method = endpoint.method_id();
        let url = self.url(routing, segments, query)?;
        let mut rate_limited = 0u32;
        let mut server_errors = 0u32;

        loop {
            let key = self.current_key().ok_or(RiotError::KeyMissing)?;
            let mut token = HeaderValue::from_str(key.expose()).map_err(|_| RiotError::KeyRejected { status: 0 })?;
            token.set_sensitive(true);

            self.limiter.acquire(routing, method).await;
            let started = Instant::now();
            let result = self.http.get(url.clone()).header("X-Riot-Token", token).send().await;

            let resp = match result {
                Ok(r) => r,
                Err(e) if server_errors < self.retry.server => {
                    server_errors += 1;
                    tracing::warn!(method, routing, error = %e, attempt = server_errors, "network error, retrying");
                    tokio::time::sleep(self.backoff(server_errors)).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            let status = resp.status().as_u16();
            let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
            let (app_limit, app_count) = (header("x-app-rate-limit"), header("x-app-rate-limit-count"));
            let (method_limit, method_count) = (header("x-method-rate-limit"), header("x-method-rate-limit-count"));
            let (limit_type, retry_after) = (header("x-rate-limit-type"), header("retry-after"));
            self.limiter.update_from_headers(
                routing,
                method,
                app_limit.as_deref(),
                app_count.as_deref(),
                method_limit.as_deref(),
                method_count.as_deref(),
            );
            tracing::debug!(method, routing, status, elapsed_ms = started.elapsed().as_millis() as u64, "riot request");

            match status {
                200..=299 => return Ok(resp.bytes().await?),
                429 => {
                    rate_limited += 1;
                    let scope = LimitScope::from_header(limit_type.as_deref());
                    let wait = retry_after
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .map(Duration::from_secs)
                        .unwrap_or_else(|| self.backoff(rate_limited));
                    tracing::warn!(
                        method,
                        routing,
                        ?scope,
                        wait_s = wait.as_secs(),
                        attempt = rate_limited,
                        "rate limited (429)"
                    );
                    self.limiter.on_rate_limited(routing, method, scope, wait);
                    if rate_limited > self.retry.rate_limited {
                        return Err(RiotError::RateLimited { attempts: rate_limited });
                    }
                }
                401 | 403 => return Err(RiotError::KeyRejected { status }),
                404 => return Err(RiotError::NotFound),
                500..=599 if server_errors < self.retry.server => {
                    server_errors += 1;
                    tracing::warn!(method, routing, status, attempt = server_errors, "server error, retrying");
                    tokio::time::sleep(self.backoff(server_errors)).await;
                }
                500..=599 => return Err(RiotError::Server { status }),
                _ => {
                    let body = resp.text().await.unwrap_or_default();
                    let body: String = body.chars().take(300).collect();
                    return Err(RiotError::BadRequest { status, body });
                }
            }
        }
    }

    fn backoff(&self, attempt: u32) -> Duration {
        (self.retry.base_backoff * 2u32.saturating_pow(attempt.saturating_sub(1))).min(Duration::from_secs(60))
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        routing: &str,
        endpoint: Endpoint,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<T, RiotError> {
        let body = self.get(routing, endpoint, segments, query).await?;
        Ok(serde_json::from_slice(&body)?)
    }

    // ---- Account-V1 ----

    pub async fn account_by_riot_id(&self, region: Region, id: &RiotId) -> Result<AccountDto, RiotError> {
        let segs = ["riot", "account", "v1", "accounts", "by-riot-id", &id.game_name, &id.tag_line];
        self.get_json(region.as_str(), Endpoint::AccountByRiotId, &segs, &[]).await
    }

    pub async fn account_by_puuid(&self, region: Region, puuid: &str) -> Result<AccountDto, RiotError> {
        let segs = ["riot", "account", "v1", "accounts", "by-puuid", puuid];
        self.get_json(region.as_str(), Endpoint::AccountByPuuid, &segs, &[]).await
    }

    // ---- Match-V5 ----

    pub async fn match_ids(&self, region: Region, puuid: &str, q: &MatchIdQuery) -> Result<Vec<String>, RiotError> {
        let mut query = vec![("start", q.start.to_string()), ("count", q.count.clamp(1, 100).to_string())];
        if let Some(queue) = q.queue {
            query.push(("queue", queue.to_string()));
        }
        if let Some(t) = q.start_time_s {
            query.push(("startTime", t.to_string()));
        }
        if let Some(t) = q.end_time_s {
            query.push(("endTime", t.to_string()));
        }
        let segs = ["lol", "match", "v5", "matches", "by-puuid", puuid, "ids"];
        self.get_json(region.as_str(), Endpoint::MatchIdsByPuuid, &segs, &query).await
    }

    /// Raw match JSON (stored verbatim; parse with [`crate::dto::MatchDto`]).
    pub async fn match_raw(&self, region: Region, match_id: &str) -> Result<Bytes, RiotError> {
        self.get(region.as_str(), Endpoint::Match, &["lol", "match", "v5", "matches", match_id], &[]).await
    }

    /// Raw timeline JSON (stored verbatim; parse with [`crate::dto::TimelineDto`]).
    pub async fn timeline_raw(&self, region: Region, match_id: &str) -> Result<Bytes, RiotError> {
        let segs = ["lol", "match", "v5", "matches", match_id, "timeline"];
        self.get(region.as_str(), Endpoint::Timeline, &segs, &[]).await
    }

    // ---- Platform-routed ----

    pub async fn league_entries(&self, platform: Platform, puuid: &str) -> Result<Vec<LeagueEntryDto>, RiotError> {
        let segs = ["lol", "league", "v4", "entries", "by-puuid", puuid];
        self.get_json(platform.as_str(), Endpoint::LeagueEntriesByPuuid, &segs, &[]).await
    }

    pub async fn mastery_top(
        &self,
        platform: Platform,
        puuid: &str,
        count: u32,
    ) -> Result<Vec<ChampionMasteryDto>, RiotError> {
        let segs = ["lol", "champion-mastery", "v4", "champion-masteries", "by-puuid", puuid, "top"];
        self.get_json(platform.as_str(), Endpoint::MasteryTopByPuuid, &segs, &[("count", count.to_string())]).await
    }

    /// `MASTER`, `GRANDMASTER` or `CHALLENGER` ladder for a queue
    /// (e.g. `RANKED_SOLO_5x5`).
    pub async fn league_apex(&self, platform: Platform, tier: &str, queue: &str) -> Result<LeagueListDto, RiotError> {
        let list = match tier {
            "MASTER" => "masterleagues",
            "GRANDMASTER" => "grandmasterleagues",
            "CHALLENGER" => "challengerleagues",
            other => return Err(RiotError::Invalid(format!("{other} is not an apex tier"))),
        };
        let segs = ["lol", "league", "v4", list, "by-queue", queue];
        self.get_json(platform.as_str(), Endpoint::LeagueApex, &segs, &[]).await
    }

    /// One page (about 205 players) of a tier and division, e.g. DIAMOND I.
    pub async fn league_entries_by_tier(
        &self,
        platform: Platform,
        queue: &str,
        tier: &str,
        division: &str,
        page: u32,
    ) -> Result<Vec<LeagueEntryDto>, RiotError> {
        let segs = ["lol", "league", "v4", "entries", queue, tier, division];
        self.get_json(platform.as_str(), Endpoint::LeagueEntriesByTier, &segs, &[("page", page.to_string())]).await
    }

    /// Current game for this player, or `None` if they aren't in one.
    pub async fn active_game_raw(&self, platform: Platform, puuid: &str) -> Result<Option<Bytes>, RiotError> {
        let segs = ["lol", "spectator", "v5", "active-games", "by-summoner", puuid];
        match self.get(platform.as_str(), Endpoint::ActiveGameByPuuid, &segs, &[]).await {
            Ok(b) => Ok(Some(b)),
            Err(RiotError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
