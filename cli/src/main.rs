//! `rc-cli`: headless commands for development. Uses the same database and
//! engine as the desktop app.

mod fixtures;
mod mock_live;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use rc_common::secret::load_riot_api_key;
use rc_common::{AppPaths, KeySource};
use rc_jobs::{Engine, EngineConfig, EngineEvent, IdleOutcome};
use rc_riot::client::MatchIdQuery;
use rc_riot::{Platform, RiotId};

#[derive(Parser)]
#[command(name = "rc-cli", about = "Rift Coach developer CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Track a Riot ID and ingest its newest matches and timelines.
    Ingest {
        /// e.g. "TapRooted#BTC"
        riot_id: RiotId,
        #[arg(long, default_value = "na1")]
        platform: Platform,
        #[arg(long, default_value_t = 100)]
        count: u32,
    },
    /// Show queue, key, and stored-match status.
    Status,
    /// Call each Riot endpoint once to confirm paths and key access.
    Smoke {
        riot_id: RiotId,
        #[arg(long, default_value = "na1")]
        platform: Platform,
    },
    /// Games and win rate per role and champion (standard Summoner's Rift only).
    Summary,
    /// Compute any missing metrics, then print one game's metrics with
    /// percentiles (default: the newest Summoner's Rift game).
    Analyze {
        match_id: Option<String>,
        /// Personal baseline window (games).
        #[arg(long, default_value_t = 30)]
        games: u32,
        /// Print the AI facts object instead of the table.
        #[arg(long)]
        facts: bool,
    },
    /// Compute any missing metrics, then print recurring patterns and which
    /// metrics move your win rate over your newest games.
    Patterns {
        #[arg(long, default_value_t = 30)]
        games: u32,
        /// Only one role (TOP, JUNGLE, MIDDLE, BOTTOM, UTILITY).
        #[arg(long)]
        role: Option<String>,
        /// Also list sessions.
        #[arg(long)]
        sessions: bool,
    },
    /// Benchmark collection progress and your gaps to the benchmark tier;
    /// `--collect N` first runs the collector for N minutes.
    Benchmarks {
        #[arg(long)]
        collect: Option<u64>,
        /// Role for the gaps (default: your main role).
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        champion: Option<String>,
    },
    /// Save raw API responses to a gitignored folder for building fixtures.
    RecordFixtures {
        riot_id: RiotId,
        #[arg(long, default_value = "na1")]
        platform: Platform,
        /// Number of newest matches (with timelines) to record.
        #[arg(long, default_value_t = 3)]
        matches: u32,
        #[arg(long, default_value = "raw-captures")]
        out: PathBuf,
    },
    /// Replace other players' identities in recorded responses, writing
    /// commit-safe fixtures.
    ScrubFixtures {
        #[arg(long, default_value = "raw-captures")]
        input: PathBuf,
        #[arg(long, default_value = "tests/fixtures")]
        output: PathBuf,
        /// The Riot ID to keep (yours).
        #[arg(long)]
        keep: RiotId,
    },
    /// Serve a scripted game on the Live Client Data API path, to try the
    /// live view without League (point the app at it with RIFT_COACH_LIVE_URL).
    MockLive {
        #[arg(long, default_value_t = 2998)]
        port: u16,
        /// Game seconds per real second.
        #[arg(long, default_value_t = 10.0)]
        speed: f64,
        /// Game length in minutes.
        #[arg(long, default_value_t = 28.0)]
        minutes: f64,
        /// Data Dragon champion id (default: your most played).
        #[arg(long)]
        champion: Option<String>,
        /// Default: your primary account.
        #[arg(long)]
        riot_id: Option<String>,
        /// CLASSIC games are looked up in Match-V5 afterwards; PRACTICETOOL ones aren't.
        #[arg(long, default_value = "PRACTICETOOL")]
        mode: String,
        #[arg(long)]
        lose: bool,
        /// Real seconds of loading screen before the game starts.
        #[arg(long, default_value_t = 5.0)]
        loading: f64,
        /// Real seconds the end screen stays up.
        #[arg(long, default_value_t = 20.0)]
        hold: f64,
    },
    /// Capture a window the way vision review captures the game, and report
    /// the cost per frame and this process's CPU (no League needed).
    VisionCapture {
        /// Part of the window title to capture (default: the League game window),
        /// e.g. "Notepad" or a browser playing a VOD.
        #[arg(long)]
        window: Option<String>,
        #[arg(long, default_value_t = 30)]
        seconds: u64,
        #[arg(long, default_value_t = 4.0)]
        interval: f64,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        /// Keep the frames in this folder (default: a temporary folder, deleted).
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Run the vision review of a recorded game now, like the app's
    /// "Review screenshots" (default: the newest game with screenshots).
    VisionReview { match_id: Option<String> },
}

