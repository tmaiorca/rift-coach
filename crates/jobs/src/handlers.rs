//! Job implementations. Each is idempotent: re-running a finished job only
//! re-checks what's stored.

use std::sync::Arc;

use rc_riot::client::MatchIdQuery;
use rc_riot::dto::{MatchDto, TimelineDto, patch_of};
use rc_riot::{Platform, RiotError, RiotId};
use rc_store::accounts::{self, PuuidOutcome};
use rc_store::matches::{self, NewMatch, NewParticipant, RawKind};

use crate::engine::{Engine, EngineEvent, KeyStatus};
use crate::job::Job;

/// Riot caps Match-V5 `ids` at 100 per page.
const MATCH_PAGE: u32 = 100;
/// Auto reviews only cover games that ended this recently.
const AUTO_REVIEW_MAX_AGE_MS: i64 = 6 * 3600 * 1000;
/// Auto session reviews only cover sessions that ended this recently.
const AUTO_SESSION_MAX_AGE_MS: i64 = 24 * 3600 * 1000;
/// Auto weekly reports only cover a week that ended this recently.
const AUTO_WEEKLY_MAX_AGE_MS: i64 = 3 * 24 * 3600 * 1000;
/// Weekly reports need this many Summoner's Rift games in the week.
const WEEKLY_MIN_GAMES: usize = 3;
/// Ranks older than this are refreshed.
const RANK_MAX_AGE_MS: i64 = 6 * 3600 * 1000;
/// A live game is the Match-V5 game that started within this of it (the
/// live start time is estimated from the game clock).
const LIVE_MATCH_WINDOW_MS: i64 = 5 * 60 * 1000;
const DAY_MS: i64 = 24 * 3600 * 1000;

#[derive(Debug)]
pub(crate) enum JobError {
    /// Needs a working API key; requeued without burning an attempt.
    Key(KeyStatus),
    /// Transient; retried with backoff.
    Retry(String),
    /// Retrying won't help.
    Permanent(String),
}

impl From<RiotError> for JobError {
    fn from(e: RiotError) -> Self {
        match e {
            RiotError::KeyMissing => Self::Key(KeyStatus::Missing),
            RiotError::KeyRejected { .. } => Self::Key(KeyStatus::Rejected),
            // Usually a match that isn't published yet.
            RiotError::NotFound => Self::Retry("not found (yet)".into()),
            e if e.is_transient() => Self::Retry(e.to_string()),
            e => Self::Permanent(e.to_string()),
        }
    }
}

impl From<rc_store::StoreError> for JobError {
    fn from(e: rc_store::StoreError) -> Self {
        Self::Retry(format!("storage: {e}"))
    }
}

impl From<rc_ddragon::DdragonError> for JobError {
    fn from(e: rc_ddragon::DdragonError) -> Self {
        Self::Retry(format!("Data Dragon: {e}"))
    }
}

type JobResult = Result<(), JobError>;

impl Engine {
    pub(crate) async fn handle(&self, job: &Job) -> JobResult {
        match job {
            Job::ResolveAccount { account_id, then_sync } => self.resolve_account(*account_id, *then_sync).await,
            Job::SyncMatchList { account_id, count } => self.sync_match_list(*account_id, *count).await,
            Job::FetchMatch { match_id } => self.fetch_match(match_id).await,
            Job::FetchTimeline { match_id } => self.fetch_timeline(match_id).await,
            Job::DdragonSync { patch } => self.ddragon_sync(patch).await,
            Job::ComputeMetrics { match_id } => self.compute_metrics(match_id).await,
            Job::AiReview { account_id, match_id } => self.ai_review(*account_id, match_id).await,
            Job::SessionReview { account_id, scope } => self.session_review(*account_id, scope).await,
            Job::PostGame { account_id, live_game_id } => self.post_game(*account_id, *live_game_id).await,
            Job::FetchRank { account_id } => self.fetch_rank(*account_id).await,
            Job::WeeklyReport { account_id, week } => self.weekly_report(*account_id, week).await,
        }
    }

