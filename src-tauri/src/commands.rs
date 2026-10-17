//! Commands callable from the UI via `invoke`. Errors are returned as
//! display strings. Keys go *in* through the setters below and are written
//! straight to Windows Credential Manager; no command ever returns a key.

use std::path::Path;

use base64::Engine as _;
use rc_ai::{AiClient, CompletionRequest, ProviderConfig, ProviderKind};
use rc_coach::detail::MatchDetail;
use rc_common::queues::{is_standard_rift, queue_name};
use rc_common::secret::load_riot_api_key;
use rc_common::{KeySource, Secret, credentials};
use rc_jobs::{EngineStatus, KeyStatus};
use rc_metrics::defs::{METRICS, MetricDef};
use rc_riot::{Platform, RiotId};
use rc_store::accounts::{self, Account};
use rc_store::activity::{self, ActivityEntry};
use rc_store::matches::{self, MatchCounts, MatchSummary, RoleChampionStat};
use rc_store::metrics::GameMetrics;
use rc_store::reports::{self, Report};
use rc_store::rusqlite::Connection;
use rc_store::settings::{self, AppSettings};
use serde::Serialize;
use tauri::State;

use crate::AppState;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The account the UI is showing: the chosen one, else the primary.
fn active_account(c: &Connection) -> rc_store::Result<Option<Account>> {
    if let Some(id) = settings::app(c)?.active_account_id
        && let Some(a) = accounts::get_account(c, id)?
    {
        return Ok(Some(a));
    }
    accounts::primary_account(c)
}

async fn active_account_id(state: &AppState) -> CmdResult<i64> {
    state
        .engine
        .db()
        .call(|c| active_account(c))
        .await
        .map_err(err)?
        .map(|a| a.id)
        .ok_or_else(|| "No account yet. Add your Riot ID on the Players page.".into())
}

