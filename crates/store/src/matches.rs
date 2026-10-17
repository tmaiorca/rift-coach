use rc_common::queues::{STANDARD_RIFT_QUEUES, SUMMONERS_RIFT_MAP};
use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{Result, raw};

#[derive(Debug, Clone)]
pub struct NewMatch {
    pub match_id: String,
    pub platform: String,
    pub queue_id: i32,
    pub map_id: i32,
    pub game_mode: String,
    pub game_type: Option<String>,
    pub game_version: String,
    pub patch: String,
    pub game_start_ms: i64,
    pub duration_s: i64,
    pub is_remake: bool,
}

#[derive(Debug, Clone)]
pub struct NewParticipant {
    pub participant_id: i32,
    pub puuid: String,
    pub riot_id_name: Option<String>,
    pub riot_id_tag: Option<String>,
    pub team_id: i32,
    pub champion_id: i32,
    pub champion_name: String,
    pub team_position: Option<String>,
    pub individual_position: Option<String>,
    pub win: bool,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub champ_level: i32,
    pub gold_earned: i32,
    pub cs: i32,
    pub vision_score: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    Match,
    Timeline,
}

impl RawKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Timeline => "timeline",
        }
    }
}

fn put_raw(conn: &Connection, match_id: &str, kind: RawKind, json: &[u8]) -> Result<()> {
    let body = raw::compress(json)?;
    conn.execute(
        "INSERT INTO match_raw (match_id, kind, codec, body, raw_len, fetched_at) VALUES (?1, ?2, 'zstd', ?3, ?4, ?5)
         ON CONFLICT (match_id, kind) DO UPDATE SET
             body = excluded.body, raw_len = excluded.raw_len, fetched_at = excluded.fetched_at",
        params![match_id, kind.as_str(), body, json.len() as i64, now_ms()],
    )?;
    Ok(())
}

/// Stores a match, its participants (PUUIDs tagged with `key_gen_id`), and
/// the raw JSON, in one transaction. Re-inserting the same match replaces
/// its rows but keeps any stored timeline.
pub fn insert_match(
    conn: &mut Connection,
    m: &NewMatch,
    participants: &[NewParticipant],
    key_gen_id: i64,
    raw_json: &[u8],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO matches (match_id, platform, queue_id, map_id, game_mode, game_type, game_version, patch,
                              game_start_ms, duration_s, is_remake, ingested_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT (match_id) DO UPDATE SET
             platform = excluded.platform, queue_id = excluded.queue_id, map_id = excluded.map_id,
             game_mode = excluded.game_mode, game_type = excluded.game_type,
             game_version = excluded.game_version, patch = excluded.patch,
             game_start_ms = excluded.game_start_ms, duration_s = excluded.duration_s,
             is_remake = excluded.is_remake, ingested_at = excluded.ingested_at",
        params![
            m.match_id,
            m.platform,
            m.queue_id,
            m.map_id,
            m.game_mode,
            m.game_type,
            m.game_version,
            m.patch,
            m.game_start_ms,
            m.duration_s,
            m.is_remake,
            now_ms()
        ],
    )?;
    tx.execute("DELETE FROM match_participants WHERE match_id = ?1", [&m.match_id])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO match_participants (match_id, participant_id, puuid, key_gen_id, riot_id_name, riot_id_tag,
                 team_id, champion_id, champion_name, team_position, individual_position, win, kills, deaths, assists,
                 champ_level, gold_earned, cs, vision_score)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        )?;
        for p in participants {
            stmt.execute(params![
                m.match_id,
                p.participant_id,
                p.puuid,
                key_gen_id,
                p.riot_id_name,
                p.riot_id_tag,
                p.team_id,
                p.champion_id,
                p.champion_name,
                p.team_position,
                p.individual_position,
                p.win,
                p.kills,
                p.deaths,
                p.assists,
                p.champ_level,
                p.gold_earned,
                p.cs,
                p.vision_score
            ])?;
        }
    }
    put_raw(&tx, &m.match_id, RawKind::Match, raw_json)?;
    tx.commit()?;
    Ok(())
}