async fn vision_review(engine: &Arc<Engine>, match_id: Option<String>) -> anyhow::Result<()> {
    let store = engine.frame_store().cloned().context("no frames folder")?;
    let (settings, account, live) = engine
        .db()
        .call(move |c| {
            let account = rc_store::accounts::primary_account(c)?;
            let live = match match_id {
                Some(id) => rc_store::live::by_match(c, &id)?,
                None => rc_store::live::recent(c, 20)?.into_iter().find(|g| g.match_id.is_some()),
            };
            Ok((rc_store::settings::app(c)?, account, live))
        })
        .await?;
    let account = account.context("no account tracked yet")?;
    let live = live.context("no recorded live game for that match")?;
    let match_id = live.match_id.clone().context("that game isn't linked to a match yet")?;
    println!("{match_id}: {} screenshots on disk", store.list(live.id).len());
    let client = rc_coach::provider::vision_client(&settings.vision)?;
    println!("Reviewing with {} {} ...", settings.vision.provider, settings.vision.model);
    let opts = rc_coach::vision::VisionOptions::from_settings(&settings);
    let report =
        rc_coach::vision::vision_review(engine.db(), &client, &store, account.id, &match_id, live.id, &opts).await?;
    let (live_id, report_id) = (live.id, report.id);
    engine.db().call(move |c| rc_store::live::set_vision_report(c, live_id, report_id)).await?;
    store.delete(live.id)?;
    println!(
        "Saved report {} ({} tokens in, {} out); screenshots deleted.",
        report.id,
        report.input_tokens.unwrap_or(0),
        report.output_tokens.unwrap_or(0)
    );
    Ok(())
}

