//! Games seen by the live poller, their snapshots, and planned builds.

use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveGame {
    pub id: i64,
    pub account_id: Option<i64>,
    pub champion_name: String,
    pub champion_ddragon: Option<String>,
    pub position: Option<String>,
    pub game_mode: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub end_reason: Option<String>,
    pub result: Option<String>,
    pub duration_s: Option<f64>,
    pub final_json: Option<String>,
    pub match_id: Option<String>,
    pub analyzed_at: Option<i64>,
    pub report_id: Option<i64>,
    pub footprint_json: Option<String>,
    pub snapshots_pruned: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewLiveGame<'a> {
    pub account_id: Option<i64>,
    pub champion_name: &'a str,
    pub champion_ddragon: Option<&'a str>,
    pub position: Option<&'a str>,
    pub game_mode: &'a str,
    pub started_at: i64,
}

const COLS: &str = "id, account_id, champion_name, champion_ddragon, position, game_mode, started_at, ended_at,
                    end_reason, result, duration_s, final_json, match_id, analyzed_at, report_id, footprint_json,
                    snapshots_pruned, created_at";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<LiveGame> {
    Ok(LiveGame {
        id: r.get(0)?,
        account_id: r.get(1)?,
        champion_name: r.get(2)?,
        champion_ddragon: r.get(3)?,
        position: r.get(4)?,
        game_mode: r.get(5)?,
        started_at: r.get(6)?,
        ended_at: r.get(7)?,
        end_reason: r.get(8)?,
        result: r.get(9)?,
        duration_s: r.get(10)?,
        final_json: r.get(11)?,
        match_id: r.get(12)?,
        analyzed_at: r.get(13)?,
        report_id: r.get(14)?,
        footprint_json: r.get(15)?,
        snapshots_pruned: r.get(16)?,
        created_at: r.get(17)?,
    })
}

pub fn insert(conn: &Connection, g: &NewLiveGame<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO live_games (account_id, champion_name, champion_ddragon, position, game_mode, started_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![g.account_id, g.champion_name, g.champion_ddragon, g.position, g.game_mode, g.started_at, now_ms()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<LiveGame>> {
    Ok(conn.query_row(&format!("SELECT {COLS} FROM live_games WHERE id = ?1"), [id], row).optional()?)
}

pub fn recent(conn: &Connection, limit: u32) -> Result<Vec<LiveGame>> {
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM live_games ORDER BY started_at DESC, id DESC LIMIT ?1"))?;
    Ok(st.query_map([limit], row)?.collect::<rusqlite::Result<_>>()?)
}

/// The live game linked to a match, if any.
pub fn by_match(conn: &Connection, match_id: &str) -> Result<Option<LiveGame>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM live_games WHERE match_id = ?1 ORDER BY id DESC LIMIT 1"),
            [match_id],
            row,
        )
        .optional()?)
}

/// The account's newest stored match that started within `window_ms` of
/// `started_at` and isn't linked to another live game.
pub fn unlinked_match_near(
    conn: &Connection,
    account_id: i64,
    started_at: i64,
    window_ms: i64,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT m.match_id FROM account_matches am JOIN matches m ON m.match_id = am.match_id
              WHERE am.account_id = ?1 AND abs(m.game_start_ms - ?2) <= ?3
                AND NOT EXISTS (SELECT 1 FROM live_games lg WHERE lg.match_id = m.match_id)
              ORDER BY abs(m.game_start_ms - ?2) LIMIT 1",
            params![account_id, started_at, window_ms],
            |r| r.get(0),
        )
        .optional()?)
}

/// The game that started within `window_ms` of `started_at` (the same game
/// seen again after an app restart).
pub fn find_near(conn: &Connection, started_at: i64, window_ms: i64) -> Result<Option<LiveGame>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM live_games WHERE abs(started_at - ?1) <= ?2 ORDER BY abs(started_at - ?1) LIMIT 1"
            ),
            params![started_at, window_ms],
            row,
        )
        .optional()?)
}

/// Ends games left open (the app closed mid-game and never saw the end)
/// that started before `started_before`. Returns how many.
pub fn close_stale(conn: &Connection, started_before: i64, now: i64) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE live_games SET ended_at = ?2, end_reason = 'disappeared' WHERE ended_at IS NULL AND started_at < ?1",
        params![started_before, now],
    )?)
}

/// Records the end of a game. Returns false if it had already ended.
pub fn end(
    conn: &Connection,
    id: i64,
    ended_at: i64,
    reason: &str,
    result: Option<&str>,
    duration_s: f64,
    final_json: &str,
) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE live_games SET ended_at = ?2, end_reason = ?3, result = ?4, duration_s = ?5, final_json = ?6
         WHERE id = ?1 AND ended_at IS NULL",
        params![id, ended_at, reason, result, duration_s, final_json],
    )? > 0)
}

/// The pre-game brief shown for this game (JSON).
pub fn set_brief(conn: &Connection, id: i64, json: &str) -> Result<()> {
    conn.execute("UPDATE live_games SET brief_json = ?2 WHERE id = ?1", params![id, json])?;
    Ok(())
}

pub fn set_footprint(conn: &Connection, id: i64, json: &str) -> Result<()> {
    conn.execute("UPDATE live_games SET footprint_json = ?2 WHERE id = ?1", params![id, json])?;
    Ok(())
}

pub fn link_match(conn: &Connection, id: i64, match_id: &str) -> Result<()> {
    conn.execute("UPDATE live_games SET match_id = ?2 WHERE id = ?1", params![id, match_id])?;
    Ok(())
}

/// Marks the linked game analyzed. Returns false if it already was.
pub fn mark_analyzed(conn: &Connection, id: i64, at: i64) -> Result<bool> {
    Ok(conn.execute("UPDATE live_games SET analyzed_at = ?2 WHERE id = ?1 AND analyzed_at IS NULL", params![id, at])?
        > 0)
}

