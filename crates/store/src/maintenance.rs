//! Backup, restore, and "delete all my data". Restores and deletes are
//! staged as files in the data folder and carried out by [`apply_pending`]
//! on the next start, before the database is opened, so nothing is swapped
//! under an open connection.

use std::path::{Path, PathBuf};

use rc_common::AppPaths;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::{Result, StoreError};

/// A backup staged to replace the database on the next start.
const RESTORE_PENDING: &str = "restore-pending.db";
/// Marker: wipe the data folder on the next start.
const DELETE_PENDING: &str = "delete-pending";
/// The database as it was before the last restore.
const BEFORE_RESTORE: &str = "rift-coach.before-restore.db";

/// Schema version a database at the newest migration has.
pub fn schema_version() -> i64 {
    crate::migrations_len() as i64
}

/// Writes a consistent copy of the database to `dest` (replacing it).
pub fn backup_to(conn: &Connection, dest: &Path) -> Result<()> {
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    let dest = dest.to_str().ok_or_else(|| StoreError::Io(std::io::Error::other("the path isn't valid Unicode")))?;
    conn.execute("VACUUM INTO ?1", [dest])?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub schema_version: i64,
    pub accounts: i64,
    pub matches: i64,
    pub reports: i64,
}

/// Opens a file read-only and checks it's a Rift Coach database this
/// version can use.
pub fn inspect_backup(path: &Path) -> Result<BackupInfo> {
    let invalid = |m: &str| StoreError::Io(std::io::Error::other(m.to_owned()));
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|_| invalid("That file isn't a database."))?;
    let version: i64 =
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|_| invalid("That file isn't a database."))?;
    if version == 0 {
        return Err(invalid("That file isn't a Rift Coach backup."));
    }
    if version > schema_version() {
        return Err(invalid("That backup is from a newer Rift Coach. Update the app first."));
    }
    let count = |t: &str| -> Result<i64> {
        conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
            .map_err(|_| invalid("That file isn't a Rift Coach backup."))
    };
    Ok(BackupInfo {
        schema_version: version,
        accounts: count("accounts")?,
        matches: count("matches")?,
        reports: count("ai_reports")?,
    })
}

/// Stages `backup` to replace the database on the next start.
pub fn stage_restore(paths: &AppPaths, backup: &Path) -> Result<BackupInfo> {
    let info = inspect_backup(backup)?;
    std::fs::copy(backup, paths.root.join(RESTORE_PENDING))?;
    Ok(info)
}

/// Stages a wipe of the data folder on the next start.
pub fn stage_delete(paths: &AppPaths) -> Result<()> {
    std::fs::write(paths.root.join(DELETE_PENDING), b"")?;
    Ok(())
}

/// What [`apply_pending`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Nothing,
    Restored,
    Deleted,
}

fn db_files(db: &Path) -> [PathBuf; 3] {
    let with = |suffix: &str| {
        let mut s = db.as_os_str().to_owned();
        s.push(suffix);
        PathBuf::from(s)
    };
    [db.to_path_buf(), with("-wal"), with("-shm")]
}

fn remove_if_exists(p: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Carries out a staged delete or restore. Call before opening the
/// database or the log file.
pub fn apply_pending(paths: &AppPaths) -> std::io::Result<Applied> {
    let delete = paths.root.join(DELETE_PENDING);
    if delete.exists() {
        for f in db_files(&paths.db_file) {
            remove_if_exists(&f)?;
        }
        for f in [RESTORE_PENDING, BEFORE_RESTORE] {
            remove_if_exists(&paths.root.join(f))?;
        }
        for dir in [&paths.ddragon_dir, &paths.logs_dir, &paths.vision_dir] {
            if dir.exists() {
                std::fs::remove_dir_all(dir)?;
            }
            std::fs::create_dir_all(dir)?;
        }
        std::fs::remove_file(delete)?;
        return Ok(Applied::Deleted);
    }
    let restore = paths.root.join(RESTORE_PENDING);
    if restore.exists() {
        let before = paths.root.join(BEFORE_RESTORE);
        let [db, wal, shm] = db_files(&paths.db_file);
        // Keep the old database (and its WAL, under matching names) in case
        // the restore was a mistake.
        for (current, old) in [&db, &wal, &shm].into_iter().zip(db_files(&before)) {
            remove_if_exists(&old)?;
            if current.exists() {
                std::fs::rename(current, &old)?;
            }
        }
        std::fs::rename(&restore, &db)?;
        // Frame folders are named by live game ids of the old database.
        if paths.vision_dir.exists() {
            std::fs::remove_dir_all(&paths.vision_dir)?;
        }
        std::fs::create_dir_all(&paths.vision_dir)?;
        return Ok(Applied::Restored);
    }
    Ok(Applied::Nothing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    #[test]
    fn backup_restore_and_delete() {
        let dir = tempfile::tempdir().expect("tmp");
        let paths = AppPaths::at(dir.path().join("home")).expect("paths");
        let db = Db::open(&paths.db_file).expect("db");
        db.with(|c| crate::accounts::upsert_account(c, "Me", "NA1", "na1", "americas", true)).expect("acct");
        let backup = dir.path().join("backup.db");
        db.with(|c| backup_to(c, &backup)).expect("backup");
        let info = inspect_backup(&backup).expect("valid");
        assert_eq!((info.accounts, info.schema_version), (1, schema_version()));

        // Not a database.
        let junk = dir.path().join("junk.db");
        std::fs::write(&junk, b"hello").expect("write");
        assert!(inspect_backup(&junk).is_err());

        // Change the live database, then restore the backup.
        db.with(|c| crate::accounts::upsert_account(c, "Alt", "NA1", "na1", "americas", false)).expect("acct");
        drop(db);
        stage_restore(&paths, &backup).expect("stage");
        assert_eq!(apply_pending(&paths).expect("apply"), Applied::Restored);
        let db = Db::open(&paths.db_file).expect("reopen");
        assert_eq!(db.with(|c| crate::accounts::list_accounts(c)).expect("list").len(), 1);
        assert!(paths.root.join(BEFORE_RESTORE).exists());
        drop(db);

        std::fs::write(paths.ddragon_dir.join("x.json"), b"{}").expect("write");
        std::fs::create_dir_all(paths.vision_dir.join("1")).expect("mkdir");
        std::fs::write(paths.vision_dir.join("1").join("00004000.jpg"), b"x").expect("write");
        stage_delete(&paths).expect("stage");
        assert_eq!(apply_pending(&paths).expect("apply"), Applied::Deleted);
        assert!(!paths.db_file.exists() && !paths.root.join(BEFORE_RESTORE).exists());
        assert!(paths.ddragon_dir.exists() && !paths.ddragon_dir.join("x.json").exists());
        assert!(paths.vision_dir.exists() && !paths.vision_dir.join("1").exists(), "frames deleted too");
        assert_eq!(apply_pending(&paths).expect("again"), Applied::Nothing);
    }
}