async fn data_url(path: &Path, mime: &str) -> CmdResult<String> {
    let bytes = tokio::fs::read(path).await.map_err(err)?;
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

// ---------- status ----------

#[derive(Serialize)]
pub struct StatusView {
    pub key_source: Option<KeySource>,
    pub engine: EngineStatus,
    pub account: Option<Account>,
    pub counts: Option<MatchCounts>,
    pub data_dir: String,
    pub version: &'static str,
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> CmdResult<StatusView> {
    let engine = state.engine.status().await.map_err(err)?;
    let (account, counts) = state
        .engine
        .db()
        .call(|c| {
            let account = active_account(c)?;
            let counts = account.as_ref().map(|a| matches::account_match_counts(c, a.id)).transpose()?;
            Ok((account, counts))
        })
        .await
        .map_err(err)?;
    let key_source = *state.key_source.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(StatusView {
        key_source,
        engine,
        account,
        counts,
        data_dir: state.paths.root.display().to_string(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[tauri::command]
pub async fn recent_activity(state: State<'_, AppState>, limit: u32) -> CmdResult<Vec<ActivityEntry>> {
    state.engine.db().call(move |c| activity::recent(c, limit.min(500))).await.map_err(err)
}

// ---------- accounts / players ----------

#[tauri::command]
pub async fn start_ingest(state: State<'_, AppState>, riot_id: String, platform: String, count: u32) -> CmdResult<i64> {
    let riot_id: RiotId = riot_id.parse()?;
    let platform: Platform = platform.parse()?;
    if !(1..=1000).contains(&count) {
        return Err("count must be between 1 and 1000".into());
    }
    tracing::info!(%riot_id, %platform, count, "ingest requested from UI");
    let id = state.engine.track_and_ingest(&riot_id, platform, count).await.map_err(err)?;
    state.engine.log_activity("info", format!("Sync requested: {riot_id} ({platform}), newest {count} matches")).await;
    Ok(id)
}

#[derive(Serialize)]
pub struct AccountView {
    #[serde(flatten)]
    pub account: Account,
    pub counts: MatchCounts,
    pub is_active: bool,
}

#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> CmdResult<Vec<AccountView>> {
    state
        .engine
        .db()
        .call(|c| {
            let active = active_account(c)?.map(|a| a.id);
            accounts::list_accounts(c)?
                .into_iter()
                .map(|a| {
                    Ok(AccountView {
                        counts: matches::account_match_counts(c, a.id)?,
                        is_active: Some(a.id) == active,
                        account: a,
                    })
                })
                .collect()
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_active_account(state: State<'_, AppState>, account_id: i64) -> CmdResult<()> {
    state
        .engine
        .db()
        .call(move |c| {
            let mut s = settings::app(c)?;
            s.active_account_id = Some(account_id);
            settings::set(c, settings::APP, &s)
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn set_primary_account(state: State<'_, AppState>, account_id: i64) -> CmdResult<()> {
    state.engine.db().call(move |c| accounts::set_primary(c, account_id)).await.map_err(err)
}

/// Stops tracking an account. Its matches stay (other accounts may share
/// them); its links, PUUIDs and reports are removed.
#[tauri::command]
pub async fn remove_account(state: State<'_, AppState>, account_id: i64) -> CmdResult<()> {
    state.engine.db().call(move |c| accounts::delete_account(c, account_id)).await.map_err(err)
}

// ---------- matches ----------

#[derive(Serialize)]
pub struct MatchRow {
    #[serde(flatten)]
    pub summary: MatchSummary,
    pub queue_name: &'static str,
    pub is_standard_rift: bool,
    pub has_review: bool,
}

#[tauri::command]
pub async fn list_matches(state: State<'_, AppState>, limit: u32, offset: u32) -> CmdResult<Vec<MatchRow>> {
    state
        .engine
        .db()
        .call(move |c| {
            let Some(a) = active_account(c)? else { return Ok(vec![]) };
            let show_other = settings::app(c)?.show_other_queues;
            matches::list_account_matches(c, a.id, limit.min(1000), offset)?
                .into_iter()
                .filter(|s| show_other || is_standard_rift(s.queue_id, s.map_id))
                .map(|s| {
                    Ok(MatchRow {
                        queue_name: queue_name(s.queue_id),
                        is_standard_rift: is_standard_rift(s.queue_id, s.map_id),
                        has_review: reports::has_report(c, rc_coach::detail::POST_GAME, a.id, &s.match_id)?,
                        summary: s,
                    })
                })
                .collect()
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn match_detail(state: State<'_, AppState>, match_id: String) -> CmdResult<MatchDetail> {
    let account_id = active_account_id(&state).await?;
    state
        .engine
        .db()
        .call(move |c| {
            let games = settings::app(c)?.baseline_games;
            Ok(rc_coach::detail::match_detail(c, account_id, &match_id, games))
        })
        .await
        .map_err(err)?
        .map_err(err)
}

#[derive(Serialize)]
pub struct DashboardView {
    pub account: Option<Account>,
    pub baseline_games: u32,
    pub games: Vec<GameMetrics>,
    pub roles: Vec<RoleChampionStat>,
}

#[tauri::command]
pub async fn dashboard(state: State<'_, AppState>) -> CmdResult<DashboardView> {
    state
        .engine
        .db()
        .call(|c| {
            let s = settings::app(c)?;
            let Some(a) = active_account(c)? else {
                return Ok(DashboardView {
                    account: None,
                    baseline_games: s.baseline_games,
                    games: vec![],
                    roles: vec![],
                });
            };
            Ok(DashboardView {
                games: rc_store::metrics::recent_game_metrics(c, a.id, s.baseline_games)?,
                roles: matches::role_champion_stats(c, a.id)?,
                baseline_games: s.baseline_games,
                account: Some(a),
            })
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub fn metric_definitions() -> Vec<MetricDef> {
    METRICS.to_vec()
}

// ---------- AI ----------

#[tauri::command]
pub async fn generate_review(state: State<'_, AppState>, match_id: String, force: bool) -> CmdResult<Report> {
    let account_id = active_account_id(&state).await?;
    let settings = state.engine.db().call(|c| settings::app(c)).await.map_err(err)?;
    let client = rc_coach::provider::client_from_settings(&settings.ai).map_err(err)?;
    let id = match_id.clone();
    let patch =
        state.engine.db().call(move |c| matches::patch_of_match(c, &id)).await.map_err(err)?.unwrap_or_default();
    let items = state.engine.item_catalog(&patch).await.ok();
    let opts = rc_coach::review::ReviewOptions {
        baseline_games: settings.baseline_games,
        max_tokens: settings.ai.max_tokens,
        force,
    };
    let result =
        rc_coach::review::post_game_review(state.engine.db(), &client, account_id, &match_id, &opts, items).await;
    match &result {
        Ok(r) => state.engine.log_activity("info", format!("AI review ready for {match_id} ({})", r.model)).await,
        Err(e) => state.engine.log_activity("error", format!("AI review failed for {match_id}: {e}")).await,
    }
    result.map_err(err)
}

/// The exact facts an AI review of this match is given.
#[tauri::command]
pub async fn review_facts(state: State<'_, AppState>, match_id: String) -> CmdResult<serde_json::Value> {
    let account_id = active_account_id(&state).await?;
    let games = state.engine.db().call(|c| Ok(settings::app(c)?.baseline_games)).await.map_err(err)?;
    rc_coach::review::post_game_facts(state.engine.db(), account_id, &match_id, games, None).await.map_err(err)
}

#[tauri::command]
pub async fn list_reports(state: State<'_, AppState>, limit: u32) -> CmdResult<Vec<Report>> {
    let account_id = active_account_id(&state).await.ok();
    state.engine.db().call(move |c| reports::list(c, account_id, limit.min(500))).await.map_err(err)
}

#[derive(Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    pub key_required: bool,
    pub has_key: bool,
}

#[tauri::command]
pub fn ai_providers() -> Vec<ProviderInfo> {
    ProviderKind::ALL
        .iter()
        .map(|k| ProviderInfo {
            id: k.as_str(),
            label: k.label(),
            default_base_url: k.default_base_url(),
            default_model: k.default_model(),
            key_required: k.key_required(),
            has_key: credentials::has_secret(&k.credential_name()),
        })
        .collect()
}

#[tauri::command]
pub fn set_ai_key(provider: String, key: String) -> CmdResult<()> {
    let kind: ProviderKind = provider.parse()?;
    let key = key.trim();
    if key.is_empty() {
        return Err("The key is empty.".into());
    }
    credentials::store_secret(&kind.credential_name(), &Secret::new(key)).map_err(err)?;
    tracing::info!(provider = kind.as_str(), "AI key saved to Credential Manager");
    Ok(())
}

#[tauri::command]
pub fn delete_ai_key(provider: String) -> CmdResult<()> {
    let kind: ProviderKind = provider.parse()?;
    credentials::delete_secret(&kind.credential_name()).map_err(err)
}

fn client_for(provider: &str, model: &str, base_url: Option<String>) -> CmdResult<AiClient> {
    let kind: ProviderKind = provider.parse()?;
    let key = credentials::load_secret(&kind.credential_name()).map_err(err)?;
    AiClient::new(ProviderConfig { kind, model: model.to_owned(), base_url }, key).map_err(err)
}

#[tauri::command]
pub async fn list_ai_models(provider: String, base_url: Option<String>) -> CmdResult<Vec<String>> {
    client_for(&provider, "", base_url)?.list_models().await.map_err(err)
}

/// Sends a tiny prompt to confirm the provider, key and model work.
#[tauri::command]
pub async fn test_ai(provider: String, model: String, base_url: Option<String>) -> CmdResult<String> {
    let client = client_for(&provider, &model, base_url)?;
    let req = CompletionRequest {
        system: "You are a connectivity check.".into(),
        user: "Reply with the single word OK.".into(),
        max_tokens: 200,
        json: false,
    };
    let c = client.complete(&req).await.map_err(err)?;
    Ok(c.text.chars().take(80).collect())
}

// ---------- settings ----------

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CmdResult<AppSettings> {
    state.engine.db().call(|c| settings::app(c)).await.map_err(err)
}

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, new: AppSettings) -> CmdResult<AppSettings> {
    if !(1..=1000).contains(&new.ingest_count) {
        return Err("Matches per sync must be between 1 and 1000.".into());
    }
    if !(3..=1000).contains(&new.baseline_games) {
        return Err("Look-back window must be between 3 and 1000 games.".into());
    }
    if !(500..=32_000).contains(&new.ai.max_tokens) {
        return Err("Max AI output tokens must be between 500 and 32000.".into());
    }
    new.ai.provider.parse::<ProviderKind>()?;
    state
        .engine
        .db()
        .call(move |c| {
            settings::set(c, settings::APP, &new)?;
            settings::app(c)
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn get_ui_state(state: State<'_, AppState>) -> CmdResult<Option<serde_json::Value>> {
    state.engine.db().call(|c| settings::get(c, settings::UI_STATE)).await.map_err(err)
}

#[tauri::command]
pub async fn save_ui_state(state: State<'_, AppState>, ui: serde_json::Value) -> CmdResult<()> {
    if ui.to_string().len() > 64 * 1024 {
        return Err("UI state too large".into());
    }
    state.engine.db().call(move |c| settings::set(c, settings::UI_STATE, &ui)).await.map_err(err)
}

// ---------- Riot key ----------

fn apply_key(state: &AppState) -> KeyStatus {
    let found = load_riot_api_key();
    let source = found.as_ref().map(|(_, s)| *s);
    *state.key_source.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = source;
    state.engine.set_key(found.map(|(k, _)| k));
    state.engine.key_status()
}

/// Re-read the key (after the user changes the environment variable).
#[tauri::command]
pub async fn reload_key(state: State<'_, AppState>) -> CmdResult<KeyStatus> {
    Ok(apply_key(&state))
}

/// Save a Riot key to Credential Manager; it then takes precedence over
/// the environment variable.
#[tauri::command]
pub async fn set_riot_key(state: State<'_, AppState>, key: String) -> CmdResult<KeyStatus> {
    let key = key.trim();
    if !key.starts_with("RGAPI-") {
        return Err("Riot API keys start with RGAPI-".into());
    }
    credentials::store_secret(credentials::RIOT_API_KEY, &Secret::new(key)).map_err(err)?;
    tracing::info!("Riot key saved to Credential Manager");
    Ok(apply_key(&state))
}

#[tauri::command]
pub async fn delete_riot_key(state: State<'_, AppState>) -> CmdResult<KeyStatus> {
    credentials::delete_secret(credentials::RIOT_API_KEY).map_err(err)?;
    Ok(apply_key(&state))
}

// ---------- Data Dragon assets ----------

#[derive(Serialize)]
pub struct ChampionEntry {
    pub key: i32,
    pub id: String,
    pub name: String,
}

/// Champion ids → display names (newest patch).
#[tauri::command]
pub async fn champions(state: State<'_, AppState>) -> CmdResult<Vec<ChampionEntry>> {
    let dd = state.engine.ddragon();
    let version = dd.versions().await.map_err(err)?.first().cloned().ok_or("no Data Dragon versions")?;
    let idx = dd.champions(&version).await.map_err(err)?;
    let mut list: Vec<_> =
        idx.iter().map(|(key, c)| ChampionEntry { key, id: c.id.clone(), name: c.name.clone() }).collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

/// Champion square icon as a `data:` URL (downloaded once, then served from
/// the local cache).
#[tauri::command]
pub async fn champion_icon(state: State<'_, AppState>, champion_id: i32, patch: String) -> CmdResult<String> {
    let dd = state.engine.ddragon();
    let version = dd.version_for_patch(&patch).await.map_err(err)?;
    data_url(&dd.champion_icon(&version, champion_id).await.map_err(err)?, "image/png").await
}

#[tauri::command]
pub async fn item_icon(state: State<'_, AppState>, item_id: i32, patch: String) -> CmdResult<String> {
    let dd = state.engine.ddragon();
    let version = dd.version_for_patch(&patch).await.map_err(err)?;
    data_url(&dd.item_icon(&version, item_id).await.map_err(err)?, "image/png").await
}

#[tauri::command]
pub async fn spell_icon(state: State<'_, AppState>, spell_id: i32, patch: String) -> CmdResult<String> {
    let dd = state.engine.ddragon();
    let version = dd.version_for_patch(&patch).await.map_err(err)?;
    data_url(&dd.summoner_spell_icon(&version, spell_id).await.map_err(err)?, "image/png").await
}

/// Splash art by Data Dragon id (`Leblanc`) or numeric champion key.
#[tauri::command]
pub async fn champion_splash(state: State<'_, AppState>, champion: String) -> CmdResult<String> {
    let dd = state.engine.ddragon();
    let id = match champion.parse::<i32>() {
        Ok(key) => {
            let version = dd.versions().await.map_err(err)?.first().cloned().ok_or("no Data Dragon versions")?;
            dd.champions(&version).await.map_err(err)?.get(key).map(|c| c.id.clone()).ok_or("unknown champion")?
        }
        Err(_) => champion,
    };
    data_url(&dd.champion_splash(&id).await.map_err(err)?, "image/jpeg").await
}

#[tauri::command]
pub async fn minimap(state: State<'_, AppState>, patch: String) -> CmdResult<String> {
    let dd = state.engine.ddragon();
    let version = dd.version_for_patch(&patch).await.map_err(err)?;
    data_url(&dd.minimap(&version).await.map_err(err)?, "image/png").await
}