    async fn fetch_rank(&self, account_id: i64) -> JobResult {
        let (account, puuid) = self
            .db
            .call(move |c| Ok((accounts::get_account(c, account_id)?, accounts::current_puuid(c, account_id)?)))
            .await?;
        let account = account.ok_or_else(|| JobError::Permanent(format!("account {account_id} no longer exists")))?;
        let Some(puuid) = puuid else {
            self.enqueue(Job::ResolveAccount { account_id, then_sync: None }).await?;
            return Err(JobError::Retry("resolving the account first".into()));
        };
        let platform: Platform = account.platform.parse().map_err(JobError::Permanent)?;
        let entries = self.riot.league_entries(platform, &puuid).await?;
        let now = rc_common::time::now_ms();
        let ranks: Vec<rc_store::bench::Rank> = entries
            .into_iter()
            .filter(|e| matches!(e.queue_type.as_str(), "RANKED_SOLO_5x5" | "RANKED_FLEX_SR"))
            .map(|e| rc_store::bench::Rank {
                queue_type: e.queue_type,
                tier: e.tier,
                division: e.rank,
                lp: e.league_points,
                wins: e.wins,
                losses: e.losses,
                fetched_at: now,
            })
            .collect();
        self.db.call(move |c| rc_store::bench::set_ranks(c, account_id, &ranks)).await?;
        Ok(())
    }

    async fn weekly_report(&self, account_id: i64, week: &str) -> JobResult {
        let settings = self.db.call(|c| rc_store::settings::app(c)).await?;
        let client =
            rc_coach::provider::client_from_settings(&settings.ai).map_err(|e| JobError::Permanent(e.to_string()))?;
        let opts = rc_coach::review::ReviewOptions::from_settings(&settings, false);
        match rc_coach::review::weekly_report(&self.db, &client, account_id, week, &opts).await {
            Ok(report) => {
                self.emit(EngineEvent::WeeklyReportReady { week: week.to_owned(), report_id: report.id });
                Ok(())
            }
            Err(e) => Err(review_error(e)),
        }
    }

    async fn post_game(&self, account_id: i64, live_game_id: i64) -> JobResult {
        let live = self
            .db
            .call(move |c| rc_store::live::get(c, live_game_id))
            .await?
            .ok_or_else(|| JobError::Permanent(format!("live game {live_game_id} no longer exists")))?;
        if live.match_id.is_some() || self.link_live_game(account_id, &live).await? {
            return Ok(());
        }
        let (account, puuid) = self
            .db
            .call(move |c| Ok((accounts::get_account(c, account_id)?, accounts::current_puuid(c, account_id)?)))
            .await?;
        let account = account.ok_or_else(|| JobError::Permanent(format!("account {account_id} no longer exists")))?;
        let Some(puuid) = puuid else {
            self.enqueue(Job::ResolveAccount { account_id, then_sync: None }).await?;
            return Err(JobError::Retry("resolving your account first".into()));
        };
        let platform: Platform = account.platform.parse().map_err(JobError::Permanent)?;
        let query = MatchIdQuery {
            start: 0,
            count: 5,
            start_time_s: Some((live.started_at - 30 * 60_000) / 1000),
            ..Default::default()
        };
        for id in self.riot.match_ids(platform.match_region(), &puuid, &query).await? {
            // Idempotent: stores the match and queues its timeline and metrics.
            self.fetch_match(&id).await?;
        }
        if self.link_live_game(account_id, &live).await? {
            Ok(())
        } else {
            Err(JobError::Retry("Riot hasn't published the game yet".into()))
        }
    }

    /// Links a live game to the stored match that started at about the same
    /// time. Returns whether it found one.
    async fn link_live_game(&self, account_id: i64, live: &rc_store::live::LiveGame) -> Result<bool, JobError> {
        let (live_game_id, started_at) = (live.id, live.started_at);
        let found = self
            .db
            .call(move |c| {
                let m = rc_store::live::unlinked_match_near(c, account_id, started_at, LIVE_MATCH_WINDOW_MS)?;
                let Some(m) = m else { return Ok(None) };
                rc_store::live::link_match(c, live_game_id, &m)?;
                let has_timeline = matches::has_timeline(c, &m)?;
                Ok(Some((m, has_timeline)))
            })
            .await?;
        let Some((match_id, has_timeline)) = found else { return Ok(false) };
        tracing::info!(live_game_id, match_id, "live game linked");
        self.emit(EngineEvent::LiveGameLinked { live_game_id, match_id: match_id.clone() });
        // A sync may have stored it already; make sure it gets analyzed.
        self.enqueue(if has_timeline { Job::ComputeMetrics { match_id } } else { Job::FetchTimeline { match_id } })
            .await?;
        Ok(true)
    }