async fn vision_capture(
    window: Option<String>,
    seconds: u64,
    interval: f64,
    width: u32,
    out: Option<std::path::PathBuf>,
) -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    let tmp = tempfile::tempdir()?;
    let dir = out.clone().unwrap_or_else(|| tmp.path().to_path_buf());
    let clock = Arc::new(rc_vision::GameClock::default());
    let store = rc_vision::FrameStore::new(&dir);
    let mut session = rc_vision::Session::new(rc_vision::CaptureConfig {
        live_game_id: 1,
        interval: Duration::from_secs_f64(interval),
        max_width: width,
        store: store.clone(),
        clock: clock.clone(),
        window,
    });
    let start = Instant::now();
    clock.set(0.0);
    session.ensure_running().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut sampler = rc_live::footprint::Sampler::new();
    let mut footprint = rc_live::footprint::Footprint::default();
    println!("Capturing for {seconds}s every {interval}s at up to {width}px wide...");
    while start.elapsed() < Duration::from_secs(seconds) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        clock.set(start.elapsed().as_secs_f64());
        if let Some(s) = sampler.as_mut().and_then(rc_live::footprint::Sampler::sample) {
            footprint.add(s);
        }
    }
    let stats = tokio::task::spawn_blocking(move || session.stop()).await?;
    println!("{}", serde_json::to_string_pretty(&stats)?);
    println!(
        "process CPU avg {:.2}% (of all cores), peak {:.2}%, memory {:.0} MB",
        footprint.cpu_avg_pct, footprint.cpu_max_pct, footprint.mem_max_mb
    );
    let usage = store.usage();
    println!("{} frames, {:.1} MB on disk", usage.frames, usage.bytes as f64 / 1_048_576.0);
    if let Some(o) = out {
        println!("frames kept in {}", o.display());
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::resolve().context("resolving app data folder")?;
    let _log = rc_common::logging::init(&paths.logs_dir, "rc-cli").context("starting logging")?;

    match cli.command {
        Command::ScrubFixtures { input, output, keep } => {
            let report = fixtures::scrub_dir(&input, &output, &keep)?;
            println!(
                "Scrubbed {} files -> {} ({} other players anonymized)",
                report.files,
                output.display(),
                report.players_replaced
            );
            Ok(())
        }
        Command::Summary => summary(&paths),
        Command::VisionCapture { window, seconds, interval, width, out } => {
            vision_capture(window, seconds, interval, width, out).await
        }
        Command::MockLive { port, speed, minutes, champion, riot_id, mode, lose, loading, hold } => {
            let (default_id, default_champ) = mock_defaults(&paths)?;
            let riot_id = riot_id.or(default_id).ok_or_else(|| anyhow::anyhow!("no account yet: pass --riot-id"))?;
            let champion = champion.or(default_champ).unwrap_or_else(|| "Ahri".into());
            mock_live::run(mock_live::Options {
                port,
                speed,
                minutes,
                champion,
                riot_id,
                mode,
                lose,
                loading_s: loading,
                hold_s: hold,
            })
            .await
        }
        command => {
            let key = load_key();
            let engine = rc_jobs::build_engine(&paths, key, EngineConfig::default())?;
            match command {
                Command::Ingest { riot_id, platform, count } => ingest(&engine, &riot_id, platform, count).await,
                Command::Status => status(&engine, &paths).await,
                Command::Smoke { riot_id, platform } => smoke(&engine, &riot_id, platform).await,
                Command::Analyze { match_id, games, facts } => analyze(&engine, match_id, games, facts).await,
                Command::Patterns { games, role, sessions } => patterns(&engine, games, role, sessions).await,
                Command::Benchmarks { collect, role, champion } => benchmarks(&engine, collect, role, champion).await,
                Command::RecordFixtures { riot_id, platform, matches, out } => {
                    fixtures::record(engine.riot(), &riot_id, platform, matches, &out).await
                }
                Command::VisionReview { match_id } => vision_review(&engine, match_id).await,
                Command::ScrubFixtures { .. }
                | Command::Summary
                | Command::MockLive { .. }
                | Command::VisionCapture { .. } => {
                    unreachable!("handled above")
                }
            }
        }
    }
}

fn load_key() -> Option<rc_common::Secret> {
    match load_riot_api_key() {
        Some((key, source)) => {
            let from = match source {
                KeySource::ProcessEnv => "process environment",
                KeySource::UserEnv => "user environment (registry)",
                KeySource::CredentialManager => "Windows Credential Manager",
            };
            println!("Riot API key: found in {from}");
            Some(key)
        }
        None => {
            println!("Riot API key: NOT FOUND (set the RIOT_API_KEY user environment variable)");
            None
        }
    }
}

