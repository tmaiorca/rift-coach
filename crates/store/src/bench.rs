//! Higher-rank benchmark collection: sampled ladder players, the queue of
//! match ids to fetch, and the benchmark populations built from those games.

use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::Result;
use crate::metrics::{Population, rift_filter};

/// Marks a stored match as a benchmark sample for `tier`.
pub fn set_benchmark_tier(conn: &Connection, match_id: &str, tier: &str) -> Result<()> {
    conn.execute("UPDATE matches SET benchmark_tier = ?2 WHERE match_id = ?1", params![match_id, tier])?;
    Ok(())
}

/// When the ladder was last sampled for this key generation and tier.
pub fn seeded_at(conn: &Connection, key_gen_id: i64, tier: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT seeded_at FROM bench_seeds WHERE key_gen_id = ?1 AND tier = ?2",
            params![key_gen_id, tier],
            |r| r.get(0),
        )
        .optional()?)
}

/// Adds sampled players (existing ones are kept) and records the seeding.
pub fn add_players(
    conn: &mut Connection,
    key_gen_id: i64,
    platform: &str,
    tier: &str,
    puuids: &[String],
) -> Result<usize> {
    let tx = conn.transaction()?;
    let now = now_ms();
    let mut added = 0;
    {
        let mut st = tx.prepare(
            "INSERT OR IGNORE INTO bench_players (key_gen_id, puuid, platform, tier, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for p in puuids {
            added += st.execute(params![key_gen_id, p, platform, tier, now])?;
        }
    }
    tx.execute(
        "INSERT INTO bench_seeds (key_gen_id, tier, seeded_at, players) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (key_gen_id, tier) DO UPDATE SET seeded_at = excluded.seeded_at, players = excluded.players",
        params![key_gen_id, tier, now, i64::try_from(puuids.len()).unwrap_or(i64::MAX)],
    )?;
    tx.commit()?;
    Ok(added)
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchPlayer {
    pub puuid: String,
    pub platform: String,
    pub focus_score: Option<i32>,
}

fn player(r: &rusqlite::Row<'_>) -> rusqlite::Result<BenchPlayer> {
    Ok(BenchPlayer { puuid: r.get(0)?, platform: r.get(1)?, focus_score: r.get(2)? })
}

/// A player whose masteries haven't been checked yet.
pub fn next_unchecked(conn: &Connection, key_gen_id: i64, tier: &str) -> Result<Option<BenchPlayer>> {
    Ok(conn
        .query_row(
            "SELECT puuid, platform, focus_score FROM bench_players
              WHERE key_gen_id = ?1 AND tier = ?2 AND focus_score IS NULL AND scanned_at IS NULL
              ORDER BY added_at, puuid LIMIT 1",
            params![key_gen_id, tier],
            player,
        )
        .optional()?)
}

pub fn set_focus_score(conn: &Connection, key_gen_id: i64, puuid: &str, score: i32) -> Result<()> {
    conn.execute(
        "UPDATE bench_players SET focus_score = ?3 WHERE key_gen_id = ?1 AND puuid = ?2",
        params![key_gen_id, puuid, score],
    )?;
    Ok(())
}

/// The next player whose match list to read: with `focus_only`, only
/// players who main a focus champion; otherwise anyone, focus players
/// first, then players already checked, then unchecked ones.
pub fn next_to_scan(conn: &Connection, key_gen_id: i64, tier: &str, focus_only: bool) -> Result<Option<BenchPlayer>> {
    Ok(conn
        .query_row(
            "SELECT puuid, platform, focus_score FROM bench_players
              WHERE key_gen_id = ?1 AND tier = ?2 AND scanned_at IS NULL AND (?3 = 0 OR focus_score > 0)
              ORDER BY COALESCE(focus_score, -1) DESC, added_at, puuid LIMIT 1",
            params![key_gen_id, tier, focus_only],
            player,
        )
        .optional()?)
}

pub fn mark_scanned(conn: &Connection, key_gen_id: i64, puuid: &str) -> Result<()> {
    conn.execute(
        "UPDATE bench_players SET scanned_at = ?3 WHERE key_gen_id = ?1 AND puuid = ?2",
        params![key_gen_id, puuid, now_ms()],
    )?;
    Ok(())
}

/// Queues match ids not seen before. Returns how many were new.
pub fn queue_matches(conn: &mut Connection, ids: &[String], tier: &str, priority: i32) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut added = 0;
    {
        let mut st = tx.prepare(
            "INSERT OR IGNORE INTO bench_queue (match_id, tier, priority, added_at) VALUES (?1, ?2, ?3, ?4)",
        )?;
        let now = now_ms();
        for id in ids {
            added += st.execute(params![id, tier, priority, now])?;
        }
    }
    tx.commit()?;
    Ok(added)
}

