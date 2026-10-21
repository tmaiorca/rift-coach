// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod data;
mod desktop;
mod mcp_host;
mod updates;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use rc_common::secret::load_riot_api_key;
use rc_common::{AppPaths, KeySource};
use rc_jobs::{BenchConfig, BenchState, Engine, EngineConfig, EngineEvent, LiveConfig, LiveHandle};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;

/// Shared state for commands.
pub struct AppState {
    pub engine: Arc<Engine>,
    pub paths: AppPaths,
    pub key_source: Mutex<Option<KeySource>>,
    pub live: LiveHandle,
    pub bench: watch::Receiver<BenchState>,
    /// Mirrors `desktop.close_to_tray` for the (synchronous) window events.
    pub close_to_tray: AtomicBool,
    pub tray_hint_shown: AtomicBool,
    pub mcp: tokio::sync::Mutex<mcp_host::McpRuntime>,
    /// An update found by `check_update`, waiting to be installed.
    pub update: tokio::sync::Mutex<Option<tauri_plugin_updater::Update>>,
}

/// Starts the read-only MCP server on stdin/stdout for clients that launch
/// a command (Claude Desktop): `rift-coach.exe --mcp-stdio`. No window.
const MCP_STDIO_ARG: &str = "--mcp-stdio";

fn run_mcp_stdio(paths: &AppPaths) -> i32 {
    let _log = rc_common::logging::init(&paths.logs_dir, "rift-coach-mcp").ok();
    let db = match rc_store::Db::open_read_only(&paths.db_file) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Rift Coach MCP: can't open the database: {e}");
            return 1;
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Rift Coach MCP: {e}");
            return 1;
        }
    };
    match rt.block_on(rc_mcp::stdio::run(db)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Rift Coach MCP: {e}");
            1
        }
    }
}

/// Carries out a staged restore or delete. The previous instance may still
/// be closing (it restarts us), so retry for a few seconds.
fn apply_pending(paths: &AppPaths) -> Option<rc_store::maintenance::Applied> {
    let mut last = None;
    for _ in 0..20 {
        match rc_store::maintenance::apply_pending(paths) {
            Ok(a) => return Some(a),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    eprintln!("Rift Coach: couldn't finish a restore or delete: {last:?}");
    None
}

/// Points the live poller somewhere else (e.g. `rc-cli mock-live`).
const LIVE_URL_ENV: &str = "RIFT_COACH_LIVE_URL";

/// Windows notification for a finished game's analysis or AI review.
async fn notify_post_game(app: &AppHandle, engine: &Engine, ev: &EngineEvent) {
    let (title, body) = match ev {
        EngineEvent::PostGameReady { review_queued: false, .. } => {
            ("Your game is analyzed", "Open Rift Coach to see this game's numbers and patterns.")
        }
        EngineEvent::ReviewReady { match_id, .. } => {
            let id = match_id.clone();
            let live = engine.db().call(move |c| rc_store::live::by_match(c, &id)).await.ok().flatten();
            if live.is_none() {
                return;
            }
            ("AI review ready", "Your post-game review is waiting in Rift Coach.")
        }
        _ => return,
    };
    let enabled = engine.db().call(|c| rc_store::settings::app(c)).await.map(|s| s.live.notify).unwrap_or(false);
    if enabled && let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %e, "could not show a notification");
    }
}