async fn analyze(engine: &Arc<Engine>, match_id: Option<String>, games: u32, facts: bool) -> anyhow::Result<()> {
    engine.start().await?;
    if let IdleOutcome::KeyProblem(k) = engine.wait_idle().await? {
        println!("(work paused: key {k:?}; analyzing what's stored)");
    }
    let detail = engine
        .db()
        .call(move |c| {
            let account = rc_store::accounts::primary_account(c)?
                .ok_or_else(|| rc_store::StoreError::Io(std::io::Error::other("no account tracked yet")))?;
            let id = match match_id {
                Some(id) => id,
                None => rc_store::metrics::recent_game_metrics(c, account.id, 1)?
                    .first()
                    .map(|g| g.match_id.clone())
                    .ok_or_else(|| rc_store::StoreError::Io(std::io::Error::other("no Rift games stored")))?,
            };
            let gap = rc_store::settings::app(c)?.session_gap_min;
            Ok((account.id, rc_coach::detail::match_detail(c, account.id, &id, games, gap)))
        })
        .await?;
    let (account_id, detail) = (detail.0, detail.1?);
    if facts {
        let items = engine.item_catalog(&detail.summary.patch).await.ok();
        let settings = engine.db().call(|c| rc_store::settings::app(c)).await?;
        let opts = rc_coach::review::ReviewOptions {
            baseline_games: games,
            ..rc_coach::review::ReviewOptions::from_settings(&settings, false)
        };
        let f =
            rc_coach::review::post_game_facts(engine.db(), account_id, &detail.summary.match_id, &opts, items).await?;
        let text = serde_json::to_string_pretty(&f)?;
        println!("{text}");
        eprintln!("{} chars (~{} tokens)", text.len(), text.len() / 4);
        return Ok(());
    }
    let s = &detail.summary;
    println!(
        "{} {} {} {}/{}/{} ({})",
        s.match_id,
        s.champion_name,
        s.team_position.as_deref().unwrap_or("-"),
        s.kills,
        s.deaths,
        s.assists,
        if s.win { "win" } else { "loss" }
    );
    let p = &detail.comparison.populations;
    println!("{:<28} {:>9}  {:>22}  {:>22}", "metric", "value", p.role_label, "your games");
    let fmt = |c: &Option<rc_metrics::percentile::Comparison>| {
        c.as_ref().map_or("-".to_string(), |c| format!("{:>3.0}% (med {:.1}, n={})", c.better_than_pct, c.median, c.n))
    };
    for m in &detail.comparison.metrics {
        println!("{:<28} {:>9.2}  {:>22}  {:>22}", m.label, m.value, fmt(&m.vs_role), fmt(&m.vs_personal));
    }
    println!("deaths: {}", detail.deaths.len());
    for d in &detail.deaths {
        println!(
            "  {:>5.0}s enemies={} pre14={} enemy_half={} gold_vs_opp={:?}",
            d.t_s, d.enemies_involved, d.before_14, d.in_enemy_half, d.gold_vs_opponent
        );
    }
    Ok(())
}

async fn benchmarks(
    engine: &Arc<Engine>,
    collect: Option<u64>,
    role: Option<String>,
    champion: Option<String>,
) -> anyhow::Result<()> {
    engine.start().await?;
    if let Some(minutes) = collect {
        let mut rx = rc_jobs::spawn_benchmarks(engine.clone(), rc_jobs::BenchConfig::default());
        let until = tokio::time::Instant::now() + std::time::Duration::from_secs(minutes * 60);
        println!("Collecting for {minutes} min...");
        while tokio::time::timeout_at(until, rx.changed()).await.is_ok_and(|r| r.is_ok()) {
            let s = rx.borrow_and_update().clone();
            println!(
                "  [{:?}] {}{}",
                s.phase,
                s.detail,
                s.last_error.map(|e| format!(" (error: {e})")).unwrap_or_default()
            );
        }
        engine.wait_idle().await?;
    }
    let (progress, gaps) = engine
        .db()
        .call(move |c| {
            let account = rc_store::accounts::primary_account(c)?
                .ok_or_else(|| rc_store::StoreError::Io(std::io::Error::other("no account tracked yet")))?;
            let s = rc_store::settings::app(c)?;
            let progress = rc_coach::benchmarks::progress(c, Some(account.id), &s)?;
            let role = role.or_else(|| progress.focus.roles.first().cloned());
            let gaps = match role {
                Some(r) => Some(rc_coach::benchmarks::gaps(
                    c,
                    account.id,
                    &s,
                    &r,
                    champion.as_deref(),
                    rc_store::metrics::GameFilter { limit: s.baseline_games, ..Default::default() },
                )?),
                None => None,
            };
            Ok((progress, gaps))
        })
        .await?;
    println!(
        "Tier {} | patch {:?} | focus {:?} {:?}",
        progress.tier_label, progress.patch, progress.focus.roles, progress.focus.champions
    );
    println!("Ladder: {:?}", progress.collector);
    println!("Games by patch: {:?}", progress.games_by_patch);
    for t in &progress.targets {
        println!("  {:<24} {}/{}", t.label, t.samples, t.target);
    }
    if let Some(g) = gaps {
        println!(
            "
Gaps vs {} ({} of your games):",
            g.reference.as_deref().unwrap_or("(not enough benchmark data)"),
            g.games
        );
        for x in &g.gaps {
            println!(
                "  {:<34} yours {:>8.2}  bench median {:>8.2}  better than {:>3.0}%{}",
                x.label,
                x.yours,
                x.benchmark_median,
                x.better_than_pct,
                if x.early { "  (early)" } else { "" }
            );
        }
    }
    Ok(())
}

