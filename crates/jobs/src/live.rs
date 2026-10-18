//! The live poller. Watches the Live Client Data API for a running game,
//! holds background work while one runs (game-aware scheduling), keeps the
//! live view's numbers (own numbers only) current, records the game and the
//! app's footprint during it, and queues the post-game pipeline when it ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rc_common::time::now_ms;
use rc_live::footprint::{Footprint, Sampler};
use rc_live::view::{self, BuildProgress, GoalLive, LiveContext, Marks, Pacing};
use rc_live::{EndReason, LiveClient, LiveError, OwnSnapshot, Phase, Tracker, Transition};
use rc_metrics::ItemCatalog;
use rc_store::live::NewLiveGame;
use rc_store::settings::LiveSettings;
use serde::Serialize;
use tokio::sync::watch;

use crate::engine::Engine;
use crate::job::Job;

/// Game time between stored snapshots.
const SNAPSHOT_EVERY_S: f64 = 15.0;
const SETTINGS_EVERY: Duration = Duration::from_secs(15);
const DISABLED_POLL: Duration = Duration::from_secs(5);
/// Practice tool games never reach Match-V5.
const UNPUBLISHED_MODES: [&str; 2] = ["PRACTICETOOL", "TUTORIAL"];
/// A game seen again within this of a recorded start is the same game (the
/// app restarted mid-game or on the end screen).
const SAME_GAME_MS: i64 = 2 * 60 * 1000;
/// Games still open this long after starting were left by a closed app.
const STALE_GAME_MS: i64 = 3 * 3600 * 1000;

#[derive(Debug, Clone)]
pub struct LiveConfig {
    pub url: String,
    /// Caps every wait between polls (tests).
    pub max_poll: Option<Duration>,
    pub footprint_every: Duration,
    /// Wait before the first look for the finished game in Match-V5.
    pub post_game_delay: Duration,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            url: rc_live::client::DEFAULT_URL.into(),
            max_poll: None,
            footprint_every: Duration::from_secs(5),
            post_game_delay: Duration::from_secs(60),
        }
    }
}

/// What the live view shows. Sent to the UI on every change.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveState {
    pub enabled: bool,
    pub phase: Phase,
    pub live_game_id: Option<i64>,
    /// The Riot ID in the game is one of your tracked accounts (post-game
    /// analysis only runs for those).
    pub tracked: bool,
    pub snapshot: Option<OwnSnapshot>,
    pub context: Option<LiveContext>,
    pub pacing: Option<Pacing>,
    pub build: Option<BuildProgress>,
    pub goals: Vec<GoalLive>,
    pub footprint: Option<Footprint>,
    pub updated_ms: i64,
}

/// A recorded live game for the UI, with where its post-game analysis is.
#[derive(Debug, Clone, Serialize)]
pub struct LiveGameView {
    #[serde(flatten)]
    pub game: rc_store::live::LiveGame,
    /// `in_progress`, `not_tracked`, `not_published` (practice tool),
    /// `waiting` (for Riot), `analyzing`, `analyzed`, `reviewed`.
    pub status: &'static str,
    pub last: Option<OwnSnapshot>,
    pub footprint: Option<Footprint>,
    pub budget_cpu_avg_pct: f64,
    pub budget_mem_mb: f64,
    pub within_budget: Option<bool>,
}

pub fn live_game_view(game: rc_store::live::LiveGame) -> LiveGameView {
    let status = if game.ended_at.is_none() {
        "in_progress"
    } else if game.report_id.is_some() {
        "reviewed"
    } else if game.analyzed_at.is_some() {
        "analyzed"
    } else if game.match_id.is_some() {
        "analyzing"
    } else if game.account_id.is_none() {
        "not_tracked"
    } else if UNPUBLISHED_MODES.contains(&game.game_mode.as_str()) {
        "not_published"
    } else {
        "waiting"
    };
    let footprint: Option<Footprint> = game.footprint_json.as_deref().and_then(|j| serde_json::from_str(j).ok());
    LiveGameView {
        status,
        last: game.final_json.as_deref().and_then(|j| serde_json::from_str(j).ok()),
        within_budget: footprint.as_ref().map(Footprint::within_budget),
        footprint,
        budget_cpu_avg_pct: rc_live::footprint::BUDGET_CPU_AVG_PCT,
        budget_mem_mb: rc_live::footprint::BUDGET_MEM_MB,
        game,
    }
}

struct Run {
    live_game_id: i64,
    account_id: Option<i64>,
    context: LiveContext,
    catalog: Option<Arc<ItemCatalog>>,
    marks: Marks,
    footprint: Footprint,
    sampler: Option<Sampler>,
    last_sample: Option<Instant>,
    last_saved_s: Option<f64>,
}

struct Poller {
    engine: Arc<Engine>,
    client: LiveClient,
    config: LiveConfig,
    tracker: Tracker,
    run: Option<Run>,
    settings: LiveSettings,
    settings_at: Option<Instant>,
    tx: watch::Sender<LiveState>,
}

