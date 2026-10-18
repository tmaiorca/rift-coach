// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::sync::{Arc, Mutex};

use rc_common::secret::load_riot_api_key;
use rc_common::{AppPaths, KeySource};
use rc_jobs::{Engine, EngineConfig, EngineEvent, LiveConfig, LiveState};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;

/// Shared state for commands.
pub struct AppState {
    pub engine: Arc<Engine>,
    pub paths: AppPaths,
    pub key_source: Mutex<Option<KeySource>>,
    pub live: watch::Receiver<LiveState>,
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
    let _log = rc_common::logging::init(&paths.logs_dir, "rift-coach").expect("cannot start logging");
    tracing::info!(data_dir = %paths.root.display(), version = env!("CARGO_PKG_VERSION"), "Rift Coach starting");

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
    let live = tauri::async_runtime::block_on(async { rc_jobs::spawn_live(engine.clone(), live_config) })
        .expect("invalid live client URL");

    tauri::Builder::default()
        // Remembers window size and position between runs.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // Opens external links (the Guide) in the default browser.
        .plugin(tauri_plugin_opener::init())
        // "Your game is analyzed" / "AI review ready" after a game.
        .plugin(tauri_plugin_notification::init())
        .manage(AppState { engine: engine.clone(), paths, key_source: Mutex::new(key_source), live: live.clone() })
        .setup(move |app| {
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
            let mut live = live.clone();
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rift Coach");
}
