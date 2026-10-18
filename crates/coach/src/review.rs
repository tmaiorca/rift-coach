//! AI reports: facts → provider → validation → stored report. Post-game
//! reviews cover one match; session reviews cover a play session.

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
use crate::sessions::SESSION;
use crate::validate::{validate, validate_plan};

pub const PROMPT_VERSION: &str = "post_game_review.v3";
const PROMPT: &str = include_str!("../../../prompts/post_game_review.v3.md");
pub const SESSION_PROMPT_VERSION: &str = "session_review.v1";
const SESSION_PROMPT: &str = include_str!("../../../prompts/session_review.v1.md");
pub const PREGAME_PROMPT_VERSION: &str = "pregame_brief.v1";
const PREGAME_PROMPT: &str = include_str!("../../../prompts/pregame_brief.v1.md");
pub const WEEKLY_PROMPT_VERSION: &str = "weekly_report.v1";
const WEEKLY_PROMPT: &str = include_str!("../../../prompts/weekly_report.v1.md");

/// The token cap counts reports from this far back.
pub const USAGE_WINDOW_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Debug, Clone)]
pub struct ReviewOptions {
    pub baseline_games: u32,
    pub session_gap_min: u32,
    pub max_tokens: u32,
    /// Refuse to call the AI once the last 30 days used this many tokens.
    pub monthly_token_limit: Option<u64>,
    /// Generate a new report even if one exists for this prompt/provider/model.
    pub force: bool,
}

impl ReviewOptions {
    pub fn from_settings(s: &rc_store::settings::AppSettings, force: bool) -> Self {
        Self {
            baseline_games: s.baseline_games,
            session_gap_min: s.session_gap_min,
            max_tokens: s.ai.max_tokens,
            monthly_token_limit: s.ai.monthly_token_limit,
            force,
        }
    }
}

/// Builds the facts object for a match (also used by the "show facts" view).
pub async fn post_game_facts(
    db: &Db,
    account_id: i64,
    match_id: &str,
    opts: &ReviewOptions,
    items: Option<Arc<ItemCatalog>>,
) -> Result<Value, CoachError> {
    let match_id = match_id.to_owned();
    let (baseline, gap) = (opts.baseline_games, opts.session_gap_min);
    db.call(move |c| Ok(build_facts(c, account_id, &match_id, baseline, gap, items.as_deref()))).await?
}

fn build_facts(
    c: &rc_store::rusqlite::Connection,
    account_id: i64,
    match_id: &str,
    baseline_games: u32,
    session_gap_min: u32,
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
    let patterns = if rift {
        crate::patterns::match_patterns(
            c,
            account_id,
            match_id,
            l.summary.game_start_ms,
            baseline_games,
            session_gap_min,
        )?
    } else {
        vec![]
    };
    let goals = crate::goals::list(c, account_id, false)?;
    Ok(crate::facts::build(&l, &cmp, &deaths, items, &patterns, &goals))
}

/// Facts for a session review.
pub async fn session_facts(db: &Db, account_id: i64, scope: &str, opts: &ReviewOptions) -> Result<Value, CoachError> {
    let scope = scope.to_owned();
    let (baseline, gap) = (opts.baseline_games, opts.session_gap_min);
    db.call(move |c| {
        Ok((|| {
            let d = crate::sessions::detail(c, account_id, &scope, gap, baseline)?;
            if d.summary.rift_games == 0 {
                return Err(CoachError::Invalid(
                    "This session has no Summoner's Rift games to review (ARAM, Arena and remakes aren't analysed)."
                        .into(),
                ));
            }
            let goals = crate::goals::list(c, account_id, false)?;
            Ok(crate::facts::session(&d, &goals))
        })())
    })
    .await?
}

pub async fn post_game_review(
    db: &Db,
    ai: &AiClient,
    account_id: i64,
    match_id: &str,
    opts: &ReviewOptions,
    items: Option<Arc<ItemCatalog>>,
) -> Result<Report, CoachError> {
    if !opts.force {
        let id = match_id.to_owned();
        let cached = db.call(move |c| reports::latest_for_match(c, POST_GAME, account_id, &id)).await?;
        if let Some(r) = cached.filter(|r| is_current(r, ai, PROMPT_VERSION)) {
            return Ok(r);
        }
    }
    let facts = post_game_facts(db, account_id, match_id, opts, items).await?;
    generate(
        db,
        ai,
        Spec {
            kind: POST_GAME,
            account_id,
            match_id: Some(match_id.to_owned()),
            scope: None,
            prompt: PROMPT,
            prompt_version: PROMPT_VERSION,
            intro: "Facts for the game to review:",
            facts,
            plan: false,
        },
        opts,
    )
    .await
}

pub async fn session_review(
    db: &Db,
    ai: &AiClient,
    account_id: i64,
    scope: &str,
    opts: &ReviewOptions,
) -> Result<Report, CoachError> {
    if !opts.force {
        let s = scope.to_owned();
        let cached = db.call(move |c| reports::latest_for_scope(c, SESSION, account_id, &s)).await?;
        if let Some(r) = cached.filter(|r| is_current(r, ai, SESSION_PROMPT_VERSION)) {
            return Ok(r);
        }
    }
    let facts = session_facts(db, account_id, scope, opts).await?;
    generate(
        db,
        ai,
        Spec {
            kind: SESSION,
            account_id,
            match_id: None,
            scope: Some(scope.to_owned()),
            prompt: SESSION_PROMPT,
            prompt_version: SESSION_PROMPT_VERSION,
            intro: "Facts for the session to review:",
            facts,
            plan: false,
        },
        opts,
    )
    .await
}