    /// Marks a live game's match analyzed and announces it (once).
    async fn maybe_post_game_ready(&self, match_id: &str, review_queued: bool) -> JobResult {
        let id = match_id.to_owned();
        let marked = self
            .db
            .call(move |c| {
                let Some(g) = rc_store::live::by_match(c, &id)? else { return Ok(None) };
                Ok(rc_store::live::mark_analyzed(c, g.id, rc_common::time::now_ms())?.then_some(g.id))
            })
            .await?;
        if let Some(live_game_id) = marked {
            self.emit(EngineEvent::PostGameReady { live_game_id, match_id: match_id.to_owned(), review_queued });
            self.prune_live_snapshots().await?;
        }
        Ok(())
    }

    /// Deletes live snapshots past the retention setting.
    pub(crate) async fn prune_live_snapshots(&self) -> Result<usize, rc_store::StoreError> {
        self.db
            .call(|c| {
                let days = i64::from(rc_store::settings::app(c)?.live.keep_snapshots_days);
                let now = rc_common::time::now_ms();
                rc_store::live::prune_snapshots(c, now - days * DAY_MS, now - days.max(1) * DAY_MS)
            })
            .await
    }

    async fn resolve_account(&self, account_id: i64, then_sync: Option<u32>) -> JobResult {
        let account = self
            .db
            .call(move |c| accounts::get_account(c, account_id))
            .await?
            .ok_or_else(|| JobError::Permanent(format!("account {account_id} no longer exists")))?;
        let platform: Platform = account.platform.parse().map_err(JobError::Permanent)?;
        let riot_id = RiotId::new(&account.game_name, &account.tag_line);

        let dto = match self.riot.account_by_riot_id(platform.account_region(), &riot_id).await {
            Err(RiotError::NotFound) => {
                return Err(JobError::Permanent(format!("Riot ID {riot_id} not found")));
            }
            other => other?,
        };

        let puuid = dto.puuid.clone();
        let outcome = self
            .db
            .call(move |c| {
                if let (Some(name), Some(tag)) = (dto.game_name.as_deref(), dto.tag_line.as_deref()) {
                    accounts::update_riot_id(c, account_id, name, tag)?;
                }
                accounts::record_resolved_puuid(c, account_id, &puuid)
            })
            .await?;

        if let PuuidOutcome::KeyChanged { key_gen_id } = outcome {
            tracing::warn!(key_gen_id, "PUUID changed: API key changed; re-resolving all tracked accounts");
            self.emit(EngineEvent::KeyGenerationChanged { key_gen_id });
            let others: Vec<_> = self
                .db
                .call(|c| accounts::list_accounts(c))
                .await?
                .into_iter()
                .filter(|a| a.id != account_id)
                .map(|a| Job::ResolveAccount { account_id: a.id, then_sync: None })
                .collect();
            self.enqueue_many(others).await?;
        }
        if let Some(count) = then_sync {
            self.enqueue(Job::SyncMatchList { account_id, count }).await?;
        }
        Ok(())
    }

