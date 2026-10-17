//! Post-game AI review: facts → provider → validation → stored report.

use std::sync::Arc;

use rc_ai::{AiClient, CompletionRequest, extract_json};
use rc_metrics::detail::deaths_of;
use rc_metrics::model::ItemCatalog;
use rc_store::Db;
use rc_store::reports::{self, NewReport, Report};
use serde_json::Value;

use crate::CoachError;
use crate::analysis::compare_metrics;
use crate::detail::{POST_GAME, load};
use crate::validate::validate;

pub const PROMPT_VERSION: &str = "post_game_review.v1";
const PROMPT: &str = include_str!("../../../prompts/post_game_review.v1.md");

#[derive(Debug, Clone)]
pub struct ReviewOptions {
    pub baseline_games: u32,
    pub max_tokens: u32,
    /// Generate a new report even if one exists for this prompt/provider/model.
    pub force: bool,
}

/// Builds the facts object for a match (also used by the "show facts" view).
pub async fn post_game_facts(
    db: &Db,
    account_id: i64,
    match_id: &str,
    baseline_games: u32,
    items: Option<Arc<ItemCatalog>>,
) -> Result<Value, CoachError> {
    let match_id = match_id.to_owned();
    db.call(move |c| Ok(build_facts(c, account_id, &match_id, baseline_games, items.as_deref()))).await?
}

fn build_facts(
    c: &rc_store::rusqlite::Connection,
    account_id: i64,
    match_id: &str,
    baseline_games: u32,
    items: Option<&ItemCatalog>,
) -> Result<Value, CoachError> {
    let l = load(c, account_id, match_id)?;
    if l.summary.is_remake {
        return Err(CoachError::Invalid("remakes can't be reviewed".into()));
    }
    let me = l.subject()?;
    let rift = rc_common::queues::is_standard_rift(l.summary.queue_id, l.summary.map_id);
    let cmp = compare_metrics(
        c,
        account_id,
        match_id,
        l.participant_id,
        if rift { me.role() } else { None },
        &me.champion_name,
        l.summary.game_start_ms,
        baseline_games,
    )?;
    let deaths = l.timeline.as_ref().map(|tl| deaths_of(&l.game, tl, me)).unwrap_or_default();
    Ok(crate::facts::build(&l, &cmp, &deaths, items))
}

pub async fn post_game_review(
    db: &Db,
    ai: &AiClient,
    account_id: i64,
    match_id: &str,
    opts: &ReviewOptions,
    items: Option<Arc<ItemCatalog>>,
) -> Result<Report, CoachError> {
    let provider = ai.config().kind.as_str();
    let model = ai.config().model.clone();
    if !opts.force {
        let id = match_id.to_owned();
        let cached = db.call(move |c| reports::latest_for_match(c, POST_GAME, account_id, &id)).await?;
        if let Some(r) =
            cached.filter(|r| r.prompt_version == PROMPT_VERSION && r.provider == provider && r.model == model)
        {
            return Ok(r);
        }
    }

    let facts = post_game_facts(db, account_id, match_id, opts.baseline_games, items).await?;
    let facts_text = serde_json::to_string_pretty(&facts).map_err(|e| CoachError::Invalid(e.to_string()))?;
    let user = format!("Facts for the game to review:\n{facts_text}");
    let mut request = CompletionRequest { system: PROMPT.to_owned(), user, max_tokens: opts.max_tokens, json: true };

    let (mut tokens_in, mut tokens_out) = (0i64, 0i64);
    let mut last: Option<(Value, crate::validate::Validation)> = None;
    for attempt in 1..=2 {
        let completion = ai.complete(&request).await?;
        tokens_in += completion.input_tokens.unwrap_or(0);
        tokens_out += completion.output_tokens.unwrap_or(0);
        let parsed = extract_json(&completion.text).unwrap_or(Value::Null);
        let v = validate(&parsed, &facts);
        tracing::info!(
            match_id,
            attempt,
            structural = v.structural.len(),
            untraced = v.untraced_numbers.len(),
            "AI review validated"
        );
        let clean = v.is_clean();
        if !clean {
            request.user = format!(
                "{}\n\nYour previous reply had problems:\n{}\nReply again with the corrected JSON object only.",
                request.user,
                v.feedback()
            );
        }
        last = Some((parsed, v));
        if clean {
            break;
        }
    }
    let Some((report, v)) = last else { return Err(CoachError::Invalid("no AI reply".into())) };
    if !v.structural.is_empty() {
        return Err(CoachError::Invalid(format!(
            "the AI reply didn't match the report format after 2 tries: {}",
            v.structural.join(" ")
        )));
    }
    let (validation, notes) = if v.untraced_numbers.is_empty() {
        ("ok", None)
    } else {
        ("unverified", Some(format!("Numbers not found in the game data: {}", v.untraced_numbers.join(", "))))
    };

    let input_json = facts_text;
    let output_json = report.to_string();
    let id = match_id.to_owned();
    let model2 = model.clone();
    let report_id = db
        .call(move |c| {
            reports::insert(
                c,
                &NewReport {
                    kind: POST_GAME,
                    account_id,
                    match_id: Some(&id),
                    provider,
                    model: &model2,
                    prompt_version: PROMPT_VERSION,
                    input_json: &input_json,
                    output_json: &output_json,
                    validation,
                    notes: notes.as_deref(),
                    input_tokens: Some(tokens_in),
                    output_tokens: Some(tokens_out),
                },
            )
        })
        .await?;
    db.call(move |c| reports::get(c, report_id))
        .await?
        .ok_or_else(|| CoachError::NotFound("the report just saved".into()))
}