/// Starts the poller. The receiver always holds the newest [`LiveState`].
pub fn spawn_live(engine: Arc<Engine>, config: LiveConfig) -> Result<watch::Receiver<LiveState>, LiveError> {
    let client = LiveClient::new(&config.url)?;
    let (tx, rx) = watch::channel(LiveState::default());
    let poller = Poller {
        engine,
        client,
        config,
        tracker: Tracker::default(),
        run: None,
        settings: LiveSettings::default(),
        settings_at: None,
        tx,
    };
    tokio::spawn(poller.run_loop());
    Ok(rx)
}

impl Poller {
    async fn refresh_settings(&mut self) {
        if self.settings_at.is_some_and(|t| t.elapsed() < SETTINGS_EVERY) {
            return;
        }
        match self.engine.db.call(|c| rc_store::settings::app(c)).await {
            Ok(s) => self.settings = s.live,
            Err(e) => tracing::warn!(error = %e, "live poller could not read settings"),
        }
        self.settings_at = Some(Instant::now());
    }

    async fn run_loop(mut self) {
        tracing::info!(url = %self.client.base(), "live poller started");
        let now = now_ms();
        match self.engine.db.call(move |c| rc_store::live::close_stale(c, now - STALE_GAME_MS, now)).await {
            Ok(n) if n > 0 => tracing::info!(games = n, "closed live games left open by an earlier run"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "could not close stale live games"),
        }
        loop {
            self.refresh_settings().await;
            if !self.settings.enabled {
                self.engine.set_in_game(false);
                if self.tx.borrow().enabled {
                    self.tx.send_replace(LiveState { enabled: false, updated_ms: now_ms(), ..Default::default() });
                }
                tokio::time::sleep(self.cap(DISABLED_POLL)).await;
                continue;
            }
            let obs = self.client.poll().await;
            let transitions = self.tracker.observe(now_ms(), obs);
            let changed = !transitions.is_empty();
            for t in transitions {
                if let Err(e) = self.handle(t).await {
                    tracing::warn!(error = %e, "live poller could not record the game");
                }
            }
            if self.tracker.phase() == Phase::InGame {
                self.tick().await;
            }
            if changed || self.tracker.phase() == Phase::InGame || !self.tx.borrow().enabled {
                self.publish();
            }
            tokio::time::sleep(self.cap(self.tracker.next_poll())).await;
        }
    }

    fn cap(&self, d: Duration) -> Duration {
        self.config.max_poll.map_or(d, |m| d.min(m))
    }

    async fn handle(&mut self, t: Transition) -> Result<(), rc_store::StoreError> {
        match t {
            Transition::Phase(p) => {
                tracing::info!(phase = ?p, "live phase");
                self.engine.set_in_game(p.is_playing() && self.settings.pause_background);
            }
            Transition::Started { started_at_ms, snapshot } => self.start(started_at_ms, &snapshot).await?,
            Transition::Ended { reason, last } => self.end(reason, &last).await?,
        }
        Ok(())
    }