    async fn sync_match_list(&self, account_id: i64, count: u32) -> JobResult {
        let (account, puuid) = self
            .db
            .call(move |c| Ok((accounts::get_account(c, account_id)?, accounts::current_puuid(c, account_id)?)))
            .await?;
        let account = account.ok_or_else(|| JobError::Permanent(format!("account {account_id} no longer exists")))?;
        let Some(puuid) = puuid else {
            // Not resolved under the current key yet.
            self.enqueue(Job::ResolveAccount { account_id, then_sync: Some(count) }).await?;
            return Ok(());
        };
        let platform: Platform = account.platform.parse().map_err(JobError::Permanent)?;

        let mut ids = Vec::new();
        let mut start = 0;
        while start < count {
            let page = MATCH_PAGE.min(count - start);
            let got = self
                .riot
                .match_ids(platform.match_region(), &puuid, &MatchIdQuery { start, count: page, ..Default::default() })
                .await?;
            let done = (got.len() as u32) < page;
            ids.extend(got);
            if done {
                break;
            }
            start += page;
        }

        // Queue what's missing; link matches already stored (e.g. from
        // another tracked account) to this one.
        let total = ids.len();
        let jobs = self
            .db
            .call(move |c| {
                let mut jobs = Vec::new();
                for id in ids {
                    if !matches::has_match(c, &id)? {
                        jobs.push(Job::FetchMatch { match_id: id });
                    } else {
                        matches::link_tracked_accounts(c, &id)?;
                        if !matches::has_timeline(c, &id)? {
                            jobs.push(Job::FetchTimeline { match_id: id });
                        }
                    }
                }
                Ok(jobs)
            })
            .await?;
        let queued = self.enqueue_many(jobs).await?;
        self.enqueue(Job::FetchRank { account_id }).await?;
        tracing::info!(account_id, listed = total, queued, "match list synced");
        Ok(())
    }

    async fn fetch_match(&self, match_id: &str) -> JobResult {
        let id = match_id.to_owned();
        if self.db.call(move |c| matches::has_match(c, &id)).await? {
            let id = match_id.to_owned();
            let need_timeline = self
                .db
                .call(move |c| {
                    matches::link_tracked_accounts(c, &id)?;
                    Ok(!matches::has_timeline(c, &id)?)
                })
                .await?;
            if need_timeline {
                self.enqueue(Job::FetchTimeline { match_id: match_id.to_owned() }).await?;
            }
            return Ok(());
        }

        let platform = Platform::from_match_id(match_id)
            .ok_or_else(|| JobError::Permanent(format!("can't route match id '{match_id}'")))?;
        let raw = self.riot.match_raw(platform.match_region(), match_id).await?;
        let dto: MatchDto =
            serde_json::from_slice(&raw).map_err(|e| JobError::Permanent(format!("unparseable match: {e}")))?;
        if dto.metadata.match_id != match_id {
            return Err(JobError::Permanent(format!("asked for {match_id}, got {}", dto.metadata.match_id)));
        }

        let (new_match, participants) = to_rows(&dto, platform);
        let patch = new_match.patch.clone();
        let id = match_id.to_owned();
        self.db
            .call(move |c| {
                let key_gen = accounts::latest_key_gen(c)?.ok_or_else(|| {
                    rc_store::StoreError::Io(std::io::Error::other("no key generation yet; resolve an account first"))
                })?;
                matches::insert_match(c, &new_match, &participants, key_gen, &raw)?;
                matches::link_tracked_accounts(c, &id)?;
                Ok(())
            })
            .await?;
        self.emit(EngineEvent::MatchStored { match_id: match_id.to_owned() });
        self.enqueue_many(vec![
            Job::FetchTimeline { match_id: match_id.to_owned() },
            Job::DdragonSync { patch },
            Job::ComputeMetrics { match_id: match_id.to_owned() },
        ])
        .await?;
        Ok(())
    }

    async fn fetch_timeline(&self, match_id: &str) -> JobResult {
        let id = match_id.to_owned();
        if self.db.call(move |c| matches::has_timeline(c, &id)).await? {
            return Ok(());
        }
        let platform = Platform::from_match_id(match_id)
            .ok_or_else(|| JobError::Permanent(format!("can't route match id '{match_id}'")))?;
        let raw = self.riot.timeline_raw(platform.match_region(), match_id).await?;
        let dto: TimelineDto =
            serde_json::from_slice(&raw).map_err(|e| JobError::Permanent(format!("unparseable timeline: {e}")))?;
        if dto.metadata.match_id != match_id {
            return Err(JobError::Permanent(format!("asked for {match_id} timeline, got {}", dto.metadata.match_id)));
        }
        let id = match_id.to_owned();
        self.db.call(move |c| matches::insert_timeline(c, &id, &raw)).await?;
        tracing::debug!(match_id, frames = dto.info.frames.len(), "timeline stored");
        self.emit(EngineEvent::TimelineStored { match_id: match_id.to_owned() });
        self.enqueue(Job::ComputeMetrics { match_id: match_id.to_owned() }).await?;
        Ok(())
    }

