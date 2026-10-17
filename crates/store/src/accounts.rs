use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Account {
    pub id: i64,
    pub game_name: String,
    pub tag_line: String,
    pub platform: String,
    pub region: String,
    pub is_primary: bool,
}

const ACCOUNT_COLS: &str = "id, game_name, tag_line, platform, region, is_primary";

fn row_to_account(r: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        game_name: r.get(1)?,
        tag_line: r.get(2)?,
        platform: r.get(3)?,
        region: r.get(4)?,
        is_primary: r.get(5)?,
    })
}

/// Finds or creates the account (Riot ID match is case-insensitive). The first
/// account ever added becomes primary; `make_primary` forces it.
pub fn upsert_account(
    conn: &mut Connection,
    game_name: &str,
    tag_line: &str,
    platform: &str,
    region: &str,
    make_primary: bool,
) -> Result<i64> {
    let tx = conn.transaction()?;
    let now = now_ms();
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM accounts
             WHERE game_name = ?1 COLLATE NOCASE AND tag_line = ?2 COLLATE NOCASE AND platform = ?3",
            params![game_name, tag_line, platform],
            |r| r.get(0),
        )
        .optional()?;
    let id = match existing {
        Some(id) => id,
        None => {
            tx.execute(
                "INSERT INTO accounts (game_name, tag_line, platform, region, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![game_name, tag_line, platform, region, now],
            )?;
            tx.last_insert_rowid()
        }
    };
    let has_primary: bool =
        tx.query_row("SELECT EXISTS(SELECT 1 FROM accounts WHERE is_primary = 1)", [], |r| r.get(0))?;
    if make_primary || !has_primary {
        tx.execute("UPDATE accounts SET is_primary = (id = ?1)", params![id])?;
    }
    tx.commit()?;
    Ok(id)
}

pub fn get_account(conn: &Connection, id: i64) -> Result<Option<Account>> {
    Ok(conn
        .query_row(&format!("SELECT {ACCOUNT_COLS} FROM accounts WHERE id = ?1"), [id], row_to_account)
        .optional()?)
}

pub fn primary_account(conn: &Connection) -> Result<Option<Account>> {
    Ok(conn
        .query_row(&format!("SELECT {ACCOUNT_COLS} FROM accounts WHERE is_primary = 1"), [], row_to_account)
        .optional()?)
}

pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>> {
    let mut stmt = conn.prepare(&format!("SELECT {ACCOUNT_COLS} FROM accounts ORDER BY is_primary DESC, id"))?;
    let rows = stmt.query_map([], row_to_account)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn set_primary(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("UPDATE accounts SET is_primary = (id = ?1), updated_at = ?2", params![id, now_ms()])?;
    Ok(())
}

/// Stops tracking an account: its PUUIDs, match links and AI reports go,
/// the matches themselves stay. Another account becomes primary if needed.
pub fn delete_account(conn: &mut Connection, id: i64) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
    let has_primary: bool =
        tx.query_row("SELECT EXISTS(SELECT 1 FROM accounts WHERE is_primary = 1)", [], |r| r.get(0))?;
    if !has_primary {
        tx.execute("UPDATE accounts SET is_primary = 1 WHERE id = (SELECT MIN(id) FROM accounts)", [])?;
    }
    let mut s = crate::settings::app(&tx)?;
    if s.active_account_id == Some(id) {
        s.active_account_id = None;
        crate::settings::set(&tx, crate::settings::APP, &s)?;
    }
    tx.commit()?;
    Ok(())
}

/// Store Riot's canonical casing of the Riot ID.
pub fn update_riot_id(conn: &Connection, id: i64, game_name: &str, tag_line: &str) -> Result<()> {
    conn.execute(
        "UPDATE accounts SET game_name = ?2, tag_line = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, game_name, tag_line, now_ms()],
    )?;
    Ok(())
}

pub fn latest_key_gen(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT MAX(id) FROM key_generations", [], |r| r.get(0))?)
}