    async fn start(&mut self, started_at_ms: i64, s: &OwnSnapshot) -> Result<(), rc_store::StoreError> {
        let riot_id = s.riot_id.clone();
        let (account_id, patch) = self
            .engine
            .db
            .call(move |c| {
                let account = rc_store::accounts::list_accounts(c)?
                    .into_iter()
                    .find(|a| format!("{}#{}", a.game_name, a.tag_line).eq_ignore_ascii_case(&riot_id));
                Ok((account.map(|a| a.id), rc_store::matches::latest_patch(c)?))
            })
            .await?;
        let row = s.clone();
        let (live_game_id, resumed) = self
            .engine
            .db
            .call(move |c| {
                if let Some(g) = rc_store::live::find_near(c, started_at_ms, SAME_GAME_MS)? {
                    return Ok((g.id, true));
                }
                let id = rc_store::live::insert(
                    c,
                    &NewLiveGame {
                        account_id,
                        champion_name: &row.champion_name,
                        champion_ddragon: row.champion_ddragon_id.as_deref(),
                        position: Some(row.position.as_str()).filter(|p| !p.is_empty()),
                        game_mode: &row.game_mode,
                        started_at: started_at_ms,
                    },
                )?;
                Ok((id, false))
            })
            .await?;
        tracing::info!(live_game_id, resumed, tracked = account_id.is_some(), mode = s.game_mode, "game started");

        // Static context, built once: item data from the local cache for the
        // newest stored patch (no downloads mid-game when it's cached).
        let catalog = match &patch {
            Some(p) => self.engine.item_catalog(p).await.ok(),
            None => None,
        };
        let champion_id = match (&patch, &s.champion_ddragon_id) {
            (Some(p), Some(dd)) => self.champion_key(p, dd).await,
            _ => None,
        };
        let input = (account_id, champion_id, s.champion_ddragon_id.clone(), s.position.clone(), patch.clone());
        let cat = catalog.clone();
        let context = self
            .engine
            .db
            .call(move |c| {
                let settings = rc_store::settings::app(c)?;
                let ctx = rc_coach::live::context(
                    c,
                    rc_coach::live::ContextInput {
                        account_id: input.0,
                        champion_id: input.1,
                        champion: input.2.as_deref(),
                        role: Some(input.3.as_str()),
                        patch: input.4.as_deref(),
                    },
                    &settings,
                    cat.as_deref(),
                );
                Ok(ctx.unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "live context incomplete");
                    LiveContext::default()
                }))
            })
            .await?;
        self.run = Some(Run {
            live_game_id,
            account_id,
            context,
            catalog,
            marks: Marks::default(),
            footprint: Footprint::default(),
            sampler: Sampler::new(),
            last_sample: None,
            last_saved_s: None,
        });
        Ok(())
    }

    async fn champion_key(&self, patch: &str, ddragon_id: &str) -> Option<i32> {
        let dd = self.engine.ddragon();
        let version = dd.version_for_patch(patch).await.ok()?;
        let idx = dd.champions(&version).await.ok()?;
        idx.iter().find(|(_, c)| c.id.eq_ignore_ascii_case(ddragon_id)).map(|(k, _)| k)
    }

    /// Per poll in game: derived numbers, snapshots, footprint.
    async fn tick(&mut self) {
        let Some(s) = self.tracker.last().cloned() else { return };
        let Some(run) = &mut self.run else { return };
        run.marks.update(&s);
        if run.last_sample.is_none_or(|t| t.elapsed() >= self.config.footprint_every) {
            run.last_sample = Some(Instant::now());
            if let Some(sample) = run.sampler.as_mut().and_then(Sampler::sample) {
                run.footprint.add(sample);
            }
        }
        if run.last_saved_s.is_none_or(|t| s.game_time_s - t >= SNAPSHOT_EVERY_S) {
            run.last_saved_s = Some(s.game_time_s);
            let (id, t_ms) = (run.live_game_id, (s.game_time_s * 1000.0).round() as i64);
            if let Ok(json) = serde_json::to_string(&s)
                && let Err(e) = self.engine.db.call(move |c| rc_store::live::add_snapshot(c, id, t_ms, &json)).await
            {
                tracing::warn!(error = %e, "could not store live snapshot");
            }
        }
    }

    async fn end(&mut self, reason: EndReason, last: &OwnSnapshot) -> Result<(), rc_store::StoreError> {
        let Some(run) = &self.run else { return Ok(()) };
        let id = run.live_game_id;
        let final_json = serde_json::to_string(last)?;
        let footprint = (run.footprint.samples > 0).then(|| serde_json::to_string(&run.footprint)).transpose()?;
        let (result, duration) = (last.game_end.clone().filter(|r| !r.is_empty()), last.game_time_s);
        let first_end = self
            .engine
            .db
            .call(move |c| {
                let first =
                    rc_store::live::end(c, id, now_ms(), reason.as_str(), result.as_deref(), duration, &final_json)?;
                if let Some(f) = footprint.filter(|_| first) {
                    rc_store::live::set_footprint(c, id, &f)?;
                }
                Ok(first)
            })
            .await?;
        if !first_end {
            // Seen again after a restart on the end screen: already handled.
            return Ok(());
        }
        tracing::info!(live_game_id = id, ?reason, footprint = ?run.footprint, "game ended");
        let published = !UNPUBLISHED_MODES.contains(&last.game_mode.as_str());
        if let (Some(account_id), true) = (run.account_id, published) {
            let delay = i64::try_from(self.config.post_game_delay.as_millis()).unwrap_or(i64::MAX);
            self.engine.enqueue_at(vec![Job::PostGame { account_id, live_game_id: id }], now_ms() + delay).await?;
        }
        Ok(())
    }

    fn publish(&self) {
        let phase = self.tracker.phase();
        let snapshot = self.tracker.last().cloned().filter(|_| phase != Phase::Idle);
        let mut state = LiveState { enabled: true, phase, updated_ms: now_ms(), ..Default::default() };
        if let (Some(run), Some(s)) = (&self.run, &snapshot) {
            state.live_game_id = Some(run.live_game_id);
            state.tracked = run.account_id.is_some();
            state.pacing = Some(view::pacing(s, &run.context));
            state.build = run
                .catalog
                .as_deref()
                .and_then(|cat| view::build_progress(&run.context.build, cat, &s.items, s.current_gold));
            state.goals = view::goals_live(&run.context.goals, s, &run.marks);
            state.footprint = (run.footprint.samples > 0).then(|| run.footprint.clone());
            state.context = Some(run.context.clone());
        }
        state.snapshot = snapshot;
        self.tx.send_replace(state);
    }
}
