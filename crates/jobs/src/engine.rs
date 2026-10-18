use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rc_common::Secret;
use rc_common::time::now_ms;
use rc_ddragon::{DdragonCache, DdragonError};
use rc_metrics::ItemCatalog;
use rc_riot::limiter::BucketSnapshot;
use rc_riot::{Platform, RiotClient, RiotId};
use rc_store::jobs::{self as q, ClaimedJob, FailedJob, JobCounts, NewJob};
use rc_store::{Db, StoreError};
use serde::Serialize;
use tokio::sync::{Notify, broadcast};
use tokio::task::JoinHandle;

use crate::handlers::JobError;
use crate::job::Job;

/// How often to look for a finished session to review.
const SESSION_CHECK_EVERY: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Concurrent workers. The rate limiter is the real throttle; a few
    /// workers just keep requests in flight while others wait on it.
    pub workers: usize,
    /// Longest a worker sleeps without checking the queue.
    pub idle_poll: Duration,
    /// First retry delay; doubles per attempt up to `max_backoff`.
    pub base_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            workers: 3,
            idle_poll: Duration::from_secs(30),
            base_backoff: Duration::from_secs(15),
            max_backoff: Duration::from_secs(30 * 60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    /// No call made yet with the current key.
    Unknown,
    Ok,
    Missing,
    /// 401/403: invalid, revoked, or expired. Work pauses until the key changes.
    Rejected,
}

impl KeyStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "loaded",
            Self::Ok => "OK",
            Self::Missing => "missing",
            Self::Rejected => "rejected",
        }
    }

    fn blocks_work(self) -> bool {
        matches!(self, Self::Missing | Self::Rejected)
    }
}

/// Progress notifications for the UI and CLI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    JobStarted {
        kind: String,
        label: String,
    },
    JobSucceeded {
        kind: String,
        label: String,
    },
    JobRetrying {
        kind: String,
        label: String,
        error: String,
        retry_in_s: u64,
    },
    JobFailed {
        kind: String,
        label: String,
        error: String,
    },
    MatchStored {
        match_id: String,
    },
    TimelineStored {
        match_id: String,
    },
    KeyStatusChanged {
        status: KeyStatus,
    },
    KeyGenerationChanged {
        key_gen_id: i64,
    },
    MetricsComputed {
        match_id: String,
        /// A collected benchmark game (the UI ignores these).
        benchmark: bool,
    },
    ReviewReady {
        match_id: String,
        report_id: i64,
    },
    SessionReviewReady {
        scope: String,
        report_id: i64,
    },
    /// A game seen live was found in Match-V5.
    LiveGameLinked {
        live_game_id: i64,
        match_id: String,
    },
    /// A game seen live is fully analyzed (metrics with timeline).
    /// `review_queued`: an automatic AI review follows.
    PostGameReady {
        live_game_id: i64,
        match_id: String,
        review_queued: bool,
    },
    /// Background work paused or resumed because a game is running.
    InGameChanged {
        in_game: bool,
    },
    WeeklyReportReady {
        week: String,
        report_id: i64,
    },
    /// The loading-screen brief (and AI game plan, if on) is ready.
    PregameReady {
        live_game_id: Option<i64>,
    },
}

