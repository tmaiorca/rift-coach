// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::sync::{Arc, Mutex};

use rc_common::secret::load_riot_api_key;
use rc_common::{AppPaths, KeySource};
use rc_jobs::{Engine, EngineConfig};
use tauri::{Emitter, Manager};
use tokio::sync::broadcast::error::RecvError;

/// Shared state for commands.
pub struct AppState {
    pub engine: Arc<Engine>,
    pub paths: AppPaths,
    pub key_source: Mutex<Option<KeySource>>,
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

    tauri::Builder::default()
        .manage(AppState { engine: engine.clone(), paths, key_source: Mutex::new(key_source) })
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
                            let _ = handle.emit("engine-event", ev);
                        }
                        Err(RecvError::Lagged(n)) => tracing::debug!(skipped = n, "UI event forwarder lagged"),
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(&format!("Rift Coach {}", env!("CARGO_PKG_VERSION")));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::start_ingest,
            commands::list_matches,
            commands::champion_icon,
            commands::reload_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rift Coach");
}
