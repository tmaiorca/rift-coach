//! The read-only tools. Each returns the same views the app shows, limited
//! to the tracked player's own data: no other players' Riot IDs, no keys.

use rc_coach::review::ReviewOptions;
use rc_store::Db;
use rc_store::accounts::{self, Account};
use rc_store::rusqlite::Connection;
use rc_store::settings::{self, AppSettings};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Bad arguments or nothing found: shown to the agent as a tool error.
    #[error("{0}")]
    Invalid(String),
    #[error("unknown tool: {0}")]
    Unknown(String),
    #[error("{0}")]
    Internal(String),
}

impl From<rc_store::StoreError> for ToolError {
    fn from(e: rc_store::StoreError) -> Self {
        Self::Internal(e.to_string())
    }
}

impl From<rc_coach::CoachError> for ToolError {
    fn from(e: rc_coach::CoachError) -> Self {
        match e {
            rc_coach::CoachError::NotFound(_) | rc_coach::CoachError::Invalid(_) => Self::Invalid(e.to_string()),
            other => Self::Internal(other.to_string()),
        }
    }
}

const RIOT_ID_ARG: &str = "Riot ID (Name#TAG) of a tracked account. Default: the account the app is showing.";

/// Tool definitions for `tools/list`.
pub fn definitions() -> Value {
    let account = json!({"type": "string", "description": RIOT_ID_ARG});
    let tool = |name: &str, title: &str, description: &str, props: Value, required: &[&str]| {
        json!({
            "name": name,
            "title": title,
            "description": description,
            "inputSchema": {"type": "object", "properties": props, "required": required, "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        })
    };
    json!([
        tool(
            "list_accounts",
            "Tracked accounts",
            "Riot accounts tracked in Rift Coach, with region, ranked tiers, and how many games are stored.",
            json!({}),
            &[]
        ),
        tool(
            "list_matches",
            "Recent matches",
            "The player's recent games, newest first: champion, role, result, K/D/A, queue, duration, and whether an AI review exists.",
            json!({
                "riot_id": account,
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Default 20."},
                "champion": {"type": "string", "description": "Data Dragon champion id, e.g. Ahri."},
                "role": {"type": "string", "enum": ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"]},
            }),
            &[]
        ),
        tool(
            "get_match",
            "Match analysis",
            "One game's analysis: the player's metrics with percentiles (vs their lobbies, their own recent games, and higher-rank benchmarks), patterns flagged in the game, timeline moments, and active goals. Same facts the AI reviews use.",
            json!({"match_id": {"type": "string", "description": "e.g. NA1_5123456789"}, "riot_id": account}),
            &["match_id"]
        ),
        tool(
            "get_patterns",
            "Recurring patterns",
            "Recurring strengths and weaknesses across recent games, with the evidence games, plus which metrics actually move the player's win rate.",
            json!({
                "riot_id": account,
                "games": {"type": "integer", "minimum": 5, "maximum": 500, "description": "How many recent games to look at. Default: the app's look-back window."},
                "role": {"type": "string", "enum": ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"]},
            }),
            &[]
        ),
        tool(
            "get_goals",
            "Focus goals",
            "The player's focus goals and progress game over game.",
            json!({"riot_id": account, "include_ended": {"type": "boolean"}}),
            &[]
        ),
        tool(
            "list_reports",
            "AI reports",
            "AI coaching reports (post-game, session, weekly, game plan), newest first, without their full text.",
            json!({
                "riot_id": account,
                "kind": {"type": "string", "enum": ["post_game", "session", "weekly", "pregame"]},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Default 20."},
            }),
            &[]
        ),
        tool(
            "get_report",
            "AI report",
            "One AI coaching report in full: prioritized fixes with evidence, focus, and confidence.",
            json!({"report_id": {"type": "integer"}}),
            &["report_id"]
        ),
        tool(
            "get_week",
            "Weekly summary",
            "A Monday-to-Sunday week: results, averages vs the 4 weeks before, champions, patterns, an 8-week trend of key numbers, and the gap to higher ranks.",
            json!({"riot_id": account, "week": {"type": "string", "description": "Monday of the week, YYYY-MM-DD. Default: the current week."}}),
            &[]
        ),
        tool(
            "get_benchmarks",
            "Higher-rank benchmarks",
            "How the player's typical numbers rank among higher-rank players in a role (optionally on one champion).",
            json!({
                "riot_id": account,
                "role": {"type": "string", "enum": ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"], "description": "Default: main role."},
                "champion": {"type": "string", "description": "Data Dragon champion id."},
            }),
            &[]
        ),
        tool(
            "get_champions",
            "Champions",
            "Without `champion`: every champion played recently with record and averages. With it: that champion's numbers vs the player's other champions in the role, lane matchups, patterns, and benchmark gaps.",
            json!({"riot_id": account, "champion": {"type": "string", "description": "Data Dragon champion id."}}),
            &[]
        ),
        tool(
            "list_sessions",
            "Play sessions",
            "Recent play sessions (games played close together) with results.",
            json!({"riot_id": account, "limit": {"type": "integer", "minimum": 1, "maximum": 100}}),
            &[]
        ),
    ])
}

fn str_arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn int_arg(args: &Value, k: &str, default: i64, min: i64, max: i64) -> i64 {
    args.get(k).and_then(Value::as_i64).unwrap_or(default).clamp(min, max)
}

/// The account a call is about.
fn account(conn: &Connection, args: &Value) -> Result<Account, ToolError> {
    let all = accounts::list_accounts(conn)?;
    if let Some(id) = str_arg(args, "riot_id") {
        let (name, tag) =
            id.split_once('#').ok_or_else(|| ToolError::Invalid("riot_id must look like Name#TAG".into()))?;
        return all
            .into_iter()
            .find(|a| a.game_name.eq_ignore_ascii_case(name) && a.tag_line.eq_ignore_ascii_case(tag))
            .ok_or_else(|| ToolError::Invalid(format!("{id} isn't tracked in Rift Coach")));
    }
    let s = settings::app(conn)?;
    let active = s.active_account_id.and_then(|id| all.iter().find(|a| a.id == id).cloned());
    active
        .or_else(|| all.iter().find(|a| a.is_primary).cloned())
        .or_else(|| all.into_iter().next())
        .ok_or_else(|| ToolError::Invalid("No account is tracked yet.".into()))
}

/// Local time with its UTC offset.
fn iso8601(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
        .unwrap_or_default()
}

fn to_value(v: impl serde::Serialize) -> Result<Value, ToolError> {
    serde_json::to_value(v).map_err(|e| ToolError::Internal(e.to_string()))
}

/// Signals shown to agents: the strongest few, with a few evidence games
/// each (the full lists would crowd an agent's context).
const MAX_SIGNALS: usize = 12;
const MAX_EVIDENCE: usize = 5;

fn signals(list: &[rc_signals::Signal]) -> Result<Value, ToolError> {
    let mut v = to_value(list.iter().take(MAX_SIGNALS).collect::<Vec<_>>())?;
    if let Some(arr) = v.as_array_mut() {
        for s in arr {
            if let Some(ev) = s.get_mut("evidence").and_then(Value::as_array_mut) {
                ev.truncate(MAX_EVIDENCE);
            }
        }
    }
    Ok(v)
}

const ROLES: [&str; 5] = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"];

fn role_arg(args: &Value) -> Result<Option<String>, ToolError> {
    match str_arg(args, "role") {
        Some(r) => {
            let r = r.to_ascii_uppercase();
            if ROLES.contains(&r.as_str()) { Ok(Some(r)) } else { Err(ToolError::Invalid(format!("unknown role {r}"))) }
        }
        None => Ok(None),
    }
}

/// Runs one tool. Everything but `get_match` is a synchronous read.
pub async fn call(db: &Db, name: &str, args: Value) -> Result<Value, ToolError> {
    if name == "get_match" {
        let match_id =
            str_arg(&args, "match_id").ok_or_else(|| ToolError::Invalid("match_id is required".into()))?.to_owned();
        let a = args.clone();
        let (account, s) = db.call(move |c| Ok(account(c, &a).map(|acc| (acc, settings::app(c))))).await??;
        let opts = ReviewOptions::from_settings(&s?, false);
        return Ok(rc_coach::review::post_game_facts(db, account.id, &match_id, &opts, None).await?);
    }
    let name = name.to_owned();
    db.call(move |c| Ok(sync_call(c, &name, &args))).await?
}

fn sync_call(c: &Connection, name: &str, args: &Value) -> Result<Value, ToolError> {
    let s: AppSettings = settings::app(c)?;
    match name {
        "list_accounts" => {
            let mut out = Vec::new();
            for a in accounts::list_accounts(c)? {
                let counts = rc_store::matches::account_match_counts(c, a.id)?;
                let ranks: Vec<Value> = rc_store::bench::ranks(c, a.id)?
                    .into_iter()
                    .map(|r| json!({"queue": r.queue_type, "tier": r.tier, "division": r.division, "lp": r.lp, "wins": r.wins, "losses": r.losses}))
                    .collect();
                out.push(json!({
                    "riot_id": format!("{}#{}", a.game_name, a.tag_line),
                    "platform": a.platform,
                    "primary": a.is_primary,
                    "games_stored": counts,
                    "ranks": ranks,
                }));
            }
            Ok(json!({"accounts": out}))
        }
        "list_matches" => {
            let a = account(c, args)?;
            let limit = int_arg(args, "limit", 20, 1, 200) as usize;
            let champion = str_arg(args, "champion");
            let role = role_arg(args)?;
            let rows: Vec<Value> = rc_store::matches::list_account_matches(c, a.id, 2000, 0)?
                .into_iter()
                .filter(|m| champion.is_none_or(|ch| m.champion_name.eq_ignore_ascii_case(ch)))
                .filter(|m| role.as_deref().is_none_or(|r| m.team_position.as_deref() == Some(r)))
                .take(limit)
                .map(|m| {
                    let has_review = rc_store::reports::has_report(c, rc_coach::detail::POST_GAME, a.id, &m.match_id)?;
                    Ok(json!({
                        "match_id": m.match_id,
                        "start": iso8601(m.game_start_ms),
                        "queue": rc_common::queues::queue_name(m.queue_id),
                        "analyzed": rc_common::queues::is_standard_rift(m.queue_id, m.map_id) && !m.is_remake,
                        "champion": m.champion_name,
                        "role": m.team_position,
                        "win": m.win,
                        "remake": m.is_remake,
                        "kills": m.kills, "deaths": m.deaths, "assists": m.assists,
                        "duration_min": (m.duration_s as f64 / 6.0).round() / 10.0,
                        "has_ai_review": has_review,
                    }))
                })
                .collect::<Result<_, ToolError>>()?;
            Ok(json!({"riot_id": format!("{}#{}", a.game_name, a.tag_line), "matches": rows}))
        }
        "get_patterns" => {
            let a = account(c, args)?;
            let games = int_arg(args, "games", i64::from(s.baseline_games), 5, 500) as u32;
            let role = role_arg(args)?;
            let p = rc_coach::patterns::patterns(c, a.id, games, role.as_deref(), s.session_gap_min)?;
            Ok(json!({
                "games": p.games, "wins": p.wins, "role": p.role, "roles": p.roles,
                "signals": signals(&p.signals)?,
                "win_links": to_value(&p.win_links)?,
            }))
        }
        "get_goals" => {
            let a = account(c, args)?;
            let include = args.get("include_ended").and_then(Value::as_bool).unwrap_or(false);
            Ok(json!({"goals": to_value(rc_coach::goals::list(c, a.id, include)?)?}))
        }
        "list_reports" => {
            let a = account(c, args)?;
            let limit = int_arg(args, "limit", 20, 1, 100) as usize;
            let kind = str_arg(args, "kind");
            let rows: Vec<Value> = rc_store::reports::list(c, Some(a.id), 500)?
                .into_iter()
                .filter(|r| kind.is_none_or(|k| r.kind == k))
                .take(limit)
                .map(|r| {
                    json!({
                        "report_id": r.id, "kind": r.kind, "match_id": r.match_id, "scope": r.scope,
                        "model": r.model, "created": iso8601(r.created_at),
                        "summary": r.output.get("summary"),
                    })
                })
                .collect();
            Ok(json!({"reports": rows}))
        }
        "get_report" => {
            let id = args
                .get("report_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| ToolError::Invalid("report_id is required".into()))?;
            let r = rc_store::reports::get(c, id)?.ok_or_else(|| ToolError::Invalid(format!("no report {id}")))?;
            Ok(json!({
                "report_id": r.id, "kind": r.kind, "match_id": r.match_id, "scope": r.scope, "model": r.model,
                "created": iso8601(r.created_at), "report": r.output,
            }))
        }
        "get_week" => {
            let a = account(c, args)?;
            let now = rc_common::time::now_ms();
            let week = str_arg(args, "week").map_or_else(|| rc_coach::weekly::week_of(now), str::to_owned);
            Ok(to_value(rc_coach::weekly::detail(c, a.id, &week, &s, now)?)?)
        }
        "get_benchmarks" => {
            let a = account(c, args)?;
            let role = match role_arg(args)? {
                Some(r) => r,
                None => rc_coach::benchmarks::focus(c, Some(a.id), &s)?
                    .roles
                    .into_iter()
                    .next()
                    .ok_or_else(|| ToolError::Invalid("No Summoner's Rift games yet.".into()))?,
            };
            let filter = rc_store::metrics::GameFilter { limit: s.baseline_games, ..Default::default() };
            Ok(to_value(rc_coach::benchmarks::gaps(c, a.id, &s, &role, str_arg(args, "champion"), filter)?)?)
        }
        "get_champions" => {
            let a = account(c, args)?;
            match str_arg(args, "champion") {
                Some(ch) => {
                    // Accept any capitalization of the Data Dragon id.
                    let list = rc_coach::champions::list(c, a.id, &s)?;
                    let name =
                        list.iter().find(|x| x.champion.eq_ignore_ascii_case(ch)).map_or(ch, |x| x.champion.as_str());
                    let detail = rc_coach::champions::detail(c, a.id, name, &s)?;
                    let mut d = to_value(&detail)?;
                    d["signals"] = signals(&detail.signals)?;
                    Ok(d)
                }
                None => Ok(json!({"champions": to_value(rc_coach::champions::list(c, a.id, &s)?)?})),
            }
        }
        "list_sessions" => {
            let a = account(c, args)?;
            let limit = int_arg(args, "limit", 20, 1, 100) as usize;
            Ok(json!({"sessions": to_value(rc_coach::sessions::list(c, a.id, s.session_gap_min, limit)?)?}))
        }
        other => Err(ToolError::Unknown(other.to_owned())),
    }
}