impl EngineEvent {
    /// A line for the persisted activity feed, if this event deserves one.
    pub fn describe(&self) -> Option<(&'static str, String)> {
        Some(match self {
            Self::MatchStored { match_id } => ("info", format!("Stored match {match_id}")),
            Self::TimelineStored { match_id } => ("info", format!("Stored timeline {match_id}")),
            // Waiting for Riot to publish a game is expected, not a warning.
            Self::JobRetrying { kind, retry_in_s, .. } if kind == "post_game" => {
                ("info", format!("Waiting for Riot to publish your last game (checking again in {retry_in_s}s)"))
            }
            Self::JobRetrying { label, error, retry_in_s, .. } => {
                ("warn", format!("Retrying {label} in {retry_in_s}s: {error}"))
            }
            Self::JobFailed { label, error, .. } => ("error", format!("Failed: {label}: {error}")),
            Self::KeyStatusChanged { status } => (
                if matches!(status, KeyStatus::Ok | KeyStatus::Unknown) { "info" } else { "error" },
                format!("Riot API key status: {}", status.label()),
            ),
            Self::KeyGenerationChanged { .. } => ("warn", "Riot API key changed; re-resolving accounts".into()),
            Self::ReviewReady { match_id, .. } => ("info", format!("AI review ready for {match_id}")),
            Self::SessionReviewReady { .. } => ("info", "AI session review ready".into()),
            Self::LiveGameLinked { match_id, .. } => ("info", format!("Found your last game: {match_id}")),
            Self::PostGameReady { match_id, .. } => ("info", format!("Your last game is analyzed ({match_id})")),
            Self::InGameChanged { in_game: true } => ("info", "Game detected: background work paused".into()),
            Self::InGameChanged { in_game: false } => ("info", "Background work resumed".into()),
            Self::WeeklyReportReady { week, .. } => ("info", format!("AI weekly report ready (week of {week})")),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub key_status: KeyStatus,
    pub paused: bool,
    /// Work is held because a game is running.
    pub in_game: bool,
    pub jobs: JobCounts,
    pub rate_limits: Vec<BucketSnapshot>,
    pub recent_failures: Vec<FailedJob>,
}

/// Why [`Engine::wait_idle`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleOutcome {
    Drained,
    KeyProblem(KeyStatus),
}

pub struct Engine {
    pub(crate) db: Db,
    pub(crate) riot: Arc<RiotClient>,
    pub(crate) ddragon: Arc<DdragonCache>,
    config: EngineConfig,
    events: broadcast::Sender<EngineEvent>,
    wake: Notify,
    key_status: Mutex<KeyStatus>,
    paused: AtomicBool,
    in_game: AtomicBool,
    items: Mutex<HashMap<String, Arc<ItemCatalog>>>,
}

impl Engine {
    pub fn new(db: Db, riot: Arc<RiotClient>, ddragon: Arc<DdragonCache>, config: EngineConfig) -> Arc<Self> {
        let key_status = if riot.has_key() { KeyStatus::Unknown } else { KeyStatus::Missing };
        Arc::new(Self {
            db,
            riot,
            ddragon,
            config,
            events: broadcast::channel(1024).0,
            wake: Notify::new(),
            key_status: Mutex::new(key_status),
            paused: AtomicBool::new(false),
            in_game: AtomicBool::new(false),
            items: Mutex::new(HashMap::new()),
        })
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn ddragon(&self) -> &Arc<DdragonCache> {
        &self.ddragon
    }

    pub fn riot(&self) -> &Arc<RiotClient> {
        &self.riot
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    pub(crate) fn emit(&self, event: EngineEvent) {
        // No receivers is fine.
        let _ = self.events.send(event);
    }

    /// Adds a line to the persisted activity feed.
    pub async fn log_activity(&self, level: &'static str, message: impl Into<String>) {
        let message = message.into();
        if let Err(e) = self.db.call(move |c| rc_store::activity::add(c, level, &message)).await {
            tracing::warn!(error = %e, "could not record activity");
        }
    }

    /// Item prices/names for a game patch, from the Data Dragon cache.
    pub async fn item_catalog(&self, patch: &str) -> Result<Arc<ItemCatalog>, DdragonError> {
        let version = self.ddragon.version_for_patch(patch).await?;
        if let Some(c) = self.items.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&version) {
            return Ok(c.clone());
        }
        let raw = self.ddragon.data_file(&version, "item.json").await?;
        let catalog = Arc::new(ItemCatalog::from_ddragon_json(&raw)?);
        self.items.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(version, catalog.clone());
        Ok(catalog)
    }

    /// Re-queues jobs interrupted by a crash, queues metrics for matches that
    /// need them, starts the activity recorder, then starts the workers.
    pub async fn start(self: &Arc<Self>) -> Result<Vec<JoinHandle<()>>, StoreError> {
        let recovered = self.db.call(|c| q::recover_running(c, now_ms())).await?;
        if recovered > 0 {
            tracing::info!(recovered, "re-queued jobs interrupted by the last shutdown");
        }
        let pruned = self.prune_live_snapshots().await?;
        if pruned > 0 {
            tracing::info!(games = pruned, "pruned live snapshots");
        }
        let stale = self.db.call(|c| rc_store::metrics::stale_matches(c, rc_metrics::METRICS_VERSION)).await?;
        if !stale.is_empty() {
            tracing::info!(matches = stale.len(), "queueing metrics computation");
            self.enqueue_many(stale.into_iter().map(|match_id| Job::ComputeMetrics { match_id }).collect()).await?;
        }
        let recorder = self.clone();
        let mut events = self.subscribe();
        tokio::spawn(async move {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match events.recv().await {
                    Ok(ev) => {
                        if let Some((level, msg)) = ev.describe() {
                            recorder.log_activity(level, msg).await;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });
        // Session reviews wait for a session to end, which no event signals:
        // check now and then.
        let checker = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SESSION_CHECK_EVERY);
            loop {
                tick.tick().await;
                if checker.in_game() {
                    continue;
                }
                if let Err(e) = checker.maybe_queue_session_review().await {
                    tracing::warn!(error = ?e, "session review check failed");
                }
                if let Err(e) = checker.maybe_queue_weekly_report().await {
                    tracing::warn!(error = ?e, "weekly report check failed");
                }
                if let Err(e) = checker.refresh_stale_ranks().await {
                    tracing::warn!(error = ?e, "rank refresh check failed");
                }
            }
        });
        Ok((0..self.config.workers.max(1))
            .map(|i| {
                let engine = self.clone();
                tokio::spawn(async move { engine.worker_loop(i).await })
            })
            .collect())
    }

    pub async fn enqueue(&self, job: Job) -> Result<bool, StoreError> {
        let added = self.enqueue_many(vec![job]).await? > 0;
        Ok(added)
    }

    pub async fn enqueue_many(&self, jobs: Vec<Job>) -> Result<usize, StoreError> {
        self.enqueue_at(jobs, now_ms()).await
    }

    /// Queues jobs that become due at `run_at_ms`.
    pub async fn enqueue_at(&self, jobs: Vec<Job>, run_at_ms: i64) -> Result<usize, StoreError> {
        let rows = jobs
            .iter()
            .map(|j| {
                Ok(NewJob {
                    kind: j.kind().to_owned(),
                    payload: serde_json::to_string(j)?,
                    priority: j.priority(),
                    dedupe_key: j.dedupe_key(),
                    run_at_ms,
                    max_attempts: j.max_attempts(),
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let added = self
            .db
            .call(move |c| {
                let tx = c.transaction()?;
                let now = now_ms();
                let mut added = 0;
                for row in &rows {
                    added += usize::from(q::enqueue(&tx, row, now)?);
                }
                tx.commit()?;
                Ok(added)
            })
            .await?;
        if added > 0 {
            self.wake.notify_waiters();
        }
        Ok(added)
    }

    /// Starts tracking a Riot ID (if new) and queues ingestion of its newest
    /// `count` matches. Returns the account id.
    pub async fn track_and_ingest(&self, riot_id: &RiotId, platform: Platform, count: u32) -> Result<i64, StoreError> {
        let (name, tag) = (riot_id.game_name.clone(), riot_id.tag_line.clone());
        let account_id = self
            .db
            .call(move |c| {
                rc_store::accounts::upsert_account(
                    c,
                    &name,
                    &tag,
                    platform.as_str(),
                    platform.match_region().as_str(),
                    false,
                )
            })
            .await?;
        self.enqueue(Job::ResolveAccount { account_id, then_sync: Some(count) }).await?;
        Ok(account_id)
    }

    pub fn key_status(&self) -> KeyStatus {
        *self.key_status.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn set_key_status(&self, status: KeyStatus) {
        let changed = {
            let mut cur = self.key_status.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let changed = *cur != status;
            *cur = status;
            changed
        };
        if changed {
            tracing::info!(?status, "API key status changed");
            self.emit(EngineEvent::KeyStatusChanged { status });
            if !status.blocks_work() {
                self.wake.notify_waiters();
            }
        }
    }

    /// Swap the API key (e.g. after the user fixes it) and resume work.
    pub fn set_key(&self, key: Option<Secret>) {
        let status = if key.is_some() { KeyStatus::Unknown } else { KeyStatus::Missing };
        self.riot.set_key(key);
        self.set_key_status(status);
    }

    /// Pause/resume all background work (used by game-aware scheduling).
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        if !paused {
            self.wake.notify_waiters();
        }
    }

    /// Hold all queued work while a game runs (game-aware scheduling). A
    /// job already running finishes; nothing new starts until released.
    pub fn set_in_game(&self, in_game: bool) {
        if self.in_game.swap(in_game, Ordering::SeqCst) != in_game {
            tracing::info!(in_game, "game-aware scheduling");
            self.emit(EngineEvent::InGameChanged { in_game });
            if !in_game {
                self.wake.notify_waiters();
            }
        }
    }

    pub fn in_game(&self) -> bool {
        self.in_game.load(Ordering::SeqCst)
    }

    fn blocked(&self) -> bool {
        self.paused.load(Ordering::SeqCst) || self.in_game() || self.key_status().blocks_work()
    }

    pub async fn status(&self) -> Result<EngineStatus, StoreError> {
        let (jobs, recent_failures) =
            self.db.call(|c| Ok((q::counts(c, now_ms())?, q::recent_failures(c, 10)?))).await?;
        Ok(EngineStatus {
            key_status: self.key_status(),
            paused: self.paused.load(Ordering::SeqCst),
            in_game: self.in_game(),
            jobs,
            rate_limits: self.riot.limiter().snapshot(),
            recent_failures,
        })
    }

    /// Resolves when no jobs are pending, or work is blocked on the key.
    pub async fn wait_idle(&self) -> Result<IdleOutcome, StoreError> {
        loop {
            let status = self.key_status();
            if status.blocks_work() {
                return Ok(IdleOutcome::KeyProblem(status));
            }
            if self.db.call(|c| q::counts(c, now_ms())).await?.pending() == 0 {
                return Ok(IdleOutcome::Drained);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn worker_loop(self: Arc<Self>, worker: usize) {
        tracing::debug!(worker, "worker started");
        loop {
            // Register for wake-ups *before* checking, so an enqueue between
            // the check and the wait isn't missed.
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            let sleep_for = if self.blocked() {
                Some(self.config.idle_poll)
            } else {
                match self.db.call(|c| q::claim_next(c, now_ms())).await {
                    Ok(Some(job)) => {
                        self.run(job).await;
                        None
                    }
                    Ok(None) => {
                        let next = self.db.call(|c| q::next_due_at(c)).await.ok().flatten();
                        let until_due = next.map(|t| Duration::from_millis((t - now_ms()).max(0) as u64));
                        Some(until_due.map_or(self.config.idle_poll, |d| d.min(self.config.idle_poll)))
                    }
                    Err(e) => {
                        tracing::error!(worker, error = %e, "could not claim a job");
                        Some(Duration::from_secs(5))
                    }
                }
            };
            if let Some(d) = sleep_for {
                tokio::select! {
                    () = &mut notified => {}
                    () = tokio::time::sleep(d) => {}
                }
            }
        }
    }

    fn backoff(&self, attempt: i32) -> Duration {
        let exp = u32::try_from(attempt.saturating_sub(1)).unwrap_or(0).min(16);
        (self.config.base_backoff * 2u32.pow(exp)).min(self.config.max_backoff)
    }

    async fn run(&self, claimed: ClaimedJob) {
        let id = claimed.id;
        let job: Job = match serde_json::from_str(&claimed.payload) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(id, kind = claimed.kind, error = %e, "unreadable job payload");
                let msg = format!("unreadable payload: {e}");
                let _ = self.db.call(move |c| q::fail(c, id, &msg, now_ms())).await;
                return;
            }
        };
        let (kind, label) = (job.kind().to_owned(), job.label());
        tracing::debug!(id, kind, label, attempt = claimed.attempts, "job started");
        self.emit(EngineEvent::JobStarted { kind: kind.clone(), label: label.clone() });

        let result = self.handle(&job).await;
        let now = now_ms();
        let outcome = match result {
            Ok(()) => {
                if job.uses_riot_api() {
                    self.set_key_status(KeyStatus::Ok);
                }
                tracing::info!(kind, label, "job done");
                self.emit(EngineEvent::JobSucceeded { kind, label });
                self.db.call(move |c| q::complete(c, id, now)).await
            }
            Err(JobError::Key(status)) => {
                self.set_key_status(status);
                let msg = format!("waiting for a valid API key ({status:?})");
                self.db.call(move |c| q::requeue_without_attempt(c, id, now, &msg, now)).await
            }
            Err(JobError::Retry(msg)) if claimed.attempts < claimed.max_attempts => {
                let delay = self.backoff(claimed.attempts);
                tracing::warn!(kind, label, error = msg, retry_in_s = delay.as_secs(), "job will retry");
                self.emit(EngineEvent::JobRetrying { kind, label, error: msg.clone(), retry_in_s: delay.as_secs() });
                let run_at = now + i64::try_from(delay.as_millis()).unwrap_or(i64::MAX);
                self.db.call(move |c| q::retry_later(c, id, run_at, &msg, now)).await
            }
            Err(JobError::Retry(msg) | JobError::Permanent(msg)) => {
                tracing::error!(kind, label, error = msg, "job failed");
                self.emit(EngineEvent::JobFailed { kind, label, error: msg.clone() });
                self.db.call(move |c| q::fail(c, id, &msg, now)).await
            }
        };
        if let Err(e) = outcome {
            tracing::error!(id, error = %e, "could not record job outcome");
        }
    }
}
