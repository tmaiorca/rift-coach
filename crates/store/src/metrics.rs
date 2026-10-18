//! Derived per-participant metrics and the populations used for
//! percentiles. Rows are rebuildable from `match_raw`.

use std::collections::{BTreeMap, HashMap};

use rc_common::queues::{STANDARD_RIFT_QUEUES, SUMMONERS_RIFT_MAP};
use rc_common::time::now_ms;
use rusqlite::{Connection, params};

use crate::Result;

pub(crate) fn rift_filter() -> String {
    let queues = STANDARD_RIFT_QUEUES.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    format!("m.map_id = {SUMMONERS_RIFT_MAP} AND m.queue_id IN ({queues}) AND m.is_remake = 0")
}

/// One champion death with its context (see `death_events` in migration 003).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DeathRow {
    pub participant_id: i32,
    pub t_ms: i64,
    pub x: i32,
    pub y: i32,
    pub zone: String,
    pub enemies: i32,
    pub solo: bool,
    pub before_14: bool,
    pub enemy_side: bool,
    pub gold_vs_opponent: Option<i32>,
    pub unspent_gold: Option<i32>,
    pub shutdown_bounty: i32,
    pub enemy_objective: Option<String>,
    pub killer_champion: Option<String>,
}

/// Replaces a match's metrics and death events and records `version`.
pub fn save_match_metrics(
    conn: &mut Connection,
    match_id: &str,
    version: i64,
    by_participant: &BTreeMap<i32, BTreeMap<String, f64>>,
    deaths: &[DeathRow],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM participant_metrics WHERE match_id = ?1", [match_id])?;
    tx.execute("DELETE FROM death_events WHERE match_id = ?1", [match_id])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO participant_metrics (match_id, participant_id, metric, value) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (pid, metrics) in by_participant {
            for (k, v) in metrics {
                stmt.execute(params![match_id, pid, k, v])?;
            }
        }
        let mut stmt = tx.prepare(
            "INSERT INTO death_events (match_id, participant_id, seq, t_ms, x, y, zone, enemies, solo, before_14,
                                       enemy_side, gold_vs_opponent, unspent_gold, shutdown_bounty, enemy_objective,
                                       killer_champion)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        )?;
        let mut seq: BTreeMap<i32, i32> = BTreeMap::new();
        for d in deaths {
            let n = seq.entry(d.participant_id).or_default();
            *n += 1;
            stmt.execute(params![
                match_id,
                d.participant_id,
                *n,
                d.t_ms,
                d.x,
                d.y,
                d.zone,
                d.enemies,
                d.solo,
                d.before_14,
                d.enemy_side,
                d.gold_vs_opponent,
                d.unspent_gold,
                d.shutdown_bounty,
                d.enemy_objective,
                d.killer_champion
            ])?;
        }
    }
    tx.execute(
        "INSERT INTO match_metrics_state (match_id, version, computed_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (match_id) DO UPDATE SET version = excluded.version, computed_at = excluded.computed_at",
        params![match_id, version, now_ms()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Stored matches whose metrics are missing, older than `version`, or were
/// computed before the timeline arrived.
pub fn stale_matches(conn: &Connection, version: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT m.match_id FROM matches m
           LEFT JOIN match_metrics_state s ON s.match_id = m.match_id
          WHERE s.match_id IS NULL OR s.version < ?1
             OR (m.has_timeline = 1 AND s.computed_at < (SELECT fetched_at FROM match_raw r
                                                        WHERE r.match_id = m.match_id AND r.kind = 'timeline'))
          ORDER BY m.game_start_ms DESC",
    )?;
    let ids = stmt.query_map([version], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

pub fn participant_metrics(conn: &Connection, match_id: &str, participant_id: i32) -> Result<BTreeMap<String, f64>> {
    let mut stmt =
        conn.prepare("SELECT metric, value FROM participant_metrics WHERE match_id = ?1 AND participant_id = ?2")?;
    let rows = stmt
        .query_map(params![match_id, participant_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn participant_deaths(conn: &Connection, match_id: &str, participant_id: i32) -> Result<Vec<DeathRow>> {
    let mut stmt = conn.prepare(
        "SELECT participant_id, t_ms, x, y, zone, enemies, solo, before_14, enemy_side, gold_vs_opponent,
                unspent_gold, shutdown_bounty, enemy_objective, killer_champion
           FROM death_events WHERE match_id = ?1 AND participant_id = ?2 ORDER BY seq",
    )?;
    let rows = stmt
        .query_map(params![match_id, participant_id], |r| {
            Ok(DeathRow {
                participant_id: r.get(0)?,
                t_ms: r.get(1)?,
                x: r.get(2)?,
                y: r.get(3)?,
                zone: r.get(4)?,
                enemies: r.get(5)?,
                solo: r.get(6)?,
                before_14: r.get(7)?,
                enemy_side: r.get(8)?,
                gold_vs_opponent: r.get(9)?,
                unspent_gold: r.get(10)?,
                shutdown_bounty: r.get(11)?,
                enemy_objective: r.get(12)?,
                killer_champion: r.get(13)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Where other players in `role` die in the account's stored games:
/// zone → deaths (the account's own rows excluded).
pub fn role_death_zones(conn: &Connection, account_id: i64, role: &str) -> Result<BTreeMap<String, u32>> {
    let sql = format!(
        "SELECT d.zone, COUNT(*)
           FROM death_events d
           JOIN match_participants p ON p.match_id = d.match_id AND p.participant_id = d.participant_id
           JOIN matches m ON m.match_id = d.match_id
          WHERE {} AND m.benchmark_tier IS NULL AND p.team_position = ?2
            AND NOT EXISTS (SELECT 1 FROM account_matches am
                             WHERE am.account_id = ?1 AND am.match_id = p.match_id
                               AND am.participant_id = p.participant_id)
          GROUP BY d.zone",
        rift_filter()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows =
        stmt.query_map(params![account_id, role], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub type Population = HashMap<String, Vec<f64>>;

/// Other players' metrics in the same role across stored Summoner's Rift
/// games (the account's own rows and benchmark games excluded). `champion`
/// narrows it further.
pub fn role_population(
    conn: &Connection,
    account_id: i64,
    role: &str,
    champion_name: Option<&str>,
) -> Result<Population> {
    let sql = format!(
        "SELECT pm.metric, pm.value
           FROM participant_metrics pm
           JOIN match_participants p ON p.match_id = pm.match_id AND p.participant_id = pm.participant_id
           JOIN matches m ON m.match_id = pm.match_id
          WHERE {} AND m.benchmark_tier IS NULL AND p.team_position = ?2 AND (?3 IS NULL OR p.champion_name = ?3)
            AND NOT EXISTS (SELECT 1 FROM account_matches am
                             WHERE am.account_id = ?1 AND am.match_id = p.match_id
                               AND am.participant_id = p.participant_id)",
        rift_filter()
    );
    collect(conn, &sql, params![account_id, role, champion_name])
}

/// The account's own metrics over its newest `games` Summoner's Rift games
/// in `role` (optionally on one champion), started before `before_ms`.
pub fn personal_population(
    conn: &Connection,
    account_id: i64,
    role: &str,
    champion_name: Option<&str>,
    games: u32,
    before_ms: i64,
) -> Result<Population> {
    let sql = format!(
        "WITH recent AS (
             SELECT am.match_id, am.participant_id
               FROM account_matches am
               JOIN matches m ON m.match_id = am.match_id
               JOIN match_participants p ON p.match_id = am.match_id AND p.participant_id = am.participant_id
              WHERE am.account_id = ?1 AND {} AND p.team_position = ?2
                AND (?3 IS NULL OR p.champion_name = ?3) AND m.game_start_ms < ?5
              ORDER BY m.game_start_ms DESC
              LIMIT ?4)
         SELECT pm.metric, pm.value FROM participant_metrics pm
           JOIN recent r ON r.match_id = pm.match_id AND r.participant_id = pm.participant_id",
        rift_filter()
    );
    collect(conn, &sql, params![account_id, role, champion_name, games, before_ms])
}

fn collect(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<Population> {
    let mut stmt = conn.prepare(sql)?;
    let mut out: Population = HashMap::new();
    let mut rows = stmt.query(p)?;
    while let Some(r) = rows.next()? {
        out.entry(r.get(0)?).or_default().push(r.get(1)?);
    }
    Ok(out)
}

/// One game of the account's recent history with its metrics (for trends
/// and baselines on the dashboard).
#[derive(Debug, Clone, serde::Serialize)]
pub struct GameMetrics {
    pub match_id: String,
    pub participant_id: i32,
    pub game_start_ms: i64,
    pub duration_s: i64,
    pub champion_name: String,
    pub champion_id: i32,
    pub team_position: Option<String>,
    pub win: bool,
    pub metrics: BTreeMap<String, f64>,
}

/// Which of the account's Summoner's Rift games to load.
#[derive(Debug, Clone, Default)]
pub struct GameFilter {
    /// Newest N games (after the other filters).
    pub limit: u32,
    /// Only games that started at or after this time.
    pub since_ms: Option<i64>,
    /// Only games that started before this time.
    pub before_ms: Option<i64>,
    pub role: Option<String>,
    pub champion_name: Option<String>,
}

/// The account's newest `games` Summoner's Rift games with metrics.
pub fn recent_game_metrics(conn: &Connection, account_id: i64, games: u32) -> Result<Vec<GameMetrics>> {
    game_metrics(conn, account_id, &GameFilter { limit: games, ..Default::default() })
}

/// The account's Summoner's Rift games matching `f`, newest first.
pub fn game_metrics(conn: &Connection, account_id: i64, f: &GameFilter) -> Result<Vec<GameMetrics>> {
    let sql = format!(
        "SELECT m.match_id, m.game_start_ms, m.duration_s, p.champion_name, p.champion_id, p.team_position, p.win,
                p.participant_id
           FROM account_matches am
           JOIN matches m ON m.match_id = am.match_id
           JOIN match_participants p ON p.match_id = am.match_id AND p.participant_id = am.participant_id
          WHERE am.account_id = ?1 AND {}
            AND (?3 IS NULL OR m.game_start_ms >= ?3) AND (?4 IS NULL OR m.game_start_ms < ?4)
            AND (?5 IS NULL OR p.team_position = ?5) AND (?6 IS NULL OR p.champion_name = ?6)
          ORDER BY m.game_start_ms DESC
          LIMIT ?2",
        rift_filter()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<GameMetrics> = stmt
        .query_map(params![account_id, f.limit, f.since_ms, f.before_ms, f.role, f.champion_name], |r| {
            Ok(GameMetrics {
                match_id: r.get(0)?,
                game_start_ms: r.get(1)?,
                duration_s: r.get(2)?,
                champion_name: r.get(3)?,
                champion_id: r.get(4)?,
                team_position: r.get(5)?,
                win: r.get(6)?,
                participant_id: r.get(7)?,
                metrics: BTreeMap::new(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|mut g| {
            g.metrics = participant_metrics(conn, &g.match_id, g.participant_id)?;
            Ok(g)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use crate::accounts::{record_resolved_puuid, upsert_account};
    use crate::matches::{NewMatch, NewParticipant, insert_match, link_tracked_accounts};

    fn participant(id: i32, puuid: &str, name: &str) -> NewParticipant {
        NewParticipant {
            participant_id: id,
            puuid: puuid.into(),
            riot_id_name: Some(name.into()),
            riot_id_tag: Some("NA1".into()),
            team_id: if id <= 5 { 100 } else { 200 },
            champion_id: 7,
            champion_name: "Leblanc".into(),
            team_position: Some(if id == 3 || id == 8 { "MIDDLE" } else { "TOP" }.into()),
            individual_position: None,
            win: id <= 5,
            kills: 1,
            deaths: 1,
            assists: 1,
            champ_level: 10,
            gold_earned: 1,
            cs: 1,
            vision_score: 1,
        }
    }

    #[test]
    fn populations_exclude_own_games_and_respect_role() {
        let db = Db::open_in_memory().expect("db");
        db.with(|c| {
            let me = upsert_account(c, "Me", "NA1", "na1", "americas", true)?;
            let g = record_resolved_puuid(c, me, "p-me")?.key_gen_id();
            for (i, id) in ["NA1_1", "NA1_2"].iter().enumerate() {
                let parts: Vec<_> = (1..=10)
                    .map(|n| if n == 3 { participant(n, "p-me", "Me") } else { participant(n, &format!("p{n}"), "X") })
                    .collect();
                let m = NewMatch {
                    match_id: (*id).into(),
                    platform: "na1".into(),
                    queue_id: 420,
                    map_id: 11,
                    game_mode: "CLASSIC".into(),
                    game_type: None,
                    game_version: "16.20.1".into(),
                    patch: "16.20".into(),
                    game_start_ms: 1000 * (i as i64 + 1),
                    duration_s: 1800,
                    is_remake: false,
                };
                insert_match(c, &m, &parts, g, b"{}")?;
                link_tracked_accounts(c, id)?;
                let metrics: BTreeMap<i32, BTreeMap<String, f64>> = (1..=10)
                    .map(|n| (n, BTreeMap::from([("cs_10".to_string(), f64::from(n * 10 + i as i32))])))
                    .collect();
                let deaths = vec![DeathRow {
                    participant_id: 8,
                    t_ms: 60_000,
                    x: 100,
                    y: 100,
                    zone: "own_base".into(),
                    enemies: 1,
                    solo: true,
                    before_14: true,
                    enemy_side: false,
                    gold_vs_opponent: Some(-5),
                    unspent_gold: None,
                    shutdown_bounty: 0,
                    enemy_objective: None,
                    killer_champion: Some("Leblanc".into()),
                }];
                save_match_metrics(c, id, 1, &metrics, &deaths)?;
            }
            assert!(stale_matches(c, 1)?.is_empty());
            assert_eq!(stale_matches(c, 2)?.len(), 2);

            let pop = role_population(c, me, "MIDDLE", None)?;
            // Only participant 8 (the enemy mid) in each game; never mine (3).
            let mut cs = pop["cs_10"].clone();
            cs.sort_by(f64::total_cmp);
            assert_eq!(cs, vec![80.0, 81.0]);

            let mine = personal_population(c, me, "MIDDLE", Some("Leblanc"), 1, i64::MAX)?;
            assert_eq!(mine["cs_10"], vec![31.0], "newest game only");

            let recent = recent_game_metrics(c, me, 10)?;
            assert_eq!(recent.len(), 2);
            assert_eq!(recent[0].metrics["cs_10"], 31.0);
            let older = game_metrics(c, me, &GameFilter { limit: 10, before_ms: Some(2000), ..Default::default() })?;
            assert_eq!(older.len(), 1);
            assert_eq!(older[0].match_id, "NA1_1");

            // The enemy mid's deaths form the role's death-zone population.
            assert_eq!(role_death_zones(c, me, "MIDDLE")?, BTreeMap::from([("own_base".to_string(), 2)]));
            assert_eq!(participant_deaths(c, "NA1_1", 8)?.len(), 1);
            assert!(participant_deaths(c, "NA1_1", 3)?.is_empty());
            Ok(())
        })
        .expect("ok");
    }
}