/// Links a match to every tracked account that played in it, by PUUID under
/// the same key generation, or by Riot ID. Returns rows added.
pub fn link_tracked_accounts(conn: &Connection, match_id: &str) -> Result<usize> {
    Ok(conn.execute(
        "INSERT OR IGNORE INTO account_matches (account_id, match_id, participant_id)
         SELECT ap.account_id, mp.match_id, mp.participant_id
           FROM match_participants mp
           JOIN account_puuids ap ON ap.puuid = mp.puuid AND ap.key_gen_id = mp.key_gen_id
          WHERE mp.match_id = ?1
         UNION
         SELECT a.id, mp.match_id, mp.participant_id
           FROM match_participants mp
           JOIN accounts a ON a.game_name = mp.riot_id_name COLLATE NOCASE
                          AND a.tag_line = mp.riot_id_tag COLLATE NOCASE
          WHERE mp.match_id = ?1",
        [match_id],
    )?)
}

pub fn insert_timeline(conn: &mut Connection, match_id: &str, raw_json: &[u8]) -> Result<()> {
    let tx = conn.transaction()?;
    put_raw(&tx, match_id, RawKind::Timeline, raw_json)?;
    tx.execute("UPDATE matches SET has_timeline = 1 WHERE match_id = ?1", [match_id])?;
    tx.commit()?;
    Ok(())
}

pub fn has_match(conn: &Connection, match_id: &str) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM matches WHERE match_id = ?1)", [match_id], |r| r.get(0))?)
}

pub fn has_timeline(conn: &Connection, match_id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT has_timeline FROM matches WHERE match_id = ?1", [match_id], |r| r.get(0))
        .optional()?
        .unwrap_or(false))
}

pub fn patch_of_match(conn: &Connection, match_id: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT patch FROM matches WHERE match_id = ?1", [match_id], |r| r.get(0)).optional()?)
}

