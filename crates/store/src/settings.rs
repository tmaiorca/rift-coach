//! Non-secret key/value settings stored as JSON. Never put keys or tokens here.

use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Result;

pub fn get<T: DeserializeOwned>(conn: &Connection, key: &str) -> Result<Option<T>> {
    let raw: Option<String> =
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into)).transpose()
}

pub fn set<T: Serialize>(conn: &Connection, key: &str, value: &T) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, serde_json::to_string(value)?, now_ms()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trips() {
        let db = crate::Db::open_in_memory().expect("db");
        db.with(|c| {
            assert_eq!(super::get::<u32>(c, "x")?, None);
            super::set(c, "x", &5u32)?;
            super::set(c, "x", &7u32)?;
            assert_eq!(super::get::<u32>(c, "x")?, Some(7));
            Ok(())
        })
        .expect("ok");
    }
}
