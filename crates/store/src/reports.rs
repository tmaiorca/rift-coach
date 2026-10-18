//! AI report history. Every report is kept; the newest for a match (with
//! the same prompt version, provider and model) doubles as the cache.

use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Result;

#[derive(Debug, Clone)]
pub struct NewReport<'a> {
    pub kind: &'a str,
    pub account_id: i64,
    pub match_id: Option<&'a str>,
    /// For reports covering several games, e.g. `session:NA1_123:4`.
    pub scope: Option<&'a str>,
    pub provider: &'a str,
    pub model: &'a str,
    pub prompt_version: &'a str,
    pub input_json: &'a str,
    pub output_json: &'a str,
    pub validation: &'a str,
    pub notes: Option<&'a str>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub id: i64,
    pub kind: String,
    pub account_id: i64,
    pub match_id: Option<String>,
    pub scope: Option<String>,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub output: serde_json::Value,
    pub validation: String,
    pub notes: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub created_at: i64,
}

const COLS: &str = "id, kind, account_id, match_id, provider, model, prompt_version, output_json, validation, notes,
                    input_tokens, output_tokens, created_at, scope";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Report> {
    let output: String = r.get(7)?;
    Ok(Report {
        id: r.get(0)?,
        kind: r.get(1)?,
        account_id: r.get(2)?,
        match_id: r.get(3)?,
        provider: r.get(4)?,
        model: r.get(5)?,
        prompt_version: r.get(6)?,
        output: serde_json::from_str(&output).unwrap_or(serde_json::Value::Null),
        validation: r.get(8)?,
        notes: r.get(9)?,
        input_tokens: r.get(10)?,
        output_tokens: r.get(11)?,
        created_at: r.get(12)?,
        scope: r.get(13)?,
    })
}

pub fn insert(conn: &Connection, r: &NewReport<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO ai_reports (kind, account_id, match_id, provider, model, prompt_version, input_json, output_json,
                                 validation, notes, input_tokens, output_tokens, created_at, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            r.kind,
            r.account_id,
            r.match_id,
            r.provider,
            r.model,
            r.prompt_version,
            r.input_json,
            r.output_json,
            r.validation,
            r.notes,
            r.input_tokens,
            r.output_tokens,
            now_ms(),
            r.scope
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Newest report of `kind` for a match and account, any provider/model.
pub fn latest_for_match(conn: &Connection, kind: &str, account_id: i64, match_id: &str) -> Result<Option<Report>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM ai_reports WHERE kind = ?1 AND account_id = ?2 AND match_id = ?3
                 ORDER BY created_at DESC, id DESC LIMIT 1"
            ),
            params![kind, account_id, match_id],
            row,
        )
        .optional()?)
}

/// Newest report of `kind` for a multi-game scope (e.g. a session).
pub fn latest_for_scope(conn: &Connection, kind: &str, account_id: i64, scope: &str) -> Result<Option<Report>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM ai_reports WHERE kind = ?1 AND account_id = ?2 AND scope = ?3
                 ORDER BY created_at DESC, id DESC LIMIT 1"
            ),
            params![kind, account_id, scope],
            row,
        )
        .optional()?)
}

/// Reports generated since `since_ms` (all accounts) and their total tokens,
/// for the monthly usage cap.
pub fn usage_since(conn: &Connection, since_ms: i64) -> Result<(i64, i64)> {
    Ok(conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0)), 0)
           FROM ai_reports WHERE created_at >= ?1",
        [since_ms],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// Report history, newest first.
pub fn list(conn: &Connection, account_id: Option<i64>, limit: u32) -> Result<Vec<Report>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM ai_reports WHERE (?1 IS NULL OR account_id = ?1) ORDER BY created_at DESC, id DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![account_id, limit], row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Report>> {
    Ok(conn.query_row(&format!("SELECT {COLS} FROM ai_reports WHERE id = ?1"), [id], row).optional()?)
}

/// Match IDs that already have a report of `kind` for the account.
pub fn has_report(conn: &Connection, kind: &str, account_id: i64, match_id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM ai_reports WHERE kind = ?1 AND account_id = ?2 AND match_id = ?3)",
        params![kind, account_id, match_id],
        |r| r.get(0),
    )?)
}
