//! The live poller. Watches the Live Client Data API for a running game,
//! holds background work while one runs (game-aware scheduling), keeps the
//! live view's numbers (own numbers only) current, records the game and the
//! app's footprint during it, and queues the post-game pipeline when it ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rc_coach::pregame::Brief;
use rc_common::time::now_ms;
use rc_live::footprint::{Footprint, Sampler};
use rc_live::view::{self, BuildProgress, GoalLive, LiveContext, Marks, Pacing};
use rc_live::{EndReason, LiveClient, LiveError, OwnSnapshot, Phase, Tracker, Transition};
use rc_metrics::ItemCatalog;
use rc_riot::Platform;
use rc_riot::dto::ActiveGameDto;
use rc_store::live::NewLiveGame;
use rc_store::settings::{LiveSettings, VisionSettings};
use rc_vision::{CaptureConfig, CaptureStats, GameClock, Session};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::engine::Engine;
use crate::job::Job;

/// Game time between stored snapshots.
const SNAPSHOT_EVERY_S: f64 = 15.0;
const SETTINGS_EVERY: Duration = Duration::from_secs(15);
const DISABLED_POLL: Duration = Duration::from_secs(5);
/// How often to look for the game window when vision capture isn't running.
const VISION_RETRY: Duration = Duration::from_secs(10);
/// Vision review covers Summoner's Rift games only.
const VISION_MODE: &str = "CLASSIC";
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
    /// The pre-game brief (built at the loading screen; static for the game).
    pub brief: Option<BriefState>,
    /// Vision capture for this game, when on (the app's own status).
    pub vision: Option<VisionLive>,
    pub updated_ms: i64,
}

