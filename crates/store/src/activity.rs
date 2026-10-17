//! The activity feed shown in the UI, persisted so it survives restarts.

use rc_common::time::now_ms;
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::Result;

/// Rows kept; older ones are pruned on insert.
const KEEP: i64 = 500;

#[derive(Debug, Clone, Serialize)]
pub struct ActivityEntry {
    pub id: i64,
    pub at: i64,
    pub level: String,
    pub message: String,
}

pub fn add(conn: &Connection, level: &str, message: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO activity_log (at, level, message) VALUES (?1, ?2, ?3)",
        params![now_ms(), level, message],
    )?;
    conn.execute("DELETE FROM activity_log WHERE id <= (SELECT MAX(id) FROM activity_log) - ?1", [KEEP])?;
    Ok(())
}

/// Newest first.
pub fn recent(conn: &Connection, limit: u32) -> Result<Vec<ActivityEntry>> {
    let mut stmt = conn.prepare("SELECT id, at, level, message FROM activity_log ORDER BY id DESC LIMIT ?1")?;
    let rows = stmt
        .query_map([limit], |r| {
            Ok(ActivityEntry { id: r.get(0)?, at: r.get(1)?, level: r.get(2)?, message: r.get(3)? })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_newest() {
        let db = crate::Db::open_in_memory().expect("db");
        db.with(|c| {
            for i in 0..(super::KEEP + 5) {
                super::add(c, "info", &format!("m{i}"))?;
            }
            let all = super::recent(c, 10_000)?;
            assert_eq!(all.len() as i64, super::KEEP);
            assert_eq!(all[0].message, format!("m{}", super::KEEP + 4));
            Ok(())
        })
        .expect("ok");
    }
}