pub fn set_report(conn: &Connection, id: i64, report_id: i64) -> Result<()> {
    conn.execute("UPDATE live_games SET report_id = ?2 WHERE id = ?1", params![id, report_id])?;
    Ok(())
}

pub fn add_snapshot(conn: &Connection, id: i64, t_ms: i64, data: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO live_snapshots (live_game_id, t_ms, data) VALUES (?1, ?2, ?3)",
        params![id, t_ms, data],
    )?;
    Ok(())
}

pub fn snapshots(conn: &Connection, id: i64) -> Result<Vec<(i64, String)>> {
    let mut st = conn.prepare("SELECT t_ms, data FROM live_snapshots WHERE live_game_id = ?1 ORDER BY t_ms")?;
    Ok(st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
}

/// Deletes snapshots of games analyzed before `analyzed_before`, and of
/// games that never got a match (practice tool, remakes not published...)
/// that ended before `orphan_before`. Returns the games pruned.
pub fn prune_snapshots(conn: &Connection, analyzed_before: i64, orphan_before: i64) -> Result<usize> {
    let ids: Vec<i64> = {
        let mut st = conn.prepare(
            "SELECT id FROM live_games WHERE snapshots_pruned = 0 AND ended_at IS NOT NULL
               AND ((analyzed_at IS NOT NULL AND analyzed_at <= ?1) OR (analyzed_at IS NULL AND ended_at <= ?2))",
        )?;
        st.query_map(params![analyzed_before, orphan_before], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    for id in &ids {
        conn.execute("DELETE FROM live_snapshots WHERE live_game_id = ?1", [id])?;
        conn.execute("UPDATE live_games SET snapshots_pruned = 1 WHERE id = ?1", [id])?;
    }
    Ok(ids.len())
}

pub fn build_plan(conn: &Connection, champion: &str) -> Result<Option<Vec<i32>>> {
    let raw: Option<String> =
        conn.query_row("SELECT items FROM build_plans WHERE champion = ?1", [champion], |r| r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into)).transpose()
}

/// An empty list removes the plan (back to "your usual build").
pub fn set_build_plan(conn: &Connection, champion: &str, items: &[i32]) -> Result<()> {
    if items.is_empty() {
        conn.execute("DELETE FROM build_plans WHERE champion = ?1", [champion])?;
    } else {
        conn.execute(
            "INSERT INTO build_plans (champion, items, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (champion) DO UPDATE SET items = excluded.items, updated_at = excluded.updated_at",
            params![champion, serde_json::to_string(items)?, now_ms()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_and_pruning() {
        let db = crate::Db::open_in_memory().expect("db");
        db.with(|c| {
            let new = NewLiveGame {
                account_id: None,
                champion_name: "LeBlanc",
                champion_ddragon: Some("Leblanc"),
                position: Some("MIDDLE"),
                game_mode: "CLASSIC",
                started_at: 1_000,
            };
            let a = insert(c, &new)?;
            let b = insert(c, &NewLiveGame { started_at: 2_000, game_mode: "PRACTICETOOL", ..new })?;
            add_snapshot(c, a, 15_000, "{}")?;
            add_snapshot(c, a, 15_000, "{\"x\":1}")?;
            add_snapshot(c, b, 15_000, "{}")?;
            assert_eq!(snapshots(c, a)?, vec![(15_000, "{\"x\":1}".to_owned())]);
            assert!(end(c, a, 50_000, "game_end", Some("Win"), 1800.0, "{}")?);
            assert!(!end(c, a, 99_000, "disappeared", None, 1.0, "{}")?);
            assert_eq!(get(c, a)?.and_then(|g| g.result), Some("Win".into()), "end only once");
            assert_eq!(find_near(c, 1_500, 600)?.map(|g| g.id), Some(a));
            assert_eq!(find_near(c, 1_900, 600)?.map(|g| g.id), Some(b));
            assert!(find_near(c, 9_000, 600)?.is_none());
            end(c, b, 60_000, "disappeared", None, 600.0, "{}")?;
            link_match(c, a, "NA1_1")?;
            assert!(mark_analyzed(c, a, 70_000)?);
            assert!(!mark_analyzed(c, a, 71_000)?);
            assert_eq!(by_match(c, "NA1_1")?.map(|g| g.id), Some(a));
            assert_eq!(recent(c, 10)?.first().map(|g| g.id), Some(b));

            // Analyzed game pruned; the orphan isn't old enough yet.
            assert_eq!(prune_snapshots(c, 80_000, 10_000)?, 1);
            assert!(snapshots(c, a)?.is_empty());
            assert_eq!(snapshots(c, b)?.len(), 1);
            assert_eq!(prune_snapshots(c, 80_000, 70_000)?, 1);
            assert_eq!(prune_snapshots(c, i64::MAX, i64::MAX)?, 0);

            assert_eq!(build_plan(c, "Leblanc")?, None);
            set_build_plan(c, "Leblanc", &[6655, 3020])?;
            assert_eq!(build_plan(c, "Leblanc")?, Some(vec![6655, 3020]));
            set_build_plan(c, "Leblanc", &[])?;
            assert_eq!(build_plan(c, "Leblanc")?, None);

            let open = insert(c, &NewLiveGame { started_at: 3_000, ..new })?;
            assert_eq!(close_stale(c, 2_500, 7_000)?, 0, "only games started before the cutoff");
            assert_eq!(close_stale(c, 4_000, 7_000)?, 1);
            assert_eq!(get(c, open)?.and_then(|g| g.end_reason), Some("disappeared".into()));
            Ok(())
        })
        .expect("ok");
    }
}