    async fn compute_metrics(&self, match_id: &str) -> JobResult {
        let id = match_id.to_owned();
        let loaded = self
            .db
            .call(move |c| {
                let raw = matches::get_raw(c, &id, RawKind::Match)?;
                let timeline = matches::get_raw(c, &id, RawKind::Timeline)?;
                Ok(raw.map(|r| (r, timeline)))
            })
            .await?;
        let Some((raw, timeline_raw)) = loaded else {
            return Err(JobError::Permanent(format!("match {match_id} is not stored")));
        };
        let game = rc_metrics::parse_match(&raw).map_err(|e| JobError::Permanent(e.to_string()))?;
        let timeline = timeline_raw
            .map(|t| rc_metrics::parse_timeline(&t))
            .transpose()
            .map_err(|e| JobError::Permanent(e.to_string()))?;
        let patch = self
            .db
            .call({
                let id = match_id.to_owned();
                move |c| matches::patch_of_match(c, &id)
            })
            .await?
            .unwrap_or_default();
        // Item prices only feed first-back gold and first-item timing. With
        // Data Dragon unreachable, store the rest under version 0 so the next
        // start recomputes the match in full.
        let (items, version) = match self.item_catalog(&patch).await {
            Ok(items) => (items, rc_metrics::METRICS_VERSION),
            Err(e) => {
                tracing::warn!(match_id, error = %e, "no item data; computing metrics without item timings");
                (Arc::new(rc_metrics::ItemCatalog::default()), 0)
            }
        };
        let metrics = rc_metrics::compute(&game, timeline.as_ref(), &items);
        let deaths = timeline.as_ref().map(|tl| death_rows(&game, tl)).unwrap_or_default();
        let id = match_id.to_owned();
        self.db
            .call(move |c| rc_store::metrics::save_match_metrics(c, &id, version, &metrics.by_participant, &deaths))
            .await?;
        let id = match_id.to_owned();
        let benchmark = self
            .db
            .call(move |c| {
                Ok(c.query_row("SELECT benchmark_tier IS NOT NULL FROM matches WHERE match_id = ?1", [&id], |r| {
                    r.get(0)
                })?)
            })
            .await?;
        self.emit(EngineEvent::MetricsComputed { match_id: match_id.to_owned(), benchmark });
        if timeline.is_some() {
            let review_queued = self.maybe_queue_auto_review(match_id).await?;
            self.maybe_post_game_ready(match_id, review_queued).await?;
        }
        Ok(())
    }

    /// Queues an automatic review for a fresh game of the active account if
    /// the setting is on. Never for backfilled history. Returns whether it did.
    async fn maybe_queue_auto_review(&self, match_id: &str) -> Result<bool, JobError> {
        let id = match_id.to_owned();
        let target = self
            .db
            .call(move |c| {
                let s = rc_store::settings::app(c)?;
                if !s.ai.auto_post_game {
                    return Ok(None);
                }
                let account = match s.active_account_id {
                    Some(a) => accounts::get_account(c, a)?,
                    None => accounts::primary_account(c)?,
                };
                let Some(account) = account else { return Ok(None) };
                let Some((summary, _)) = matches::account_match(c, account.id, &id)? else { return Ok(None) };
                let ended = summary.game_start_ms + summary.duration_s * 1000;
                let fresh = rc_common::time::now_ms() - ended < AUTO_REVIEW_MAX_AGE_MS;
                let eligible = fresh
                    && !summary.is_remake
                    && rc_common::queues::is_standard_rift(summary.queue_id, summary.map_id)
                    && !rc_store::reports::has_report(c, rc_coach::detail::POST_GAME, account.id, &id)?;
                Ok(eligible.then_some(account.id))
            })
            .await?;
        if let Some(account_id) = target {
            self.enqueue(Job::AiReview { account_id, match_id: match_id.to_owned() }).await?;
        }
        Ok(target.is_some())
    }

