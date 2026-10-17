use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};

use crate::Result;

pub fn is_complete(conn: &Connection, version: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT complete FROM ddragon_versions WHERE version = ?1", [version], |r| r.get(0))
        .optional()?
        .unwrap_or(false))
}

pub fn mark_complete(conn: &Connection, version: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO ddragon_versions (version, fetched_at, complete) VALUES (?1, ?2, 1)
         ON CONFLICT (version) DO UPDATE SET fetched_at = excluded.fetched_at, complete = 1",
        params![version, now_ms()],
    )?;
    Ok(())
}

pub fn cached_versions(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT version FROM ddragon_versions WHERE complete = 1 ORDER BY fetched_at DESC")?;
    let rows = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}
