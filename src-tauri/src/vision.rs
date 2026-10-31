//! Vision review commands: a match's capture/review status, an on-demand
//! review, and the frames on disk.

use rc_store::reports::Report;
use rc_store::settings;
use rc_vision::{CaptureStats, DiskUsage};
use serde::Serialize;
use tauri::State;

use crate::AppState;
use crate::commands::{CmdResult, active_account_id, err};

#[derive(Serialize)]
pub struct VisionView {
    pub enabled: bool,
    /// The live game this match was recorded as (capture needs one).
    pub live_game_id: Option<i64>,
    /// `captured` | `reviewed` | `failed` | `deleted`; `None` = no frames taken.
    pub state: Option<String>,
    pub frames_on_disk: usize,
    pub stats: Option<CaptureStats>,
    pub report: Option<Report>,
    /// An automatic review is queued or running.
    pub queued: bool,
}

#[tauri::command]
pub async fn vision_status(state: State<'_, AppState>, match_id: String) -> CmdResult<VisionView> {
    let account_id = active_account_id(&state).await?;
    let (enabled, live, report, queued) = state
        .engine
        .db()
        .call(move |c| {
            let enabled = settings::app(c)?.vision.enabled;
            let live = rc_store::live::by_match(c, &match_id)?;
            let report = rc_store::reports::latest_for_match(c, rc_coach::vision::VISION, account_id, &match_id)?;
            let queued = match &live {
                Some(g) => rc_store::jobs::is_active(c, &format!("vision:{}", g.id))?,
                None => false,
            };
            Ok((enabled, live, report, queued))
        })
        .await
        .map_err(err)?;
    let frames = match (&live, state.engine.frame_store()) {
        (Some(g), Some(store)) => store.list(g.id).len(),
        _ => 0,
    };
    Ok(VisionView {
        enabled,
        live_game_id: live.as_ref().map(|g| g.id),
        state: live.as_ref().and_then(|g| g.vision_state.clone()),
        frames_on_disk: frames,
        stats: live.as_ref().and_then(|g| g.vision_json.as_deref()).and_then(|j| serde_json::from_str(j).ok()),
        report,
        queued,
    })
}

/// Reviews a match's frames now (they are deleted once the review is saved).
#[tauri::command]
pub async fn generate_vision_review(state: State<'_, AppState>, match_id: String) -> CmdResult<Report> {
    if state.engine.in_game() {
        return Err("Vision reviews run after your game, not during one.".into());
    }
    let account_id = active_account_id(&state).await?;
    let store = state.engine.frame_store().cloned().ok_or("no frames folder")?;
    let id = match_id.clone();
    let (s, live) = state
        .engine
        .db()
        .call(move |c| Ok((settings::app(c)?, rc_store::live::by_match(c, &id)?)))
        .await
        .map_err(err)?;
    let live = live.ok_or("This game wasn't recorded live, so it has no screenshots.")?;
    let client = rc_coach::provider::vision_client(&s.vision).map_err(err)?;
    let opts = rc_coach::vision::VisionOptions::from_settings(&s);
    let result =
        rc_coach::vision::vision_review(state.engine.db(), &client, &store, account_id, &match_id, live.id, &opts)
            .await;
    match &result {
        Ok(r) => {
            let (live_id, report_id) = (live.id, r.id);
            state
                .engine
                .db()
                .call(move |c| rc_store::live::set_vision_report(c, live_id, report_id))
                .await
                .map_err(err)?;
            if let Err(e) = store.delete(live.id) {
                tracing::warn!(error = %e, "could not delete reviewed frames");
            }
            state.engine.log_activity("info", format!("Vision review ready for {match_id} ({})", r.model)).await;
        }
        Err(e) => state.engine.log_activity("error", format!("Vision review failed for {match_id}: {e}")).await,
    }
    result.map_err(err)
}

#[tauri::command]
pub async fn vision_disk_usage(state: State<'_, AppState>) -> CmdResult<DiskUsage> {
    let store = state.engine.frame_store().cloned().ok_or("no frames folder")?;
    tokio::task::spawn_blocking(move || store.usage()).await.map_err(err)
}

/// Deletes every stored frame (games waiting for a review lose theirs).
#[tauri::command]
pub async fn delete_vision_frames(state: State<'_, AppState>) -> CmdResult<usize> {
    if state.engine.in_game() {
        return Err("Wait until your game ends: its frames are being written.".into());
    }
    let store = state.engine.frame_store().cloned().ok_or("no frames folder")?;
    let deleted = tokio::task::spawn_blocking(move || store.sweep(|_| false)).await.map_err(err)?;
    let ids = deleted.clone();
    state
        .engine
        .db()
        .call(move |c| {
            for id in ids {
                if rc_store::live::get(c, id)?.is_some_and(|g| g.vision_report_id.is_none()) {
                    rc_store::live::set_vision_state(c, id, "deleted")?;
                }
            }
            Ok(())
        })
        .await
        .map_err(err)?;
    Ok(deleted.len())
}