/// Decompressed raw JSON, if stored.
pub fn get_raw(conn: &Connection, match_id: &str, kind: RawKind) -> Result<Option<Vec<u8>>> {
    let body: Option<Vec<u8>> = conn
        .query_row(
            "SELECT body FROM match_raw WHERE match_id = ?1 AND kind = ?2",
            params![match_id, kind.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    body.map(|b| raw::decompress(&b).map_err(Into::into)).transpose()
}

/// Matches whose timeline hasn't been stored yet.
pub fn missing_timelines(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT match_id FROM matches WHERE has_timeline = 0 ORDER BY game_start_ms DESC")?;
    let ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// One row of an account's match list (the account's own participant).
#[derive(Debug, Clone, Serialize)]
pub struct MatchSummary {
    pub match_id: String,
    pub queue_id: i32,
    pub map_id: i32,
    pub patch: String,
    pub game_start_ms: i64,
    pub duration_s: i64,
    pub is_remake: bool,
    pub has_timeline: bool,
    pub champion_id: i32,
    pub champion_name: String,
    pub team_position: Option<String>,
    pub win: bool,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    pub cs: i32,
    pub gold_earned: i32,
    pub vision_score: i32,
    pub champ_level: i32,
}

fn summary_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MatchSummary> {
    Ok(MatchSummary {
        match_id: r.get(0)?,
        queue_id: r.get(1)?,
        map_id: r.get(2)?,
        patch: r.get(3)?,
        game_start_ms: r.get(4)?,
        duration_s: r.get(5)?,
        is_remake: r.get(6)?,
        has_timeline: r.get(7)?,
        champion_id: r.get(8)?,
        champion_name: r.get(9)?,
        team_position: r.get(10)?,
        win: r.get(11)?,
        kills: r.get(12)?,
        deaths: r.get(13)?,
        assists: r.get(14)?,
        cs: r.get(15)?,
        gold_earned: r.get(16)?,
        vision_score: r.get(17)?,
        champ_level: r.get(18)?,
    })
}

pub fn list_account_matches(conn: &Connection, account_id: i64, limit: u32, offset: u32) -> Result<Vec<MatchSummary>> {
    let mut stmt = conn.prepare(
        "SELECT m.match_id, m.queue_id, m.map_id, m.patch, m.game_start_ms, m.duration_s, m.is_remake, m.has_timeline,
                p.champion_id, p.champion_name, p.team_position, p.win, p.kills, p.deaths, p.assists, p.cs,
                p.gold_earned, p.vision_score, p.champ_level
           FROM account_matches am
           JOIN matches m ON m.match_id = am.match_id
           JOIN match_participants p ON p.match_id = am.match_id AND p.participant_id = am.participant_id
          WHERE am.account_id = ?1
          ORDER BY m.game_start_ms DESC
          LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(params![account_id, limit, offset], summary_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// The account's row for one match.
pub fn account_match(conn: &Connection, account_id: i64, match_id: &str) -> Result<Option<(MatchSummary, i32)>> {
    Ok(conn
        .query_row(
            "SELECT m.match_id, m.queue_id, m.map_id, m.patch, m.game_start_ms, m.duration_s, m.is_remake,
                    m.has_timeline, p.champion_id, p.champion_name, p.team_position, p.win, p.kills, p.deaths,
                    p.assists, p.cs, p.gold_earned, p.vision_score, p.champ_level, am.participant_id
               FROM account_matches am
               JOIN matches m ON m.match_id = am.match_id
               JOIN match_participants p ON p.match_id = am.match_id AND p.participant_id = am.participant_id
              WHERE am.account_id = ?1 AND am.match_id = ?2",
            params![account_id, match_id],
            |r| Ok((summary_row(r)?, r.get(19)?)),
        )
        .optional()?)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MatchCounts {
    pub matches: i64,
    pub with_timeline: i64,
}

pub fn account_match_counts(conn: &Connection, account_id: i64) -> Result<MatchCounts> {
    Ok(conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(m.has_timeline), 0)
           FROM account_matches am JOIN matches m ON m.match_id = am.match_id
          WHERE am.account_id = ?1",
        [account_id],
        |r| Ok(MatchCounts { matches: r.get(0)?, with_timeline: r.get(1)? }),
    )?)
}

/// Games played per (role, champion) in standard Summoner's Rift queues,
/// excluding remakes. Used to prioritize benchmark sampling.
#[derive(Debug, Clone, Serialize)]
pub struct RoleChampionStat {
    pub team_position: String,
    pub champion_name: String,
    pub games: i64,
    pub wins: i64,
}

pub fn role_champion_stats(conn: &Connection, account_id: i64) -> Result<Vec<RoleChampionStat>> {
    let queues = STANDARD_RIFT_QUEUES.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(NULLIF(p.team_position, ''), 'UNKNOWN'), p.champion_name, COUNT(*), SUM(p.win)
           FROM account_matches am
           JOIN matches m ON m.match_id = am.match_id
           JOIN match_participants p ON p.match_id = am.match_id AND p.participant_id = am.participant_id
          WHERE am.account_id = ?1 AND m.map_id = {SUMMONERS_RIFT_MAP} AND m.queue_id IN ({queues})
            AND m.is_remake = 0
          GROUP BY 1, 2
          ORDER BY 3 DESC, 2"
    ))?;
    let rows = stmt
        .query_map([account_id], |r| {
            Ok(RoleChampionStat {
                team_position: r.get(0)?,
                champion_name: r.get(1)?,
                games: r.get(2)?,
                wins: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use crate::accounts::{record_resolved_puuid, upsert_account};

    pub(crate) fn sample_match(id: &str, queue_id: i32) -> NewMatch {
        NewMatch {
            match_id: id.into(),
            platform: "na1".into(),
            queue_id,
            map_id: 11,
            game_mode: "CLASSIC".into(),
            game_type: Some("MATCHED_GAME".into()),
            game_version: "16.20.712.1".into(),
            patch: "16.20".into(),
            game_start_ms: 1_000,
            duration_s: 1800,
            is_remake: false,
        }
    }

    fn participant(id: i32, puuid: &str, name: &str) -> NewParticipant {
        NewParticipant {
            participant_id: id,
            puuid: puuid.into(),
            riot_id_name: Some(name.into()),
            riot_id_tag: Some("NA1".into()),
            team_id: if id <= 5 { 100 } else { 200 },
            champion_id: 103,
            champion_name: "Ahri".into(),
            team_position: Some("MIDDLE".into()),
            individual_position: Some("MIDDLE".into()),
            win: id <= 5,
            kills: 5,
            deaths: 2,
            assists: 7,
            champ_level: 16,
            gold_earned: 12_000,
            cs: 210,
            vision_score: 20,
        }
    }

    #[test]
    fn stores_and_lists_matches_for_tracked_account() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            let me = upsert_account(c, "Me", "NA1", "na1", "americas", false)?;
            let g = record_resolved_puuid(c, me, "p-me")?.key_gen_id();

            let parts: Vec<_> = (1..=10)
                .map(|i| {
                    if i == 3 {
                        participant(i, "p-me", "Me")
                    } else {
                        participant(i, &format!("p{i}"), &format!("P{i}"))
                    }
                })
                .collect();
            let raw = br#"{"metadata":{"matchId":"NA1_1"}}"#;
            insert_match(c, &sample_match("NA1_1", 420), &parts, g, raw)?;
            assert_eq!(link_tracked_accounts(c, "NA1_1")?, 1);
            // Idempotent.
            insert_match(c, &sample_match("NA1_1", 420), &parts, g, raw)?;
            assert_eq!(link_tracked_accounts(c, "NA1_1")?, 0);

            assert!(has_match(c, "NA1_1")?);
            assert!(!has_timeline(c, "NA1_1")?);
            assert_eq!(missing_timelines(c)?, vec!["NA1_1".to_string()]);
            insert_timeline(c, "NA1_1", b"{\"timeline\":true}")?;
            assert!(has_timeline(c, "NA1_1")?);
            assert_eq!(get_raw(c, "NA1_1", RawKind::Match)?.as_deref(), Some(&raw[..]));
            assert_eq!(get_raw(c, "NA1_1", RawKind::Timeline)?.as_deref(), Some(&b"{\"timeline\":true}"[..]));

            // Re-inserting the match keeps the timeline.
            insert_match(c, &sample_match("NA1_1", 420), &parts, g, raw)?;
            assert!(has_timeline(c, "NA1_1")?);

            let list = list_account_matches(c, me, 10, 0)?;
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].kills, 5);
            assert!(list[0].win);
            let counts = account_match_counts(c, me)?;
            assert_eq!((counts.matches, counts.with_timeline), (1, 1));

            let stats = role_champion_stats(c, me)?;
            assert_eq!(stats.len(), 1);
            assert_eq!((stats[0].team_position.as_str(), stats[0].games, stats[0].wins), ("MIDDLE", 1, 1));
            Ok(())
        })
        .expect("ok");
    }

    #[test]
    fn links_by_riot_id_when_puuid_is_from_another_key() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            let me = upsert_account(c, "Me", "NA1", "na1", "americas", false)?;
            let g = record_resolved_puuid(c, me, "p-me-current")?.key_gen_id();
            // Participant PUUID doesn't match (different key), but Riot ID does.
            let parts = vec![participant(1, "p-me-otherkey", "me")];
            insert_match(c, &sample_match("NA1_2", 450), &parts, g, b"{}")?;
            assert_eq!(link_tracked_accounts(c, "NA1_2")?, 1);
            // ARAM is excluded from role stats.
            assert!(role_champion_stats(c, me)?.is_empty());
            Ok(())
        })
        .expect("ok");
    }
}
