//! Auto-update from GitHub releases. The updater only installs packages
//! signed with the project's update key (its public half is in
//! `tauri.conf.json`), so a tampered download is refused.

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_updater::UpdaterExt;

use crate::AppState;
use crate::commands::{CmdResult, err};

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
}

/// Looks for a newer version. `None` when up to date.
#[tauri::command]
pub async fn check_update(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<UpdateInfo>> {
    let update = app.updater().map_err(err)?.check().await.map_err(err)?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
    });
    *state.update.lock().await = update;
    Ok(info)
}

/// Downloads and installs the update found by [`check_update`], then restarts.
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let Some(update) = state.update.lock().await.take() else {
        return Err("No update to install. Check for updates first.".into());
    };
    tracing::info!(version = %update.version, "installing update");
    update.download_and_install(|_, _| {}, || {}).await.map_err(err)?;
    app.request_restart();
    Ok(())
}
