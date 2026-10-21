//! Data management: back up, restore, export games as CSV, open the data
//! folder, and delete everything. Files are chosen in native dialogs on the
//! Rust side, so the UI needs no file system access.

use std::path::PathBuf;

use rc_common::credentials;
use rc_store::maintenance::{self, BackupInfo};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::AppState;
use crate::commands::{CmdResult, active_account_id, err};

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Runs a blocking native dialog off the async runtime.
async fn pick(
    f: impl FnOnce() -> Option<tauri_plugin_dialog::FilePath> + Send + 'static,
) -> CmdResult<Option<PathBuf>> {
    let chosen = tokio::task::spawn_blocking(f).await.map_err(err)?;
    chosen.map(|p| p.into_path().map_err(err)).transpose()
}

/// Saves a copy of the database. Returns where, or `None` if cancelled.
#[tauri::command]
pub async fn backup_database(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<String>> {
    let name = format!("rift-coach-backup-{}.db", today());
    let d = app
        .dialog()
        .file()
        .set_title("Back up Rift Coach")
        .add_filter("Rift Coach backup", &["db"])
        .set_file_name(name);
    let Some(dest) = pick(move || d.blocking_save_file()).await? else { return Ok(None) };
    let to = dest.clone();
    state.engine.db().call(move |c| maintenance::backup_to(c, &to)).await.map_err(err)?;
    state.engine.log_activity("info", format!("Backed up to {}", dest.display())).await;
    Ok(Some(dest.display().to_string()))
}

/// Your Summoner's Rift games with every metric, as CSV.
#[tauri::command]
pub async fn export_games_csv(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<String>> {
    let account_id = active_account_id(&state).await?;
    let csv = state.engine.db().call(move |c| rc_coach::export::games_csv(c, account_id)).await.map_err(err)?;
    let d = app
        .dialog()
        .file()
        .set_title("Export your games")
        .add_filter("CSV", &["csv"])
        .set_file_name(format!("rift-coach-games-{}.csv", today()));
    let Some(dest) = pick(move || d.blocking_save_file()).await? else { return Ok(None) };
    tokio::fs::write(&dest, csv).await.map_err(err)?;
    Ok(Some(dest.display().to_string()))
}

/// Picks a backup, checks it, and restarts into it. The current database is
/// kept next to it as `rift-coach.before-restore.db`.
#[tauri::command]
pub async fn restore_database(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<BackupInfo>> {
    let d = app.dialog().file().set_title("Restore a Rift Coach backup").add_filter("Rift Coach backup", &["db"]);
    let Some(src) = pick(move || d.blocking_pick_file()).await? else { return Ok(None) };
    let paths = state.paths.clone();
    let info = tokio::task::spawn_blocking(move || maintenance::stage_restore(&paths, &src))
        .await
        .map_err(err)?
        .map_err(err)?;
    tracing::info!(matches = info.matches, "restore staged; restarting");
    app.request_restart();
    Ok(Some(info))
}

/// Deletes every game, report, goal and setting (and, if asked, the saved
/// keys), then restarts fresh.
#[tauri::command]
pub async fn delete_all_data(app: AppHandle, state: State<'_, AppState>, forget_keys: bool) -> CmdResult<()> {
    if forget_keys {
        let mut names = vec![credentials::RIOT_API_KEY.to_owned(), credentials::MCP_TOKEN.to_owned()];
        names.extend(rc_ai::ProviderKind::ALL.iter().map(|k| k.credential_name()));
        for n in names {
            if let Err(e) = credentials::delete_secret(&n) {
                tracing::warn!(error = %e, "couldn't delete a saved key");
            }
        }
        use tauri_plugin_autostart::ManagerExt;
        let _ = app.autolaunch().disable();
    }
    maintenance::stage_delete(&state.paths).map_err(err)?;
    tracing::info!(forget_keys, "delete of all data staged; restarting");
    app.request_restart();
    Ok(())
}

#[tauri::command]
pub fn open_data_folder(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    app.opener().open_path(state.paths.root.display().to_string(), None::<&str>).map_err(err)
}