async fn patterns(engine: &Arc<Engine>, games: u32, role: Option<String>, sessions: bool) -> anyhow::Result<()> {
    engine.start().await?;
    if let IdleOutcome::KeyProblem(k) = engine.wait_idle().await? {
        println!("(work paused: key {k:?}; analyzing what's stored)");
    }
    let (view, list) = engine
        .db()
        .call(move |c| {
            let account = rc_store::accounts::primary_account(c)?
                .ok_or_else(|| rc_store::StoreError::Io(std::io::Error::other("no account tracked yet")))?;
            let gap = rc_store::settings::app(c)?.session_gap_min;
            let view = rc_coach::patterns::patterns(c, account.id, games, role.as_deref(), gap)?;
            let list = if sessions { rc_coach::sessions::list(c, account.id, gap, 15)? } else { vec![] };
            Ok((view, list))
        })
        .await?;
    println!("{} games, {} wins. Roles: {:?}", view.games, view.wins, view.roles);
    println!("\nSignals (strongest first):");
    for s in &view.signals {
        println!("  [{:?}/{:?} score {:.2}] {} ({}): {}", s.polarity, s.severity, s.score, s.title, s.id, s.detail);
        for e in s.evidence.iter().take(3) {
            let t = e.t_s.map(|t| format!("{}:{:02} ", t as i64 / 60, t as i64 % 60)).unwrap_or_default();
            println!("      {} {t}{}", e.match_id, e.note);
        }
        if let Some(g) = &s.goal {
            println!("      goal: {} {} {}", g.metric, g.comparator.as_str(), g.target);
        }
    }
    println!("\nWin links:");
    for l in view.win_links.iter().take(15) {
        println!(
            "  {:<30} {:?} early={} higher-in-wins {}% p={:.3} wr above/below {:?}/{:?}  {}",
            l.label,
            l.strength,
            l.early,
            l.higher_in_wins_pct,
            l.p_value,
            l.win_rate_above,
            l.win_rate_below,
            l.summary
        );
    }
    if sessions {
        println!("\nSessions (newest first):");
        for s in &list {
            println!("  {} games={} rift={} {}W {}L", s.scope, s.games, s.rift_games, s.wins, s.losses);
        }
    }
    Ok(())
}