fn main() {
    let paths = AppPaths::resolve().expect("cannot create the app data folder");
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == MCP_STDIO_ARG) {
        std::process::exit(run_mcp_stdio(&paths));
    }
    let applied = apply_pending(&paths);
    let _log = rc_common::logging::init(&paths.logs_dir, "rift-coach").expect("cannot start logging");
    tracing::info!(data_dir = %paths.root.display(), version = env!("CARGO_PKG_VERSION"), ?applied, "Rift Coach starting");
    let start_hidden = args.iter().any(|a| a == desktop::MINIMIZED_ARG);

    let (key, key_source) = match load_riot_api_key() {
        Some((k, s)) => (Some(k), Some(s)),
        None => (None, None),
    };
    tracing::info!(?key_source, "Riot API key lookup");
    let engine = rc_jobs::build_engine(&paths, key, EngineConfig::default()).expect("cannot open the database");
    let live_config = LiveConfig {
        url: std::env::var(LIVE_URL_ENV).unwrap_or_else(|_| rc_live::client::DEFAULT_URL.into()),
        ..Default::default()
    };
    // The poller needs a Tokio runtime: start it on Tauri's.
    let (live, bench) = tauri::async_runtime::block_on(async {
        let live = rc_jobs::spawn_live(engine.clone(), live_config).expect("invalid live client URL");
        (live, rc_jobs::spawn_benchmarks(engine.clone(), BenchConfig::default()))
    });

    tauri::Builder::default()
        // A second launch (e.g. the Start menu while it's in the tray) brings
        // this window back instead of starting another copy.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| desktop::show_main(app)))
        // Remembers window size and position between runs (not visibility:
        // starting in the tray is decided per launch).
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all() & !tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        // "Start with Windows": a Run entry that starts Rift Coach in the tray.
        .plugin(tauri_plugin_autostart::Builder::new().args([desktop::MINIMIZED_ARG]).build())
        // Native save/open dialogs for backups and exports.
        .plugin(tauri_plugin_dialog::init())
        // Signed updates from GitHub releases.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Opens external links (the Guide) in the default browser.
        .plugin(tauri_plugin_opener::init())
        // "Your game is analyzed" / "AI review ready" after a game.
        .plugin(tauri_plugin_notification::init())
        .manage(AppState {
            engine: engine.clone(),
            paths,
            key_source: Mutex::new(key_source),
            live: live.clone(),
            bench,
            close_to_tray: AtomicBool::new(true),
            tray_hint_shown: AtomicBool::new(false),
            mcp: Default::default(),
            update: Default::default(),
        })
        .on_window_event(desktop::on_window_event)
        .setup(move |app| {
            desktop::build_tray(app.handle())?;
            if start_hidden && let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }
            // Desktop settings, the MCP server, and first-run detection.
            let handle = app.handle().clone();
            let db = engine.db().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                let settings = db
                    .call(|c| {
                        let mut s = rc_store::settings::app(c)?;
                        // Installs from before the setup wizard existed are set up already.
                        if !s.desktop.onboarded && !rc_store::accounts::list_accounts(c)?.is_empty() {
                            s.desktop.onboarded = true;
                            rc_store::settings::set(c, rc_store::settings::APP, &s)?;
                        }
                        Ok(s)
                    })
                    .await;
                if let Ok(s) = settings {
                    desktop::apply_settings(&state, &s);
                }
                mcp_host::apply(&state).await;
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = engine.start().await {
                    tracing::error!(error = %e, "could not start background workers");
                    return;
                }
                // Forward engine progress to the UI.
                let mut events = engine.subscribe();
                loop {
                    match events.recv().await {
                        Ok(ev) => {
                            notify_post_game(&handle, &engine, &ev).await;
                            let _ = handle.emit("engine-event", ev);
                        }
                        Err(RecvError::Lagged(n)) => tracing::debug!(skipped = n, "UI event forwarder lagged"),
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            // Forward live view updates (every couple of seconds in game).
            let handle = app.handle().clone();
            let mut live = live.state.clone();
            tauri::async_runtime::spawn(async move {
                while live.changed().await.is_ok() {
                    let state = live.borrow_and_update().clone();
                    let _ = handle.emit("live-update", state);
                }
            });
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(&format!("Rift Coach {}", env!("CARGO_PKG_VERSION")));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::recent_activity,
            commands::start_ingest,
            commands::list_accounts,
            commands::set_active_account,
            commands::set_primary_account,
            commands::remove_account,
            commands::list_matches,
            commands::match_detail,
            commands::dashboard,
            commands::metric_definitions,
            commands::patterns,
            commands::list_goals,
            commands::create_goal,
            commands::set_goal_status,
            commands::delete_goal,
            commands::list_sessions,
            commands::session_detail,
            commands::generate_session_review,
            commands::session_facts,
            commands::ai_usage,
            commands::generate_review,
            commands::review_facts,
            commands::list_reports,
            commands::ai_providers,
            commands::set_ai_key,
            commands::delete_ai_key,
            commands::list_ai_models,
            commands::test_ai,
            commands::get_settings,
            commands::save_settings,
            commands::get_ui_state,
            commands::save_ui_state,
            commands::reload_key,
            commands::set_riot_key,
            commands::delete_riot_key,
            commands::champions,
            commands::champion_icon,
            commands::item_icon,
            commands::spell_icon,
            commands::champion_splash,
            commands::minimap,
            commands::live_state,
            commands::live_games,
            commands::planner_items,
            commands::build_plan,
            commands::set_build_plan,
            commands::open_live_window,
            commands::benchmark_status,
            commands::benchmark_gaps,
            commands::ranks,
            commands::list_weeks,
            commands::week_detail,
            commands::generate_weekly_report,
            commands::weekly_facts,
            commands::generate_game_plan,
            commands::champion_list,
            commands::champion_detail,
            desktop::autostart_enabled,
            desktop::set_autostart,
            mcp_host::mcp_status,
            mcp_host::mcp_token,
            mcp_host::mcp_new_token,
            data::backup_database,
            data::export_games_csv,
            data::restore_database,
            data::delete_all_data,
            data::open_data_folder,
            updates::check_update,
            updates::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rift Coach");
}
