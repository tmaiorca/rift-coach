//! Focus goals. Progress is computed from stored games (rc-coach); this
//! module only keeps the goal definitions and their status.

use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Goal {
    pub id: i64,
    pub account_id: i64,
    pub metric: String,
    /// `at_most` | `at_least`
    pub comparator: String,
    pub target: f64,
    pub games: u32,
    pub role: Option<String>,
    pub champion_name: Option<String>,
    pub baseline: Option<f64>,
    /// `user` | `ai` | `pattern`
    pub source: String,
    pub note: Option<String>,
    /// `active` | `completed` | `abandoned`
    pub status: String,
    pub achieved: Option<bool>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewGoal<'a> {
    pub account_id: i64,
    pub metric: &'a str,
    pub comparator: &'a str,
    pub target: f64,
    pub games: u32,
    pub role: Option<&'a str>,
    pub champion_name: Option<&'a str>,
    pub baseline: Option<f64>,
    pub source: &'a str,
    pub note: Option<&'a str>,
}

const COLS: &str = "id, account_id, metric, comparator, target, games, role, champion_name, baseline, source, note,
                    status, achieved, created_at, ended_at";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Goal> {
    Ok(Goal {
        id: r.get(0)?,
        account_id: r.get(1)?,
        metric: r.get(2)?,
        comparator: r.get(3)?,
        target: r.get(4)?,
        games: r.get(5)?,
        role: r.get(6)?,
        champion_name: r.get(7)?,
        baseline: r.get(8)?,
        source: r.get(9)?,
        note: r.get(10)?,
        status: r.get(11)?,
        achieved: r.get(12)?,
        created_at: r.get(13)?,
        ended_at: r.get(14)?,
    })
}

/// `created_at` defaults to now; tests pass an explicit time.
pub fn insert(conn: &Connection, g: &NewGoal<'_>, created_at: Option<i64>) -> Result<i64> {
    conn.execute(
        "INSERT INTO goals (account_id, metric, comparator, target, games, role, champion_name, baseline, source, note,
                            created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            g.account_id,
            g.metric,
            g.comparator,
            g.target,
            g.games,
            g.role,
            g.champion_name,
            g.baseline,
            g.source,
            g.note,
            created_at.unwrap_or_else(now_ms)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Goal>> {
    Ok(conn.query_row(&format!("SELECT {COLS} FROM goals WHERE id = ?1"), [id], row).optional()?)
}

/// Active goals first, then ended ones, newest first within each.
pub fn list(conn: &Connection, account_id: i64, include_ended: bool) -> Result<Vec<Goal>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM goals WHERE account_id = ?1 AND (?2 OR status = 'active')
         ORDER BY status = 'active' DESC, created_at DESC, id DESC"
    ))?;
    let rows = stmt.query_map(params![account_id, include_ended], row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Ends a goal (`completed` or `abandoned`), or reopens it (`active`).
pub fn set_status(conn: &Connection, id: i64, status: &str, achieved: Option<bool>) -> Result<()> {
    let ended = (status != "active").then(now_ms);
    conn.execute(
        "UPDATE goals SET status = ?2, achieved = ?3, ended_at = ?4 WHERE id = ?1",
        params![id, status, achieved, ended],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM goals WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use crate::accounts::upsert_account;

    #[test]
    fn lifecycle() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            let me = upsert_account(c, "Me", "NA1", "na1", "americas", true)?;
            let g = NewGoal {
                account_id: me,
                metric: "deaths_pre14",
                comparator: "at_most",
                target: 1.0,
                games: 10,
                role: Some("MIDDLE"),
                champion_name: None,
                baseline: Some(2.1),
                source: "user",
                note: None,
            };
            let a = insert(c, &g, Some(1_000))?;
            let b = insert(c, &NewGoal { metric: "cs_10", comparator: "at_least", target: 70.0, ..g }, Some(2_000))?;
            assert_eq!(list(c, me, false)?.iter().map(|g| g.id).collect::<Vec<_>>(), vec![b, a]);

            set_status(c, a, "completed", Some(true))?;
            assert_eq!(list(c, me, false)?.len(), 1);
            let all = list(c, me, true)?;
            assert_eq!(all.iter().map(|g| g.id).collect::<Vec<_>>(), vec![b, a], "active first");
            let done = get(c, a)?.expect("goal");
            assert_eq!((done.status.as_str(), done.achieved), ("completed", Some(true)));
            assert!(done.ended_at.is_some());

            delete(c, b)?;
            assert!(get(c, b)?.is_none());
            Ok(())
        })
        .expect("ok");
    }
}