pub fn next_queued(conn: &Connection) -> Result<Option<(String, String)>> {
    Ok(conn
        .query_row(
            "SELECT match_id, tier FROM bench_queue WHERE status = 'pending' ORDER BY priority DESC, added_at LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

/// `done` or `skipped`.
pub fn set_queued_status(conn: &Connection, match_id: &str, status: &str) -> Result<()> {
    conn.execute("UPDATE bench_queue SET status = ?2 WHERE match_id = ?1", params![match_id, status])?;
    Ok(())
}

/// Forgets pending ids (e.g. after the tier changes).
pub fn clear_pending(conn: &Connection) -> Result<usize> {
    Ok(conn.execute("DELETE FROM bench_queue WHERE status = 'pending'", [])?)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CollectorCounts {
    pub players: i64,
    pub players_checked: i64,
    pub focus_players: i64,
    pub players_scanned: i64,
    pub queued: i64,
}

pub fn collector_counts(conn: &Connection, key_gen_id: i64, tier: &str) -> Result<CollectorCounts> {
    let (players, players_checked, focus_players, players_scanned) = conn.query_row(
        "SELECT COUNT(*), COUNT(focus_score), COALESCE(SUM(focus_score > 0), 0), COUNT(scanned_at)
           FROM bench_players WHERE key_gen_id = ?1 AND tier = ?2",
        params![key_gen_id, tier],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let queued = conn.query_row("SELECT COUNT(*) FROM bench_queue WHERE status = 'pending'", [], |r| r.get(0))?;
    Ok(CollectorCounts { players, players_checked, focus_players, players_scanned, queued })
}

/// Benchmark games per patch for a tier (with metrics computed), newest patch first.
pub fn patches(conn: &Connection, tier: &str) -> Result<Vec<(String, i64)>> {
    let mut st = conn.prepare(
        "SELECT m.patch, COUNT(*) FROM matches m
          WHERE m.benchmark_tier = ?1 AND EXISTS (SELECT 1 FROM match_metrics_state s WHERE s.match_id = m.match_id)
          GROUP BY m.patch",
    )?;
    let mut v: Vec<(String, i64)> =
        st.query_map([tier], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    v.sort_by_key(|p| std::cmp::Reverse(patch_order(&p.0)));
    Ok(v)
}

/// "16.9" < "16.20".
pub fn patch_order(p: &str) -> (u32, u32) {
    let mut it = p.split('.').map(|x| x.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Benchmark samples (players with metrics) in a role, optionally on one
/// champion, for one patch.
pub fn sample_count(conn: &Connection, tier: &str, patch: &str, role: &str, champion: Option<&str>) -> Result<i64> {
    let sql = format!(
        "SELECT COUNT(DISTINCT p.match_id || ':' || p.participant_id)
           FROM match_participants p
           JOIN matches m ON m.match_id = p.match_id
           JOIN participant_metrics pm ON pm.match_id = p.match_id AND pm.participant_id = p.participant_id
          WHERE {} AND m.benchmark_tier = ?1 AND m.patch = ?2 AND p.team_position = ?3
            AND (?4 IS NULL OR p.champion_name = ?4)",
        rift_filter()
    );
    Ok(conn.query_row(&sql, params![tier, patch, role, champion], |r| r.get(0))?)
}

/// Metrics of every benchmark player in `role` (optionally on one champion)
/// on `patch`.
pub fn population(
    conn: &Connection,
    tier: &str,
    patch: &str,
    role: &str,
    champion: Option<&str>,
) -> Result<Population> {
    let sql = format!(
        "SELECT pm.metric, pm.value
           FROM participant_metrics pm
           JOIN match_participants p ON p.match_id = pm.match_id AND p.participant_id = pm.participant_id
           JOIN matches m ON m.match_id = pm.match_id
          WHERE {} AND m.benchmark_tier = ?1 AND m.patch = ?2 AND p.team_position = ?3
            AND (?4 IS NULL OR p.champion_name = ?4)",
        rift_filter()
    );
    let mut st = conn.prepare(&sql)?;
    let mut out = Population::new();
    let mut rows = st.query(params![tier, patch, role, champion])?;
    while let Some(r) = rows.next()? {
        out.entry(r.get(0)?).or_default().push(r.get(1)?);
    }
    Ok(out)
}

/// Deletes benchmark games outside the newest `keep_patches` patches, then
/// the oldest ones beyond `max_matches`. Returns how many were deleted.
pub fn prune(conn: &Connection, keep_patches: usize, max_matches: u32) -> Result<usize> {
    let all: Vec<String> = {
        let mut st = conn.prepare("SELECT DISTINCT patch FROM matches WHERE benchmark_tier IS NOT NULL")?;
        st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let mut patches = all;
    patches.sort_by_key(|p| std::cmp::Reverse(patch_order(p)));
    let mut deleted = 0;
    for old in patches.iter().skip(keep_patches) {
        deleted += conn.execute("DELETE FROM matches WHERE benchmark_tier IS NOT NULL AND patch = ?1", [old])?;
    }
    deleted += conn.execute(
        "DELETE FROM matches WHERE match_id IN (
             SELECT match_id FROM matches WHERE benchmark_tier IS NOT NULL
              ORDER BY game_start_ms DESC LIMIT -1 OFFSET ?1)",
        [max_matches],
    )?;
    Ok(deleted)
}

// ---------- ranks ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rank {
    pub queue_type: String,
    pub tier: Option<String>,
    pub division: Option<String>,
    pub lp: i32,
    pub wins: i32,
    pub losses: i32,
    pub fetched_at: i64,
}

/// Replaces the account's ranks with the latest League-V4 entries.
pub fn set_ranks(conn: &mut Connection, account_id: i64, ranks: &[Rank]) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM ranks WHERE account_id = ?1", [account_id])?;
    for r in ranks {
        tx.execute(
            "INSERT INTO ranks (account_id, queue_type, tier, division, lp, wins, losses, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![account_id, r.queue_type, r.tier, r.division, r.lp, r.wins, r.losses, r.fetched_at],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn ranks(conn: &Connection, account_id: i64) -> Result<Vec<Rank>> {
    let mut st = conn.prepare(
        "SELECT queue_type, tier, division, lp, wins, losses, fetched_at FROM ranks WHERE account_id = ?1
          ORDER BY queue_type DESC",
    )?;
    Ok(st
        .query_map([account_id], |r| {
            Ok(Rank {
                queue_type: r.get(0)?,
                tier: r.get(1)?,
                division: r.get(2)?,
                lp: r.get(3)?,
                wins: r.get(4)?,
                losses: r.get(5)?,
                fetched_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
}

/// Last time ranks were fetched for an account (any queue).
pub fn ranks_fetched_at(conn: &Connection, account_id: i64) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT MAX(fetched_at) FROM ranks WHERE account_id = ?1", [account_id], |r| r.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_queue_and_patch_order() {
        let db = crate::Db::open_in_memory().expect("db");
        db.with(|c| {
            c.execute("INSERT INTO key_generations (id, first_seen_at) VALUES (1, 0)", [])?;
            let ids: Vec<String> = ["a", "b", "c"].iter().map(|s| (*s).to_owned()).collect();
            assert_eq!(add_players(c, 1, "na1", "DIAMOND", &ids)?, 3);
            assert_eq!(add_players(c, 1, "na1", "DIAMOND", &ids)?, 0, "existing kept");
            assert!(seeded_at(c, 1, "DIAMOND")?.is_some());
            assert_eq!(next_unchecked(c, 1, "DIAMOND")?.map(|p| p.puuid), Some("a".into()));
            set_focus_score(c, 1, "a", 0)?;
            set_focus_score(c, 1, "b", 2)?;
            assert!(next_to_scan(c, 1, "DIAMOND", true)?.is_some_and(|p| p.puuid == "b"));
            mark_scanned(c, 1, "b")?;
            assert!(next_to_scan(c, 1, "DIAMOND", true)?.is_none(), "no other focus players");
            // Any player: unchecked and zero-score players both qualify.
            assert!(next_to_scan(c, 1, "DIAMOND", false)?.is_some());
            let counts = collector_counts(c, 1, "DIAMOND")?;
            assert_eq!(
                (counts.players, counts.players_checked, counts.focus_players, counts.players_scanned),
                (3, 2, 1, 1)
            );

            let m: Vec<String> = vec!["NA1_1".into(), "NA1_2".into()];
            assert_eq!(queue_matches(c, &m, "DIAMOND", 0)?, 2);
            assert_eq!(queue_matches(c, &["NA1_3".to_owned()], "DIAMOND", 5)?, 1);
            assert_eq!(next_queued(c)?.map(|q| q.0), Some("NA1_3".into()), "priority first");
            set_queued_status(c, "NA1_3", "done")?;
            assert_eq!(next_queued(c)?.map(|q| q.0), Some("NA1_1".into()));
            assert_eq!(clear_pending(c)?, 2);
            Ok(())
        })
        .expect("ok");
        assert!(patch_order("16.9") < patch_order("16.20"));
        assert!(patch_order("15.24") < patch_order("16.1"));
    }

    #[test]
    fn ranks_round_trip() {
        let db = crate::Db::open_in_memory().expect("db");
        db.with(|c| {
            let a = crate::accounts::upsert_account(c, "Me", "NA1", "na1", "americas", true)?;
            let r = Rank {
                queue_type: "RANKED_SOLO_5x5".into(),
                tier: Some("GOLD".into()),
                division: Some("II".into()),
                lp: 40,
                wins: 10,
                losses: 8,
                fetched_at: 5,
            };
            set_ranks(c, a, std::slice::from_ref(&r))?;
            set_ranks(c, a, std::slice::from_ref(&r))?;
            assert_eq!(ranks(c, a)?, vec![r]);
            assert_eq!(ranks_fetched_at(c, a)?, Some(5));
            Ok(())
        })
        .expect("ok");
    }
}