/// Status of the screenshot capture (nothing about the game itself).
#[derive(Debug, Clone, Serialize)]
pub struct VisionLive {
    pub running: bool,
    pub frames: u32,
    pub interval_s: f64,
    pub avg_ms: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Automatic game plans are off (one can be requested on the loading screen).
    Off,
    Generating,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct BriefState {
    /// `None` while loading or when unavailable (see `note`).
    pub brief: Option<Brief>,
    pub loading: bool,
    pub note: Option<String>,
    pub account_id: Option<i64>,
    pub plan_status: PlanStatus,
    /// The AI game plan (validated JSON), when ready.
    pub plan: Option<Value>,
    pub plan_report_id: Option<i64>,
    pub plan_error: Option<String>,
}

/// Requests from the UI.
#[derive(Debug, Clone, Copy)]
pub enum LiveCommand {
    /// Write the AI game plan now (only honored on the loading screen).
    GeneratePlan,
}

/// The poller's outputs and inputs.
#[derive(Debug, Clone)]
pub struct LiveHandle {
    pub state: watch::Receiver<LiveState>,
    pub commands: mpsc::Sender<LiveCommand>,
}

enum BriefMsg {
    Brief(Result<(i64, Box<Brief>), String>),
    Plan(Result<(i64, Value), String>),
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
    /// Vision capture of this game, when frames were taken.
    pub vision: Option<CaptureStats>,
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
        vision: game.vision_json.as_deref().and_then(|j| serde_json::from_str(j).ok()),
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
    clock: Arc<GameClock>,
    /// Vision capture; the mutex is only held briefly (or by a blocking task
    /// starting/stopping capture, which the poller never waits on).
    vision: Option<Arc<std::sync::Mutex<Option<Session>>>>,
    vision_tried: Option<Instant>,
}

struct Poller {
    engine: Arc<Engine>,
    client: LiveClient,
    config: LiveConfig,
    tracker: Tracker,
    run: Option<Run>,
    settings: LiveSettings,
    vision: VisionSettings,
    settings_at: Option<Instant>,
    tx: watch::Sender<LiveState>,
    brief: Option<BriefState>,
    brief_tx: mpsc::UnboundedSender<BriefMsg>,
    brief_rx: mpsc::UnboundedReceiver<BriefMsg>,
    commands: mpsc::Receiver<LiveCommand>,
}

/// Starts the poller. `state` always holds the newest [`LiveState`].
pub fn spawn_live(engine: Arc<Engine>, config: LiveConfig) -> Result<LiveHandle, LiveError> {
    let client = LiveClient::new(&config.url)?;
    let (tx, rx) = watch::channel(LiveState::default());
    let (brief_tx, brief_rx) = mpsc::unbounded_channel();
    let (cmd_tx, commands) = mpsc::channel(8);
    let poller = Poller {
        brief: None,
        brief_tx,
        brief_rx,
        commands,
        engine,
        client,
        config,
        tracker: Tracker::default(),
        run: None,
        settings: LiveSettings::default(),
        vision: VisionSettings::default(),
        settings_at: None,
        tx,
    };
    tokio::spawn(poller.run_loop());
    Ok(LiveHandle { state: rx, commands: cmd_tx })
}

/// Finds the player's current game (Spectator-V5) among tracked accounts and
/// builds the brief. Only champions and teams are read from it.
async fn build_brief(engine: Arc<Engine>) -> Result<(i64, Box<Brief>), String> {
    let candidates = engine
        .db
        .call(|c| {
            let settings = rc_store::settings::app(c)?;
            let mut out = Vec::new();
            for a in rc_store::accounts::list_accounts(c)? {
                if let Some(p) = rc_store::accounts::current_puuid(c, a.id)? {
                    let first = settings.active_account_id == Some(a.id)
                        || (settings.active_account_id.is_none() && a.is_primary);
                    out.push((first, a.id, a.platform, p));
                }
            }
            out.sort_by_key(|x| !x.0);
            Ok(out)
        })
        .await
        .map_err(|e| e.to_string())?;
    for (_, account_id, platform, puuid) in candidates {
        let Ok(platform) = platform.parse::<Platform>() else { continue };
        let raw = match engine.riot.active_game_raw(platform, &puuid).await {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => return Err(format!("Spectator: {e}")),
        };
        let game: ActiveGameDto = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        let Some(me) = game.participants.iter().find(|p| p.puuid.as_deref() == Some(puuid.as_str())) else {
            continue;
        };
        let input = rc_coach::pregame::BriefInput {
            account_id,
            game_id: game.game_id,
            champion_id: me.champion_id,
            enemies: game.participants.iter().filter(|p| p.team_id != me.team_id).map(|p| p.champion_id).collect(),
        };
        let brief = engine
            .db
            .call(move |c| Ok(rc_coach::pregame::brief(c, &input)))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        return Ok((account_id, Box::new(brief)));
    }
    Err("None of your tracked Riot IDs is in a game Riot can see yet".into())
}

async fn build_plan(engine: Arc<Engine>, account_id: i64, brief: Brief) -> Result<(i64, Value), String> {
    let settings = engine.db.call(|c| rc_store::settings::app(c)).await.map_err(|e| e.to_string())?;
    let client = rc_coach::provider::client_from_settings(&settings.ai).map_err(|e| e.to_string())?;
    let opts = rc_coach::review::ReviewOptions::from_settings(&settings, false);
    let report = rc_coach::review::pregame_plan(&engine.db, &client, account_id, &brief, &opts)
        .await
        .map_err(|e| e.to_string())?;
    Ok((report.id, report.output))
}

impl Poller {
    async fn refresh_settings(&mut self) {
        if self.settings_at.is_some_and(|t| t.elapsed() < SETTINGS_EVERY) {
            return;
        }
        match self.engine.db.call(|c| rc_store::settings::app(c)).await {
            Ok(s) => {
                self.settings = s.live;
                self.vision = s.vision;
            }
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
            self.drain_messages().await;
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
                match p {
                    Phase::Loading => self.start_brief(true).await,
                    Phase::Idle => self.brief = None,
                    Phase::InGame | Phase::Ended => {}
                }
            }
            Transition::Started { started_at_ms, snapshot } => {
                self.start(started_at_ms, &snapshot).await?;
                // App opened mid-game: the brief, but no AI plan once the game runs.
                if self.brief.is_none() {
                    self.start_brief(false).await;
                }
                self.save_brief().await;
            }
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
        let clock = Arc::new(GameClock::default());
        let vision = self.new_vision(live_game_id, account_id, &s.game_mode, &clock);
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
            clock,
            vision,
            vision_tried: None,
        });
        Ok(())
    }

    /// A capture session for a tracked Summoner's Rift game, when vision
    /// review is on. Capture itself starts in [`Self::vision_upkeep`].
    fn new_vision(
        &self,
        live_game_id: i64,
        account_id: Option<i64>,
        mode: &str,
        clock: &Arc<GameClock>,
    ) -> Option<Arc<std::sync::Mutex<Option<Session>>>> {
        let store = self.engine.frame_store()?;
        if !self.vision.enabled || account_id.is_none() || mode != VISION_MODE {
            return None;
        }
        let session = Session::new(CaptureConfig {
            live_game_id,
            interval: Duration::from_secs(u64::from(self.vision.interval_s.clamp(3, 10))),
            max_width: self.vision.max_width.clamp(640, 1920),
            store: store.clone(),
            clock: clock.clone(),
            window: std::env::var(rc_vision::WINDOW_ENV).ok(),
        });
        tracing::info!(live_game_id, interval_s = self.vision.interval_s, "vision capture armed");
        Some(Arc::new(std::sync::Mutex::new(Some(session))))
    }

    /// Starts capture when the game window appears (and again if it is
    /// re-created); stops it if vision review is turned off mid-game.
    fn vision_upkeep(&mut self) {
        let enabled = self.vision.enabled;
        let Some(run) = &mut self.run else { return };
        let Some(v) = run.vision.clone() else { return };
        if !enabled {
            run.vision = None;
            tokio::task::spawn_blocking(move || {
                if let Some(s) = v.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                    let id = s.live_game_id();
                    s.stop();
                    tracing::info!(live_game_id = id, "vision capture turned off");
                }
            });
            return;
        }
        let running = match v.try_lock() {
            Ok(g) => g.as_ref().is_none_or(Session::running),
            Err(_) => true, // starting or stopping right now
        };
        if running || run.vision_tried.is_some_and(|t| t.elapsed() < VISION_RETRY) {
            return;
        }
        run.vision_tried = Some(Instant::now());
        tokio::task::spawn_blocking(move || {
            let mut g = v.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(s) = g.as_mut() else { return };
            match s.ensure_running() {
                Ok(true) => tracing::info!(live_game_id = s.live_game_id(), "vision capture started"),
                Ok(false) => {}
                Err(e) => tracing::debug!(error = %e, "vision capture not started yet"),
            }
        });
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
        if let Some(run) = &self.run {
            run.clock.set(s.game_time_s);
        }
        self.vision_upkeep();
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
        let vision = self.run.as_mut().and_then(|r| Some((r.live_game_id, r.vision.take()?)));
        if let Some((id, v)) = vision {
            self.finish_vision(id, v).await;
        }
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

    /// Stops capture and records its stats; a game without frames keeps none.
    async fn finish_vision(&self, live_game_id: i64, v: Arc<std::sync::Mutex<Option<Session>>>) {
        let stats: Option<CaptureStats> = tokio::task::spawn_blocking(move || {
            v.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take().map(Session::stop)
        })
        .await
        .ok()
        .flatten();
        let Some(stats) = stats else { return };
        tracing::info!(live_game_id, ?stats, "vision capture finished");
        if stats.frames == 0 {
            if let Some(store) = self.engine.frame_store() {
                let _ = store.delete(live_game_id);
            }
            return;
        }
        let Ok(json) = serde_json::to_string(&stats) else { return };
        if let Err(e) = self.engine.db.call(move |c| rc_store::live::set_vision_captured(c, live_game_id, &json)).await
        {
            tracing::warn!(error = %e, "could not record vision capture");
        }
    }

    async fn start_brief(&mut self, allow_plan: bool) {
        if self.brief.is_some() {
            return;
        }
        let auto =
            allow_plan && self.engine.db.call(|c| rc_store::settings::app(c)).await.is_ok_and(|s| s.ai.auto_pregame);
        self.brief = Some(BriefState {
            brief: None,
            loading: true,
            note: None,
            account_id: None,
            plan_status: if auto { PlanStatus::Generating } else { PlanStatus::Off },
            plan: None,
            plan_report_id: None,
            plan_error: None,
        });
        let (engine, tx) = (self.engine.clone(), self.brief_tx.clone());
        tokio::spawn(async move {
            let _ = tx.send(BriefMsg::Brief(build_brief(engine).await));
        });
        self.publish();
    }

    fn start_plan(&mut self) {
        let Some(b) = &mut self.brief else { return };
        let (Some(brief), Some(account_id)) = (b.brief.clone(), b.account_id) else { return };
        b.plan_status = PlanStatus::Generating;
        b.plan_error = None;
        let (engine, tx) = (self.engine.clone(), self.brief_tx.clone());
        tokio::spawn(async move {
            let _ = tx.send(BriefMsg::Plan(build_plan(engine, account_id, brief).await));
        });
    }

    /// Applies finished brief and plan work, and UI requests.
    async fn drain_messages(&mut self) {
        let mut changed = false;
        while let Ok(cmd) = self.commands.try_recv() {
            match cmd {
                LiveCommand::GeneratePlan => {
                    let can = self.tracker.phase() == Phase::Loading
                        && self.brief.as_ref().is_some_and(|b| {
                            b.brief.is_some() && matches!(b.plan_status, PlanStatus::Off | PlanStatus::Failed)
                        });
                    if can {
                        self.start_plan();
                        changed = true;
                    }
                }
            }
        }
        while let Ok(msg) = self.brief_rx.try_recv() {
            let Some(b) = &mut self.brief else { continue };
            changed = true;
            match msg {
                BriefMsg::Brief(Ok((account_id, brief))) => {
                    tracing::info!(game_id = brief.game_id, "pre-game brief ready");
                    b.loading = false;
                    b.account_id = Some(account_id);
                    b.brief = Some(*brief);
                    let plan = b.plan_status == PlanStatus::Generating;
                    if plan {
                        self.start_plan();
                    }
                    self.engine.emit(crate::engine::EngineEvent::PregameReady {
                        live_game_id: self.run.as_ref().map(|r| r.live_game_id),
                    });
                    self.save_brief().await;
                }
                BriefMsg::Brief(Err(e)) => {
                    tracing::info!(reason = %e, "no pre-game brief");
                    b.loading = false;
                    b.note = Some(e);
                    if b.plan_status == PlanStatus::Generating {
                        b.plan_status = PlanStatus::Off;
                    }
                }
                BriefMsg::Plan(Ok((report_id, plan))) => {
                    b.plan_status = PlanStatus::Ready;
                    b.plan = Some(plan);
                    b.plan_report_id = Some(report_id);
                }
                BriefMsg::Plan(Err(e)) => {
                    tracing::warn!(error = %e, "AI game plan failed");
                    b.plan_status = PlanStatus::Failed;
                    b.plan_error = Some(e);
                }
            }
        }
        if changed {
            self.publish();
        }
    }

    /// Stores the brief with the recorded game, once both exist.
    async fn save_brief(&self) {
        let (Some(run), Some(Some(brief))) = (&self.run, self.brief.as_ref().map(|b| b.brief.as_ref())) else {
            return;
        };
        let Ok(json) = serde_json::to_string(brief) else { return };
        let id = run.live_game_id;
        if let Err(e) = self.engine.db.call(move |c| rc_store::live::set_brief(c, id, &json)).await {
            tracing::warn!(error = %e, "could not store the pre-game brief");
        }
    }

    fn publish(&self) {
        let phase = self.tracker.phase();
        let snapshot = self.tracker.last().cloned().filter(|_| phase != Phase::Idle);
        let mut state =
            LiveState { enabled: true, phase, brief: self.brief.clone(), updated_ms: now_ms(), ..Default::default() };
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
            state.vision = run.vision.as_ref().and_then(|v| {
                let g = v.try_lock().ok()?;
                let s = g.as_ref()?;
                let st = s.stats();
                Some(VisionLive {
                    running: s.running(),
                    frames: st.frames,
                    interval_s: st.interval_s,
                    avg_ms: st.avg_ms,
                    error: st.error,
                })
            });
        }
        state.snapshot = snapshot;
        self.tx.send_replace(state);
    }
}