pub fn puuid_for(conn: &Connection, account_id: i64, key_gen_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT puuid FROM account_puuids WHERE account_id = ?1 AND key_gen_id = ?2",
            params![account_id, key_gen_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The account's PUUID under the current key generation.
pub fn current_puuid(conn: &Connection, account_id: i64) -> Result<Option<String>> {
    match latest_key_gen(conn)? {
        Some(g) => puuid_for(conn, account_id, g),
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PuuidOutcome {
    /// First PUUID ever stored; generation 1 created.
    FirstGeneration { key_gen_id: i64 },
    /// Same PUUID as before under the current key.
    Unchanged { key_gen_id: i64 },
    /// Account newly resolved under the current key.
    Recorded { key_gen_id: i64 },
    /// PUUID differs from the one stored for this key generation, which means
    /// the API key changed. A new generation was opened; other tracked
    /// accounts must be re-resolved.
    KeyChanged { key_gen_id: i64 },
}

impl PuuidOutcome {
    pub fn key_gen_id(self) -> i64 {
        match self {
            Self::FirstGeneration { key_gen_id }
            | Self::Unchanged { key_gen_id }
            | Self::Recorded { key_gen_id }
            | Self::KeyChanged { key_gen_id } => key_gen_id,
        }
    }
}

/// Records a PUUID just returned by Account-V1 and detects key changes.
pub fn record_resolved_puuid(conn: &mut Connection, account_id: i64, puuid: &str) -> Result<PuuidOutcome> {
    let tx = conn.transaction()?;
    let now = now_ms();
    let new_gen = |tx: &rusqlite::Transaction<'_>, note: &str| -> Result<i64> {
        tx.execute("INSERT INTO key_generations (first_seen_at, note) VALUES (?1, ?2)", params![now, note])?;
        Ok(tx.last_insert_rowid())
    };
    let outcome = match latest_key_gen(&tx)? {
        None => PuuidOutcome::FirstGeneration { key_gen_id: new_gen(&tx, "initial key")? },
        Some(g) => match puuid_for(&tx, account_id, g)? {
            None => PuuidOutcome::Recorded { key_gen_id: g },
            Some(stored) if stored == puuid => PuuidOutcome::Unchanged { key_gen_id: g },
            Some(_) => PuuidOutcome::KeyChanged { key_gen_id: new_gen(&tx, "PUUID changed: new API key")? },
        },
    };
    tx.execute(
        "INSERT INTO account_puuids (account_id, key_gen_id, puuid, resolved_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (account_id, key_gen_id) DO UPDATE SET puuid = excluded.puuid, resolved_at = excluded.resolved_at",
        params![account_id, outcome.key_gen_id(), puuid, now],
    )?;
    tx.commit()?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    #[test]
    fn upsert_is_case_insensitive_and_sets_primary() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            let a = upsert_account(c, "TapRooted", "BTC", "na1", "americas", false)?;
            let b = upsert_account(c, "taprooted", "btc", "na1", "americas", false)?;
            assert_eq!(a, b);
            let smurf = upsert_account(c, "Smurf", "NA1", "na1", "americas", false)?;
            assert_ne!(a, smurf);
            assert_eq!(primary_account(c)?.map(|p| p.id), Some(a));
            upsert_account(c, "Smurf", "NA1", "na1", "americas", true)?;
            assert_eq!(primary_account(c)?.map(|p| p.id), Some(smurf));
            assert_eq!(list_accounts(c)?.len(), 2);
            Ok(())
        })
        .expect("ok");
    }

    #[test]
    fn detects_key_changes_from_puuid_mismatch() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            let me = upsert_account(c, "Me", "NA1", "na1", "americas", false)?;
            let friend = upsert_account(c, "Friend", "NA1", "na1", "americas", false)?;

            assert_eq!(record_resolved_puuid(c, me, "p-me-1")?, PuuidOutcome::FirstGeneration { key_gen_id: 1 });
            assert_eq!(record_resolved_puuid(c, me, "p-me-1")?, PuuidOutcome::Unchanged { key_gen_id: 1 });
            assert_eq!(record_resolved_puuid(c, friend, "p-fr-1")?, PuuidOutcome::Recorded { key_gen_id: 1 });

            // New key: same account, different encrypted PUUID.
            assert_eq!(record_resolved_puuid(c, me, "p-me-2")?, PuuidOutcome::KeyChanged { key_gen_id: 2 });
            assert_eq!(current_puuid(c, me)?.as_deref(), Some("p-me-2"));
            // Friend has no PUUID under the new key until re-resolved...
            assert_eq!(current_puuid(c, friend)?, None);
            // ...and the old one is kept for old data.
            assert_eq!(puuid_for(c, friend, 1)?.as_deref(), Some("p-fr-1"));
            assert_eq!(record_resolved_puuid(c, friend, "p-fr-2")?, PuuidOutcome::Recorded { key_gen_id: 2 });
            Ok(())
        })
        .expect("ok");
    }
}