/// Facts for a weekly report.
pub async fn weekly_facts(db: &Db, account_id: i64, week: &str) -> Result<Value, CoachError> {
    let week = week.to_owned();
    db.call(move |c| {
        Ok((|| {
            let settings = rc_store::settings::app(c)?;
            let d = crate::weekly::detail(c, account_id, &week, &settings, rc_common::time::now_ms())?;
            if d.summary.rift_games == 0 {
                return Err(CoachError::Invalid("This week has no Summoner's Rift games to report on.".into()));
            }
            let goals = crate::goals::list(c, account_id, false)?;
            Ok(crate::facts::weekly(&d, &goals))
        })())
    })
    .await?
}

pub async fn weekly_report(
    db: &Db,
    ai: &AiClient,
    account_id: i64,
    week: &str,
    opts: &ReviewOptions,
) -> Result<Report, CoachError> {
    let scope = crate::weekly::scope_of(week);
    if !opts.force {
        let s = scope.clone();
        let cached = db.call(move |c| reports::latest_for_scope(c, crate::weekly::WEEKLY, account_id, &s)).await?;
        if let Some(r) = cached.filter(|r| is_current(r, ai, WEEKLY_PROMPT_VERSION)) {
            return Ok(r);
        }
    }
    let facts = weekly_facts(db, account_id, week).await?;
    generate(
        db,
        ai,
        Spec {
            kind: crate::weekly::WEEKLY,
            account_id,
            match_id: None,
            scope: Some(scope),
            prompt: WEEKLY_PROMPT,
            prompt_version: WEEKLY_PROMPT_VERSION,
            intro: "Facts for the week to report on:",
            facts,
            plan: false,
        },
        opts,
    )
    .await
}

/// The AI game plan for a pre-game brief (one per game; reused if it exists).
pub async fn pregame_plan(
    db: &Db,
    ai: &AiClient,
    account_id: i64,
    brief: &crate::pregame::Brief,
    opts: &ReviewOptions,
) -> Result<Report, CoachError> {
    let scope = crate::pregame::scope_of(brief.game_id);
    let s = scope.clone();
    let cached = db.call(move |c| reports::latest_for_scope(c, crate::pregame::PREGAME, account_id, &s)).await?;
    if let Some(r) = cached.filter(|r| is_current(r, ai, PREGAME_PROMPT_VERSION)) {
        return Ok(r);
    }
    generate(
        db,
        ai,
        Spec {
            kind: crate::pregame::PREGAME,
            account_id,
            match_id: None,
            scope: Some(scope),
            prompt: PREGAME_PROMPT,
            prompt_version: PREGAME_PROMPT_VERSION,
            intro: "Facts for the game that is loading:",
            facts: crate::pregame::facts(brief),
            plan: true,
        },
        opts,
    )
    .await
}

fn is_current(r: &Report, ai: &AiClient, prompt_version: &str) -> bool {
    r.prompt_version == prompt_version && r.provider == ai.config().kind.as_str() && r.model == ai.config().model
}

/// Tokens used by all reports in the last 30 days, and how many reports.
pub async fn usage(db: &Db) -> Result<(i64, i64), CoachError> {
    let since = rc_common::time::now_ms() - USAGE_WINDOW_MS;
    Ok(db.call(move |c| reports::usage_since(c, since)).await?)
}

async fn check_budget(db: &Db, limit: Option<u64>) -> Result<(), CoachError> {
    let Some(limit) = limit else { return Ok(()) };
    let (_, used) = usage(db).await?;
    if u64::try_from(used).unwrap_or(0) >= limit {
        return Err(CoachError::Budget(format!(
            "The AI token cap is reached ({used} of {limit} tokens used in the last 30 days). Raise or remove it in \
             Settings → AI coach."
        )));
    }
    Ok(())
}

struct Spec {
    kind: &'static str,
    account_id: i64,
    match_id: Option<String>,
    scope: Option<String>,
    prompt: &'static str,
    prompt_version: &'static str,
    intro: &'static str,
    facts: Value,
    /// A pre-game plan (its own format) rather than a review.
    plan: bool,
}

async fn generate(db: &Db, ai: &AiClient, spec: Spec, opts: &ReviewOptions) -> Result<Report, CoachError> {
    check_budget(db, opts.monthly_token_limit).await?;
    let facts_text = serde_json::to_string_pretty(&spec.facts).map_err(|e| CoachError::Invalid(e.to_string()))?;
    let user = format!("{}\n{facts_text}", spec.intro);
    let mut request =
        CompletionRequest { system: spec.prompt.to_owned(), user, max_tokens: opts.max_tokens, json: true };

    let (mut tokens_in, mut tokens_out) = (0i64, 0i64);
    let mut last: Option<(Value, crate::validate::Validation)> = None;
    for attempt in 1..=2 {
        let completion = ai.complete(&request).await?;
        tokens_in += completion.input_tokens.unwrap_or(0);
        tokens_out += completion.output_tokens.unwrap_or(0);
        let parsed = extract_json(&completion.text).unwrap_or(Value::Null);
        let v = if spec.plan { validate_plan(&parsed, &spec.facts) } else { validate(&parsed, &spec.facts) };
        tracing::info!(
            kind = spec.kind,
            attempt,
            structural = v.structural.len(),
            untraced = v.untraced_numbers.len(),
            "AI report validated"
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

    let provider = ai.config().kind.as_str();
    let model = ai.config().model.clone();
    let output_json = report.to_string();
    let Spec { kind, account_id, match_id, scope, prompt_version, .. } = spec;
    let report_id = db
        .call(move |c| {
            reports::insert(
                c,
                &NewReport {
                    kind,
                    account_id,
                    match_id: match_id.as_deref(),
                    scope: scope.as_deref(),
                    provider,
                    model: &model,
                    prompt_version,
                    input_json: &facts_text,
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