async fn ingest(engine: &Arc<Engine>, riot_id: &RiotId, platform: Platform, count: u32) -> anyhow::Result<()> {
    let mut events = engine.subscribe();
    engine.start().await?;
    let account_id = engine.track_and_ingest(riot_id, platform, count).await?;
    println!(
        "Ingesting up to {count} matches for {riot_id} ({platform}), account #{account_id}. Ctrl+C is safe; rerun to resume."
    );

    let printer = tokio::spawn(async move {
        let (mut matches, mut timelines) = (0u32, 0u32);
        while let Ok(ev) = events.recv().await {
            match ev {
                EngineEvent::MatchStored { match_id } => {
                    matches += 1;
                    println!("  [{matches:>3} matches | {timelines:>3} timelines] stored {match_id}");
                }
                EngineEvent::TimelineStored { match_id } => {
                    timelines += 1;
                    println!("  [{matches:>3} matches | {timelines:>3} timelines] timeline {match_id}");
                }
                EngineEvent::JobRetrying { label, error, retry_in_s, .. } => {
                    println!("  retry in {retry_in_s}s: {label}: {error}");
                }
                EngineEvent::JobFailed { label, error, .. } => println!("  FAILED: {label}: {error}"),
                EngineEvent::KeyStatusChanged { status } => println!("  key status: {status:?}"),
                EngineEvent::KeyGenerationChanged { key_gen_id } => {
                    println!("  API key changed (key generation {key_gen_id}); re-resolving accounts");
                }
                _ => {}
            }
        }
    });

    let outcome = engine.wait_idle().await?;
    printer.abort();
    let counts = engine.db().call(move |c| rc_store::matches::account_match_counts(c, account_id)).await?;
    println!("Stored for {riot_id}: {} matches, {} with timelines.", counts.matches, counts.with_timeline);
    match outcome {
        IdleOutcome::Drained => {
            let failed = engine.status().await?.recent_failures;
            if !failed.is_empty() {
                println!("{} job(s) failed; see `rc-cli status`.", failed.len());
            }
            Ok(())
        }
        IdleOutcome::KeyProblem(status) => bail!("stopped: API key problem ({status:?}). Fix the key and rerun."),
    }
}

async fn status(engine: &Arc<Engine>, paths: &AppPaths) -> anyhow::Result<()> {
    let s = engine.status().await?;
    println!("Data folder: {}", paths.root.display());
    println!(
        "Jobs: {} ready, {} scheduled, {} running, {} done, {} failed",
        s.jobs.ready, s.jobs.scheduled, s.jobs.running, s.jobs.done, s.jobs.failed
    );
    let accounts = engine.db().call(|c| rc_store::accounts::list_accounts(c)).await?;
    for a in accounts {
        let id = a.id;
        let counts = engine.db().call(move |c| rc_store::matches::account_match_counts(c, id)).await?;
        println!(
            "Account #{}: {}#{} ({}){} - {} matches, {} timelines",
            a.id,
            a.game_name,
            a.tag_line,
            a.platform,
            if a.is_primary { " [primary]" } else { "" },
            counts.matches,
            counts.with_timeline
        );
    }
    for f in &s.recent_failures {
        println!("Failed: {} ({}): {}", f.dedupe_key, f.kind, f.last_error.as_deref().unwrap_or("?"));
    }
    Ok(())
}

fn summary(paths: &AppPaths) -> anyhow::Result<()> {
    let db = rc_store::Db::open(&paths.db_file)?;
    let (account, stats) = db.with(|c| {
        let Some(a) = rc_store::accounts::primary_account(c)? else { return Ok((None, vec![])) };
        let stats = rc_store::matches::role_champion_stats(c, a.id)?;
        Ok((Some(a), stats))
    })?;
    let Some(account) = account else { bail!("no tracked account yet; run `ingest` first") };
    let total: i64 = stats.iter().map(|s| s.games).sum();
    println!("{}#{}: {total} standard Summoner's Rift games (remakes excluded)\n", account.game_name, account.tag_line);

    let mut roles: Vec<(String, i64, i64)> = Vec::new();
    for s in &stats {
        match roles.iter_mut().find(|r| r.0 == s.team_position) {
            Some(r) => {
                r.1 += s.games;
                r.2 += s.wins;
            }
            None => roles.push((s.team_position.clone(), s.games, s.wins)),
        }
    }
    roles.sort_by_key(|r| std::cmp::Reverse(r.1));
    println!("{:<10} {:>6} {:>7} {:>6}", "Role", "Games", "Share", "Win%");
    for (role, games, wins) in &roles {
        println!("{role:<10} {games:>6} {:>6.0}% {:>5.0}%", pct(*games, total), pct(*wins, *games));
    }
    println!("\n{:<10} {:<14} {:>6} {:>6}", "Role", "Champion", "Games", "Win%");
    for s in stats.iter().take(15) {
        println!("{:<10} {:<14} {:>6} {:>5.0}%", s.team_position, s.champion_name, s.games, pct(s.wins, s.games));
    }
    Ok(())
}