    async fn ai_review(&self, account_id: i64, match_id: &str) -> JobResult {
        let settings = self.db.call(|c| rc_store::settings::app(c)).await?;
        let client =
            rc_coach::provider::client_from_settings(&settings.ai).map_err(|e| JobError::Permanent(e.to_string()))?;
        let id = match_id.to_owned();
        let patch = self.db.call(move |c| matches::patch_of_match(c, &id)).await?.unwrap_or_default();
        let items = self.item_catalog(&patch).await.ok();
        let opts = rc_coach::review::ReviewOptions::from_settings(&settings, false);
        match rc_coach::review::post_game_review(&self.db, &client, account_id, match_id, &opts, items).await {
            Ok(report) => {
                let (id, report_id) = (match_id.to_owned(), report.id);
                self.db
                    .call(move |c| {
                        if let Some(g) = rc_store::live::by_match(c, &id)?.filter(|g| g.report_id.is_none()) {
                            rc_store::live::set_report(c, g.id, report_id)?;
                        }
                        Ok(())
                    })
                    .await?;
                self.emit(EngineEvent::ReviewReady { match_id: match_id.to_owned(), report_id: report.id });
                Ok(())
            }
            Err(e) => Err(review_error(e)),
        }
    }

    async fn session_review(&self, account_id: i64, scope: &str) -> JobResult {
        let settings = self.db.call(|c| rc_store::settings::app(c)).await?;
        let client =
            rc_coach::provider::client_from_settings(&settings.ai).map_err(|e| JobError::Permanent(e.to_string()))?;
        let opts = rc_coach::review::ReviewOptions::from_settings(&settings, false);
        match rc_coach::review::session_review(&self.db, &client, account_id, scope, &opts).await {
            Ok(report) => {
                self.emit(EngineEvent::SessionReviewReady { scope: scope.to_owned(), report_id: report.id });
                Ok(())
            }
            Err(e) => Err(review_error(e)),
        }
    }

    /// Queues a review of the active account's latest session once it has
    /// ended, if the setting is on. Only sessions that ended in the last day
    /// and had at least 2 Summoner's Rift games.
    pub(crate) async fn maybe_queue_session_review(&self) -> JobResult {
        let target = self
            .db
            .call(|c| {
                let s = rc_store::settings::app(c)?;
                if !s.ai.auto_session {
                    return Ok(None);
                }
                let account = match s.active_account_id {
                    Some(a) => accounts::get_account(c, a)?,
                    None => accounts::primary_account(c)?,
                };
                let Some(account) = account else { return Ok(None) };
                let now = rc_common::time::now_ms();
                let Some(session) = rc_coach::sessions::latest_ended(c, account.id, s.session_gap_min, now)? else {
                    return Ok(None);
                };
                let eligible =
                    session.rift_games >= 2 && !session.has_report && now - session.end_ms < AUTO_SESSION_MAX_AGE_MS;
                Ok(eligible.then_some((account.id, session.scope)))
            })
            .await?;
        if let Some((account_id, scope)) = target {
            self.enqueue(Job::SessionReview { account_id, scope }).await?;
        }
        Ok(())
    }

    /// Queues the weekly report for the week that just ended, if the setting
    /// is on and the week had enough games.
    pub(crate) async fn maybe_queue_weekly_report(&self) -> JobResult {
        let target = self
            .db
            .call(|c| {
                let s = rc_store::settings::app(c)?;
                if !s.ai.auto_weekly {
                    return Ok(None);
                }
                let account = match s.active_account_id {
                    Some(a) => accounts::get_account(c, a)?,
                    None => accounts::primary_account(c)?,
                };
                let Some(account) = account else { return Ok(None) };
                let now = rc_common::time::now_ms();
                let Some(week) = rc_coach::weekly::last_complete_week(now) else { return Ok(None) };
                let summary = rc_coach::weekly::list(c, account.id, 3, now)?.into_iter().find(|w| w.week == week);
                let eligible = summary.is_some_and(|w| {
                    w.rift_games >= WEEKLY_MIN_GAMES && !w.has_report && now - w.end_ms < AUTO_WEEKLY_MAX_AGE_MS
                });
                Ok(eligible.then_some((account.id, week)))
            })
            .await?;
        if let Some((account_id, week)) = target {
            self.enqueue(Job::WeeklyReport { account_id, week }).await?;
        }
        Ok(())
    }

