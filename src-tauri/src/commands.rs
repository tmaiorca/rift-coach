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

pub type CmdResult<T> = Result<T, String>;

pub fn err(e: impl std::fmt::Display) -> String {
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

pub async fn active_account_id(state: &AppState) -> CmdResult<i64> {
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
            let s = settings::app(c)?;
            Ok(rc_coach::detail::match_detail(c, account_id, &match_id, s.baseline_games, s.session_gap_min))
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

// ---------- patterns, goals, sessions ----------

/// Recurring patterns and win correlation over the newest `window` games
/// (default: the look-back window), optionally one role.
#[tauri::command]
pub async fn patterns(
    state: State<'_, AppState>,
    window: Option<u32>,
    role: Option<String>,
) -> CmdResult<rc_coach::patterns::PatternsView> {
    let account_id = active_account_id(&state).await?;
    state
        .engine
        .db()
        .call(move |c| {
            let s = settings::app(c)?;
            let window = window.unwrap_or(s.baseline_games).clamp(5, 1000);
            rc_coach::patterns::patterns(c, account_id, window, role.as_deref(), s.session_gap_min)
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn list_goals(state: State<'_, AppState>, include_ended: bool) -> CmdResult<Vec<rc_coach::goals::GoalView>> {
    let Ok(account_id) = active_account_id(&state).await else { return Ok(vec![]) };
    state
        .engine
        .db()
        .call(move |c| Ok(rc_coach::goals::list(c, account_id, include_ended)))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn create_goal(
    state: State<'_, AppState>,
    input: rc_coach::goals::GoalInput,
) -> CmdResult<rc_coach::goals::GoalView> {
    let account_id = active_account_id(&state).await?;
    let view = state
        .engine
        .db()
        .call(move |c| {
            let games = settings::app(c)?.baseline_games;
            Ok(rc_coach::goals::create(c, account_id, &input, games))
        })
        .await
        .map_err(err)?
        .map_err(err)?;
    state.engine.log_activity("info", format!("New focus goal: {}", view.title)).await;
    Ok(view)
}

#[tauri::command]
pub async fn set_goal_status(state: State<'_, AppState>, goal_id: i64, status: String) -> CmdResult<()> {
    state
        .engine
        .db()
        .call(move |c| Ok(rc_coach::goals::set_status(c, goal_id, &status)))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn delete_goal(state: State<'_, AppState>, goal_id: i64) -> CmdResult<()> {
    state.engine.db().call(move |c| rc_store::goals::delete(c, goal_id)).await.map_err(err)
}

#[tauri::command]
pub async fn list_sessions(
    state: State<'_, AppState>,
    limit: u32,
) -> CmdResult<Vec<rc_coach::sessions::SessionSummary>> {
    let Ok(account_id) = active_account_id(&state).await else { return Ok(vec![]) };
    state
        .engine
        .db()
        .call(move |c| {
            let gap = settings::app(c)?.session_gap_min;
            rc_coach::sessions::list(c, account_id, gap, limit.min(500) as usize)
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn session_detail(state: State<'_, AppState>, scope: String) -> CmdResult<rc_coach::sessions::SessionDetail> {
    let account_id = active_account_id(&state).await?;
    state
        .engine
        .db()
        .call(move |c| {
            let s = settings::app(c)?;
            Ok(rc_coach::sessions::detail(c, account_id, &scope, s.session_gap_min, s.baseline_games))
        })
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn generate_session_review(state: State<'_, AppState>, scope: String, force: bool) -> CmdResult<Report> {
    let account_id = active_account_id(&state).await?;
    let settings = state.engine.db().call(|c| settings::app(c)).await.map_err(err)?;
    let client = rc_coach::provider::client_from_settings(&settings.ai).map_err(err)?;
    let opts = rc_coach::review::ReviewOptions::from_settings(&settings, force);
    let result = rc_coach::review::session_review(state.engine.db(), &client, account_id, &scope, &opts).await;
    match &result {
        Ok(r) => state.engine.log_activity("info", format!("AI session review ready ({})", r.model)).await,
        Err(e) => state.engine.log_activity("error", format!("AI session review failed: {e}")).await,
    }
    result.map_err(err)
}

#[tauri::command]
pub async fn session_facts(state: State<'_, AppState>, scope: String) -> CmdResult<serde_json::Value> {
    let account_id = active_account_id(&state).await?;
    let settings = state.engine.db().call(|c| settings::app(c)).await.map_err(err)?;
    let opts = rc_coach::review::ReviewOptions::from_settings(&settings, false);
    rc_coach::review::session_facts(state.engine.db(), account_id, &scope, &opts).await.map_err(err)
}

#[derive(Serialize)]
pub struct AiUsage {
    pub reports_30d: i64,
    pub tokens_30d: i64,
    pub limit: Option<u64>,
}

#[tauri::command]
pub async fn ai_usage(state: State<'_, AppState>) -> CmdResult<AiUsage> {
    let (reports_30d, tokens_30d) = rc_coach::review::usage(state.engine.db()).await.map_err(err)?;
    let limit = state.engine.db().call(|c| Ok(settings::app(c)?.ai.monthly_token_limit)).await.map_err(err)?;
    Ok(AiUsage { reports_30d, tokens_30d, limit })
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
    let opts = rc_coach::review::ReviewOptions::from_settings(&settings, force);
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
    let settings = state.engine.db().call(|c| settings::app(c)).await.map_err(err)?;
    let opts = rc_coach::review::ReviewOptions::from_settings(&settings, false);
    rc_coach::review::post_game_facts(state.engine.db(), account_id, &match_id, &opts, None).await.map_err(err)
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
    if !(10..=480).contains(&new.session_gap_min) {
        return Err("The session gap must be between 10 and 480 minutes.".into());
    }
    if new.ai.monthly_token_limit.is_some_and(|l| l < 10_000) {
        return Err("A token cap below 10,000 would block most reviews; use at least 10,000 or no cap.".into());
    }
    if new.live.cs_target_pm.is_some_and(|t| !(1.0..=15.0).contains(&t)) {
        return Err("The CS per minute target must be between 1 and 15.".into());
    }
    if !rc_store::settings::BENCHMARK_TIERS.contains(&new.benchmarks.tier.as_str()) {
        return Err("Benchmark tier must be Auto, Gold, Platinum, Emerald, Diamond or Master+.".into());
    }
    if !(2..=60).contains(&new.benchmarks.step_seconds) {
        return Err("Benchmark collection speed must be between 2 and 60 seconds per step.".into());
    }
    if !(200..=20_000).contains(&new.benchmarks.max_games) {
        return Err("Keep between 200 and 20,000 benchmark games.".into());
    }
    if new.live.keep_snapshots_days > 365 {
        return Err("Live snapshots can be kept for at most 365 days.".into());
    }
    new.ai.provider.parse::<ProviderKind>()?;
    if new.mcp.port < 1024 {
        return Err("Use an MCP port of 1024 or higher.".into());
    }
    let saved = state
        .engine
        .db()
        .call(move |c| {
            // A new tier starts its own sample; drop ids queued for the old one.
            let old = settings::app(c)?;
            if rc_coach::benchmarks::effective_tier(c, &old)? != rc_coach::benchmarks::effective_tier(c, &new)? {
                rc_store::bench::clear_pending(c)?;
            }
            let mcp_changed = old.mcp != new.mcp;
            settings::set(c, settings::APP, &new)?;
            Ok((settings::app(c)?, mcp_changed))
        })
        .await
        .map_err(err)?;
    let (saved, mcp_changed) = saved;
    crate::desktop::apply_settings(&state, &saved);
    if mcp_changed {
        crate::mcp_host::apply(&state).await;
    }
    Ok(saved)
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

// ---------- live ----------

#[tauri::command]
pub fn live_state(state: State<'_, AppState>) -> rc_jobs::LiveState {
    state.live.state.borrow().clone()
}

/// Recently recorded live games, newest first.
#[tauri::command]
pub async fn live_games(state: State<'_, AppState>, limit: u32) -> CmdResult<Vec<rc_jobs::LiveGameView>> {
    let rows = state.engine.db().call(move |c| rc_store::live::recent(c, limit.clamp(1, 200))).await.map_err(err)?;
    Ok(rows.into_iter().map(rc_jobs::live_game_view).collect())
}

/// Item data for the newest stored patch (what the live view uses).
async fn current_catalog(state: &AppState) -> CmdResult<(String, std::sync::Arc<rc_metrics::ItemCatalog>)> {
    let patch = state
        .engine
        .db()
        .call(|c| matches::latest_patch(c))
        .await
        .map_err(err)?
        .ok_or("No games stored yet, so there's no item data. Sync your games first.")?;
    let catalog = state.engine.item_catalog(&patch).await.map_err(err)?;
    Ok((patch, catalog))
}

#[derive(Serialize)]
pub struct PlannerView {
    pub patch: String,
    pub items: Vec<rc_coach::live::PlannerItem>,
}

#[tauri::command]
pub async fn planner_items(state: State<'_, AppState>) -> CmdResult<PlannerView> {
    let (patch, catalog) = current_catalog(&state).await?;
    Ok(PlannerView { items: rc_coach::live::planner_items(&catalog), patch })
}

#[derive(Serialize)]
pub struct BuildPlanView {
    pub champion: String,
    /// `plan` (saved), `usual` (your most common items), or `None` (nothing to go on).
    pub source: Option<&'static str>,
    pub items: Vec<rc_live::view::PlannedItem>,
    pub patch: String,
}

/// The planned build for a champion (Data Dragon id): saved, else your usual.
#[tauri::command]
pub async fn build_plan(state: State<'_, AppState>, champion: String) -> CmdResult<BuildPlanView> {
    let (patch, catalog) = current_catalog(&state).await?;
    let account_id = active_account_id(&state).await.ok();
    let (champ, cat) = (champion.clone(), catalog.clone());
    let (ids, source) = state
        .engine
        .db()
        .call(move |c| {
            if let Some(ids) = rc_store::live::build_plan(c, &champ)? {
                return Ok(Ok((ids, Some("plan"))));
            }
            let Some(a) = account_id else { return Ok(Ok((Vec::new(), None))) };
            Ok(rc_coach::live::usual_build(c, a, &champ, &cat).map(|ids| {
                let source = (!ids.is_empty()).then_some("usual");
                (ids, source)
            }))
        })
        .await
        .map_err(err)?
        .map_err(err)?;
    Ok(BuildPlanView { champion, source, items: rc_coach::live::planned(&ids, &catalog), patch })
}

/// Saves a planned build (empty = back to your usual build).
#[tauri::command]
pub async fn set_build_plan(state: State<'_, AppState>, champion: String, items: Vec<i32>) -> CmdResult<BuildPlanView> {
    if items.len() > 8 {
        return Err("A build plan holds at most 8 items.".into());
    }
    let (_, catalog) = current_catalog(&state).await?;
    if let Some(bad) = items.iter().find(|id| !catalog.get(**id).is_some_and(|i| i.plannable)) {
        return Err(format!("Item {bad} can't go in a build plan."));
    }
    let champ = champion.clone();
    state.engine.db().call(move |c| rc_store::live::set_build_plan(c, &champ, &items)).await.map_err(err)?;
    build_plan(state, champion).await
}

/// Opens (or brings back) the compact live window for a second monitor.
#[tauri::command]
pub async fn open_live_window(app: tauri::AppHandle) -> CmdResult<()> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("live") {
        w.unminimize().map_err(err)?;
        return w.set_focus().map_err(err);
    }
    tauri::WebviewWindowBuilder::new(&app, "live", tauri::WebviewUrl::App("index.html".into()))
        .title("Rift Coach · Live")
        .inner_size(380.0, 660.0)
        .min_inner_size(320.0, 420.0)
        .build()
        .map_err(err)?;
    Ok(())
}

// ---------- benchmarks ----------

#[derive(Serialize)]
pub struct BenchmarkStatus {
    pub progress: rc_coach::benchmarks::Progress,
    pub collector: rc_jobs::BenchState,
}

#[tauri::command]
pub async fn benchmark_status(state: State<'_, AppState>) -> CmdResult<BenchmarkStatus> {
    let account_id = active_account_id(&state).await.ok();
    let progress = state
        .engine
        .db()
        .call(move |c| rc_coach::benchmarks::progress(c, account_id, &settings::app(c)?))
        .await
        .map_err(err)?;
    Ok(BenchmarkStatus { progress, collector: state.bench.borrow().clone() })
}

/// Your recent games (look-back window) in a role, optionally on one
/// champion, against the benchmark tier. Defaults to your main role.
#[tauri::command]
pub async fn benchmark_gaps(
    state: State<'_, AppState>,
    role: Option<String>,
    champion: Option<String>,
) -> CmdResult<Option<rc_coach::benchmarks::GapsView>> {
    let account_id = active_account_id(&state).await?;
    state
        .engine
        .db()
        .call(move |c| {
            let s = settings::app(c)?;
            let role = match role {
                Some(r) => r,
                None => match rc_coach::benchmarks::focus(c, Some(account_id), &s)?.roles.into_iter().next() {
                    Some(r) => r,
                    None => return Ok(None),
                },
            };
            let filter = rc_store::metrics::GameFilter { limit: s.baseline_games, ..Default::default() };
            Ok(Some(rc_coach::benchmarks::gaps(c, account_id, &s, &role, champion.as_deref(), filter)?))
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn ranks(state: State<'_, AppState>) -> CmdResult<Vec<rc_store::bench::Rank>> {
    let Ok(account_id) = active_account_id(&state).await else { return Ok(vec![]) };
    state.engine.db().call(move |c| rc_store::bench::ranks(c, account_id)).await.map_err(err)
}

// ---------- champions ----------

#[tauri::command]
pub async fn champion_list(state: State<'_, AppState>) -> CmdResult<Vec<rc_coach::champions::ChampionSummary>> {
    let Ok(account_id) = active_account_id(&state).await else { return Ok(vec![]) };
    state.engine.db().call(move |c| rc_coach::champions::list(c, account_id, &settings::app(c)?)).await.map_err(err)
}

#[tauri::command]
pub async fn champion_detail(
    state: State<'_, AppState>,
    champion: String,
) -> CmdResult<rc_coach::champions::ChampionDetail> {
    let account_id = active_account_id(&state).await?;
    state
        .engine
        .db()
        .call(move |c| {
            let s = settings::app(c)?;
            Ok(rc_coach::champions::detail(c, account_id, &champion, &s))
        })
        .await
        .map_err(err)?
        .map_err(err)
}

// ---------- weekly ----------

#[tauri::command]
pub async fn list_weeks(state: State<'_, AppState>, limit: u32) -> CmdResult<Vec<rc_coach::weekly::WeekSummary>> {
    let Ok(account_id) = active_account_id(&state).await else { return Ok(vec![]) };
    state
        .engine
        .db()
        .call(move |c| rc_coach::weekly::list(c, account_id, limit.min(200) as usize, rc_common::time::now_ms()))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn week_detail(state: State<'_, AppState>, week: String) -> CmdResult<rc_coach::weekly::WeekDetail> {
    let account_id = active_account_id(&state).await?;
    state
        .engine
        .db()
        .call(move |c| {
            let s = settings::app(c)?;
            Ok(rc_coach::weekly::detail(c, account_id, &week, &s, rc_common::time::now_ms()))
        })
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub async fn generate_weekly_report(state: State<'_, AppState>, week: String, force: bool) -> CmdResult<Report> {
    let account_id = active_account_id(&state).await?;
    let settings = state.engine.db().call(|c| settings::app(c)).await.map_err(err)?;
    let client = rc_coach::provider::client_from_settings(&settings.ai).map_err(err)?;
    let opts = rc_coach::review::ReviewOptions::from_settings(&settings, force);
    let result = rc_coach::review::weekly_report(state.engine.db(), &client, account_id, &week, &opts).await;
    match &result {
        Ok(r) => state.engine.log_activity("info", format!("AI weekly report ready ({})", r.model)).await,
        Err(e) => state.engine.log_activity("error", format!("AI weekly report failed: {e}")).await,
    }
    result.map_err(err)
}

#[tauri::command]
pub async fn weekly_facts(state: State<'_, AppState>, week: String) -> CmdResult<serde_json::Value> {
    let account_id = active_account_id(&state).await?;
    rc_coach::review::weekly_facts(state.engine.db(), account_id, &week).await.map_err(err)
}

/// Asks for the AI game plan now. Only honored on the loading screen.
#[tauri::command]
pub async fn generate_game_plan(state: State<'_, AppState>) -> CmdResult<()> {
    if state.live.state.borrow().phase != rc_live::Phase::Loading {
        return Err("Game plans can only be written on the loading screen.".into());
    }
    state.live.commands.send(rc_jobs::LiveCommand::GeneratePlan).await.map_err(err)
}