fn pct(n: i64, d: i64) -> f64 {
    if d == 0 { 0.0 } else { n as f64 * 100.0 / d as f64 }
}

async fn smoke(engine: &Arc<Engine>, riot_id: &RiotId, platform: Platform) -> anyhow::Result<()> {
    let riot = engine.riot();
    let region = platform.match_region();
    let report = |name: &str, r: Result<String, rc_riot::RiotError>| match r {
        Ok(detail) => println!("  OK    {name:<36} {detail}"),
        Err(e) => println!("  FAIL  {name:<36} {e}"),
    };

    println!("Smoke test for {riot_id} on {platform} / {region}:");
    let account = riot.account_by_riot_id(platform.account_region(), riot_id).await;
    let puuid = match account {
        Ok(a) => {
            report("account-v1 by-riot-id", Ok(format!("puuid ({} chars)", a.puuid.len())));
            a.puuid
        }
        Err(e) => {
            report("account-v1 by-riot-id", Err(e));
            bail!("cannot continue without a PUUID");
        }
    };
    let ids = riot.match_ids(region, &puuid, &MatchIdQuery { count: 5, ..Default::default() }).await;
    let first = ids.as_ref().ok().and_then(|v| v.first().cloned());
    report("match-v5 ids by-puuid", ids.map(|v| format!("{} ids", v.len())));
    if let Some(id) = first {
        report("match-v5 match", riot.match_raw(region, &id).await.map(|b| format!("{id}: {} KB", b.len() / 1024)));
        report(
            "match-v5 timeline",
            riot.timeline_raw(region, &id).await.map(|b| format!("{id}: {} KB", b.len() / 1024)),
        );
    }
    report(
        "league-v4 entries by-puuid",
        riot.league_entries(platform, &puuid).await.map(|v| {
            v.iter()
                .map(|e| {
                    format!("{} {} {}", e.queue_type, e.tier.as_deref().unwrap_or("?"), e.rank.as_deref().unwrap_or(""))
                })
                .collect::<Vec<_>>()
                .join(", ")
        }),
    );
    report(
        "champion-mastery-v4 top by-puuid",
        riot.mastery_top(platform, &puuid, 3).await.map(|v| format!("{} entries", v.len())),
    );
    report(
        "spectator-v5 active game by-puuid",
        riot.active_game_raw(platform, &puuid)
            .await
            .map(|g| if g.is_some() { "in a game".into() } else { "not in a game (404, expected)".into() }),
    );
    Ok(())
}

/// The primary account's Riot ID and most played champion, for `mock-live`.
fn mock_defaults(paths: &AppPaths) -> anyhow::Result<(Option<String>, Option<String>)> {
    if !paths.db_file.exists() {
        return Ok((None, None));
    }
    let db = rc_store::Db::open(&paths.db_file)?;
    Ok(db.with(|c| {
        let Some(a) = rc_store::accounts::primary_account(c)? else { return Ok((None, None)) };
        let champ = rc_store::matches::role_champion_stats(c, a.id)?
            .into_iter()
            .max_by_key(|s| s.games)
            .map(|s| s.champion_name);
        Ok((Some(format!("{}#{}", a.game_name, a.tag_line)), champ))
    })?)
}
