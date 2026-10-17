//! Commands callable from the UI via `invoke`. Errors are returned as
//! display strings; nothing here ever returns or logs the API key.

use base64::Engine as _;
use rc_common::KeySource;
use rc_common::queues::{is_standard_rift, queue_name};
use rc_common::secret::load_riot_api_key;
use rc_jobs::{EngineStatus, KeyStatus};
use rc_riot::{Platform, RiotId};
use rc_store::accounts::{self, Account};
use rc_store::matches::{self, MatchCounts, MatchSummary};
use serde::Serialize;
use tauri::State;

use crate::AppState;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct StatusView {
    pub key_source: Option<KeySource>,
    pub engine: EngineStatus,
    pub account: Option<Account>,
    pub counts: Option<MatchCounts>,
    pub data_dir: String,
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> CmdResult<StatusView> {
    let engine = state.engine.status().await.map_err(err)?;
    let (account, counts) = state
        .engine
        .db()
        .call(|c| {
            let account = accounts::primary_account(c)?;
            let counts = account.as_ref().map(|a| matches::account_match_counts(c, a.id)).transpose()?;
            Ok((account, counts))
        })
        .await
        .map_err(err)?;
    let key_source = *state.key_source.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(StatusView { key_source, engine, account, counts, data_dir: state.paths.root.display().to_string() })
}

#[tauri::command]
pub async fn start_ingest(state: State<'_, AppState>, riot_id: String, platform: String, count: u32) -> CmdResult<i64> {
    let riot_id: RiotId = riot_id.parse()?;
    let platform: Platform = platform.parse()?;
    if !(1..=1000).contains(&count) {
        return Err("count must be between 1 and 1000".into());
    }
    tracing::info!(%riot_id, %platform, count, "ingest requested from UI");
    state.engine.track_and_ingest(&riot_id, platform, count).await.map_err(err)
}

#[derive(Serialize)]
pub struct MatchRow {
    #[serde(flatten)]
    pub summary: MatchSummary,
    pub queue_name: &'static str,
    pub is_standard_rift: bool,
}

#[tauri::command]
pub async fn list_matches(state: State<'_, AppState>, limit: u32, offset: u32) -> CmdResult<Vec<MatchRow>> {
    let rows = state
        .engine
        .db()
        .call(move |c| match accounts::primary_account(c)? {
            Some(a) => matches::list_account_matches(c, a.id, limit.min(500), offset),
            None => Ok(vec![]),
        })
        .await
        .map_err(err)?;
    Ok(rows
        .into_iter()
        .map(|s| MatchRow {
            queue_name: queue_name(s.queue_id),
            is_standard_rift: is_standard_rift(s.queue_id, s.map_id),
            summary: s,
        })
        .collect())
}

/// Champion square icon as a `data:` URL (downloaded once, then served from
/// the local cache).
#[tauri::command]
pub async fn champion_icon(state: State<'_, AppState>, champion_id: i32, patch: String) -> CmdResult<String> {
    let dd = state.engine.ddragon();
    let version = dd.version_for_patch(&patch).await.map_err(err)?;
    let path = dd.champion_icon(&version, champion_id).await.map_err(err)?;
    let bytes = tokio::fs::read(&path).await.map_err(err)?;
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Re-read the key from the environment (after the user changes it).
#[tauri::command]
pub async fn reload_key(state: State<'_, AppState>) -> CmdResult<KeyStatus> {
    let found = load_riot_api_key();
    let source = found.as_ref().map(|(_, s)| *s);
    *state.key_source.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = source;
    state.engine.set_key(found.map(|(k, _)| k));
    Ok(state.engine.key_status())
}