    /// Queues a rank refresh for tracked accounts whose rank is stale.
    pub(crate) async fn refresh_stale_ranks(&self) -> JobResult {
        let stale = self
            .db
            .call(|c| {
                let now = rc_common::time::now_ms();
                let mut out = Vec::new();
                for a in accounts::list_accounts(c)? {
                    let at = rc_store::bench::ranks_fetched_at(c, a.id)?;
                    if at.is_none_or(|t| now - t > RANK_MAX_AGE_MS) && accounts::current_puuid(c, a.id)?.is_some() {
                        out.push(Job::FetchRank { account_id: a.id });
                    }
                }
                Ok(out)
            })
            .await?;
        self.enqueue_many(stale).await?;
        Ok(())
    }

    async fn ddragon_sync(&self, patch: &str) -> JobResult {
        let version = self.ddragon.version_for_patch(patch).await?;
        self.ddragon.ensure_version(&version).await?;
        Ok(())
    }
}

fn review_error(e: rc_coach::CoachError) -> JobError {
    match e {
        rc_coach::CoachError::Ai(e) if e.is_transient() => JobError::Retry(e.to_string()),
        e => JobError::Permanent(e.to_string()),
    }
}

/// Every champion death in the match (all ten players) for `death_events`.
fn death_rows(game: &rc_metrics::model::Match, tl: &rc_metrics::model::Timeline) -> Vec<rc_store::metrics::DeathRow> {
    let champ = |pid: Option<i32>| {
        pid.and_then(|id| game.info.participants.iter().find(|p| p.participant_id == id))
            .map(|p| p.champion_name.clone())
    };
    game.info
        .participants
        .iter()
        .flat_map(|p| {
            rc_metrics::detail::deaths_of(game, tl, p).into_iter().map(|d| rc_store::metrics::DeathRow {
                participant_id: p.participant_id,
                t_ms: (d.t_s * 1000.0).round() as i64,
                x: d.x,
                y: d.y,
                zone: d.zone.as_str().to_owned(),
                enemies: i32::try_from(d.enemies_involved).unwrap_or(i32::MAX),
                solo: d.solo,
                before_14: d.before_14,
                enemy_side: d.in_enemy_half,
                gold_vs_opponent: d.gold_vs_opponent,
                unspent_gold: d.unspent_gold,
                shutdown_bounty: d.shutdown_bounty,
                enemy_objective: d.enemy_objective.map(str::to_owned),
                killer_champion: champ(d.killer_id.filter(|k| (1..=10).contains(k))),
            })
        })
        .collect()
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

pub(crate) fn to_rows(dto: &MatchDto, platform: Platform) -> (NewMatch, Vec<NewParticipant>) {
    let info = &dto.info;
    let new_match = NewMatch {
        match_id: dto.metadata.match_id.clone(),
        platform: platform.as_str().to_owned(),
        queue_id: info.queue_id,
        map_id: info.map_id,
        game_mode: info.game_mode.clone(),
        game_type: info.game_type.clone(),
        game_version: info.game_version.clone(),
        patch: patch_of(&info.game_version),
        game_start_ms: info.start_ms(),
        duration_s: info.duration_seconds(),
        is_remake: info.is_remake(),
    };
    let participants = info
        .participants
        .iter()
        .map(|p| NewParticipant {
            participant_id: p.participant_id,
            puuid: p.puuid.clone(),
            riot_id_name: p.riot_id_game_name.clone().filter(|s| !s.is_empty()),
            riot_id_tag: p.riot_id_tagline.clone().filter(|s| !s.is_empty()),
            team_id: p.team_id,
            champion_id: p.champion_id,
            champion_name: p.champion_name.clone(),
            team_position: non_empty(&p.team_position),
            individual_position: non_empty(&p.individual_position),
            win: p.win,
            kills: p.kills,
            deaths: p.deaths,
            assists: p.assists,
            champ_level: p.champ_level,
            gold_earned: p.gold_earned,
            cs: p.total_minions_killed + p.neutral_minions_killed,
            vision_score: p.vision_score,
        })
        .collect();
    (new_match, participants)
}
