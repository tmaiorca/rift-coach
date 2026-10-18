//! The benchmark collector: samples games of higher-ranked players so your
//! numbers can be compared with a tier, not just your own lobbies.
//!
//! One step at a time (each makes at most two Riot calls), spaced by
//! `step_seconds`, and only while nothing else wants the API: never during a
//! game, never while your own jobs are queued, never with a key problem.
//! Every step's state is in the database, so it resumes after a restart.
//!
//! Steps, in order of need:
//! 1. seed: read the tier's ladder (League-V4) into `bench_players`;
//! 2. fetch: download a queued match (and timeline), scrub other players'
//!    names, store it as a benchmark game, and queue its metrics;
//! 3. scan a player known to main a focus champion;
//! 4. check: read a player's top champion masteries to find such players
//!    (alternating with step 5 while role samples are also short);
//! 5. scan: list any player's recent solo queue games into `bench_queue`.

use std::sync::Arc;
use std::time::Duration;

use rc_common::time::now_ms;
use rc_riot::client::MatchIdQuery;
use rc_riot::dto::{MatchDto, patch_of};
use rc_riot::{Platform, RiotError};
use rc_store::bench::{self, patch_order};
use rc_store::matches;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::watch;

use crate::engine::{Engine, KeyStatus};
use crate::job::Job;

const SOLO_QUEUE: &str = "RANKED_SOLO_5x5";
const SOLO_QUEUE_ID: i32 = 420;
/// Re-read the ladder this often.
const RESEED_AFTER_MS: i64 = 7 * 24 * 3600 * 1000;
/// Only games this recent are listed (keeps samples on current patches).
const LOOKBACK_S: i64 = 14 * 24 * 3600;
const GAMES_PER_PLAYER: u32 = 20;
/// Mastery checks per seeding (each is one call).
const MAX_CHECKS: i64 = 400;
/// Benchmark patches kept on disk.
const KEEP_PATCHES: usize = 2;
const WAIT: Duration = Duration::from_secs(30);
const DONE_RECHECK: Duration = Duration::from_secs(10 * 60);
const ERROR_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchPhase {
    #[default]
    Starting,
    Disabled,
    /// Holding back: game running, your jobs queued, or a key problem.
    Waiting,
    Collecting,
    /// Targets met (or the ladder sample is used up) for the current patch.
    Done,
    /// No tracked account or games yet to decide what to collect.
    NothingToDo,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BenchState {
    pub phase: BenchPhase,
    /// What the last step did, e.g. "Fetched NA1_123 (patch 16.20)".
    pub detail: String,
    pub last_error: Option<String>,
    pub fetched_this_run: u32,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct BenchConfig {
    /// Overrides the settings' step spacing and the waits (tests).
    pub fast: Option<Duration>,
}

enum Outcome {
    /// Did something; wait the step spacing.
    Stepped(String),
    Wait(BenchPhase, String),
}

struct Collector {
    engine: Arc<Engine>,
    config: BenchConfig,
    tx: watch::Sender<BenchState>,
    fetched: u32,
    steps: u64,
}

pub fn spawn_benchmarks(engine: Arc<Engine>, config: BenchConfig) -> watch::Receiver<BenchState> {
    let (tx, rx) = watch::channel(BenchState::default());
    tokio::spawn(Collector { engine, config, tx, fetched: 0, steps: 0 }.run());
    rx
}

impl Collector {
    async fn run(mut self) {
        loop {
            let step_every =
                self.engine.db.call(|c| rc_store::settings::app(c)).await.map_or(5, |s| s.benchmarks.step_seconds);
            let (phase, detail, error, wait) = match self.step().await {
                Ok(Outcome::Stepped(d)) => {
                    (BenchPhase::Collecting, d, None, Duration::from_secs(u64::from(step_every.max(2))))
                }
                Ok(Outcome::Wait(p, d)) => {
                    let w = if p == BenchPhase::Done { DONE_RECHECK } else { WAIT };
                    (p, d, None, w)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "benchmark step failed");
                    (BenchPhase::Waiting, "Paused after an error".into(), Some(e), ERROR_WAIT)
                }
            };
            self.tx.send_replace(BenchState {
                phase,
                detail,
                last_error: error,
                fetched_this_run: self.fetched,
                updated_ms: now_ms(),
            });
            tokio::time::sleep(self.config.fast.map_or(wait, |f| f.min(wait))).await;
        }
    }

    fn riot_error(&self, e: RiotError) -> String {
        match e {
            RiotError::KeyMissing => {
                self.engine.set_key_status(KeyStatus::Missing);
                "no Riot API key".into()
            }
            RiotError::KeyRejected { .. } => {
                self.engine.set_key_status(KeyStatus::Rejected);
                "Riot API key rejected".into()
            }
            e => e.to_string(),
        }
    }

    async fn step(&mut self) -> Result<Outcome, String> {
        let db = &self.engine.db;
        let settings = db.call(|c| rc_store::settings::app(c)).await.map_err(|e| e.to_string())?;
        if !settings.benchmarks.enabled {
            return Ok(Outcome::Wait(BenchPhase::Disabled, "Turned off in Settings".into()));
        }
        if self.engine.in_game() {
            return Ok(Outcome::Wait(BenchPhase::Waiting, "A game is running".into()));
        }
        if matches!(self.engine.key_status(), KeyStatus::Missing | KeyStatus::Rejected) {
            return Ok(Outcome::Wait(BenchPhase::Waiting, "Waiting for a working Riot API key".into()));
        }
        let busy = db.call(|c| rc_store::jobs::counts(c, now_ms())).await.map_err(|e| e.to_string())?;
        if busy.ready + busy.running > 0 {
            return Ok(Outcome::Wait(BenchPhase::Waiting, "Your own downloads go first".into()));
        }

        let tier = settings.benchmarks.tier.clone();
        let s = settings.clone();
        let (account, key_gen, progress) = db
            .call(move |c| {
                let account = match s.active_account_id {
                    Some(a) => rc_store::accounts::get_account(c, a)?,
                    None => rc_store::accounts::primary_account(c)?,
                };
                let key_gen = rc_store::accounts::latest_key_gen(c)?;
                let progress = rc_coach::benchmarks::progress(c, account.as_ref().map(|a| a.id), &s)?;
                Ok((account, key_gen, progress))
            })
            .await
            .map_err(|e| e.to_string())?;
        let (Some(account), Some(key_gen), Some(patch)) = (account, key_gen, progress.patch.clone()) else {
            return Ok(Outcome::Wait(BenchPhase::NothingToDo, "Add your Riot ID and sync your games first".into()));
        };
        if progress.focus.roles.is_empty() {
            return Ok(Outcome::Wait(BenchPhase::NothingToDo, "No Summoner's Rift games yet".into()));
        }
        let platform: Platform = account.platform.parse()?;

        // 1. Seed.
        let t = tier.clone();
        let seeded = db.call(move |c| bench::seeded_at(c, key_gen, &t)).await.map_err(|e| e.to_string())?;
        if seeded.is_none_or(|at| now_ms() - at > RESEED_AFTER_MS) {
            return self.seed(platform, key_gen, &tier).await;
        }
        if progress.complete() {
            return Ok(Outcome::Wait(BenchPhase::Done, format!("Targets met for patch {patch}")));
        }

        // 2. Fetch what's queued.
        if let Some((match_id, qtier)) = db.call(|c| bench::next_queued(c)).await.map_err(|e| e.to_string())? {
            return self.fetch(&match_id, &qtier, &patch, settings.benchmarks.max_games).await;
        }

        self.steps += 1;
        let needs_champ = progress.needs_champion();
        // 3. Players known to main a focus champion first.
        let t = tier.clone();
        let focus_player =
            db.call(move |c| bench::next_to_scan(c, key_gen, &t, true)).await.map_err(|e| e.to_string())?;
        if let (true, Some(p)) = (needs_champ, focus_player) {
            return self.scan(key_gen, &tier, p).await;
        }
        // 4. Look for such players, alternating with plain scans while the
        //    role sample is short too.
        let check_turn = !progress.needs_role() || !self.steps.is_multiple_of(2);
        if needs_champ && check_turn && progress.collector.players_checked < MAX_CHECKS {
            let t = tier.clone();
            if let Some(p) = db.call(move |c| bench::next_unchecked(c, key_gen, &t)).await.map_err(|e| e.to_string())? {
                let champs = progress.focus.champions.clone();
                let ids = db
                    .call(move |c| rc_coach::benchmarks::champion_ids(c, &champs))
                    .await
                    .map_err(|e| e.to_string())?;
                let pf: Platform = p.platform.parse()?;
                let top = self.engine.riot.mastery_top(pf, &p.puuid, 10).await.map_err(|e| self.riot_error(e))?;
                let score =
                    i32::try_from(top.iter().filter(|m| ids.contains(&(m.champion_id as i32))).count()).unwrap_or(0);
                db.call(move |c| bench::set_focus_score(c, key_gen, &p.puuid, score))
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(Outcome::Stepped(if score > 0 {
                    "Found a player who mains a focus champion".into()
                } else {
                    "Checked a player's champions".into()
                }));
            }
        }

        // 5. List any player's recent games.
        let t = tier.clone();
        let next = db.call(move |c| bench::next_to_scan(c, key_gen, &t, false)).await.map_err(|e| e.to_string())?;
        let Some(p) = next else {
            return Ok(Outcome::Wait(
                BenchPhase::Done,
                "Used up this week's ladder sample; more after the next refresh".into(),
            ));
        };
        self.scan(key_gen, &tier, p).await
    }

    async fn scan(&self, key_gen: i64, tier: &str, p: bench::BenchPlayer) -> Result<Outcome, String> {
        let db = &self.engine.db;
        let pf: Platform = p.platform.parse()?;
        let query = MatchIdQuery {
            start: 0,
            count: GAMES_PER_PLAYER,
            queue: Some(SOLO_QUEUE_ID),
            start_time_s: Some(now_ms() / 1000 - LOOKBACK_S),
            end_time_s: None,
        };
        let ids =
            self.engine.riot.match_ids(pf.match_region(), &p.puuid, &query).await.map_err(|e| self.riot_error(e))?;
        let (t, priority) = (tier.to_owned(), p.focus_score.unwrap_or(0));
        let added = db
            .call(move |c| {
                let added = bench::queue_matches(c, &ids, &t, priority)?;
                bench::mark_scanned(c, key_gen, &p.puuid)?;
                Ok(added)
            })
            .await
            .map_err(|e| e.to_string())?;
        Ok(Outcome::Stepped(format!("Listed a player's recent games ({added} new)")))
    }

    async fn seed(&self, platform: Platform, key_gen: i64, tier: &str) -> Result<Outcome, String> {
        let riot = &self.engine.riot;
        let mut puuids: Vec<String> = Vec::new();
        if tier == "MASTER" {
            for t in ["CHALLENGER", "GRANDMASTER", "MASTER"] {
                let list = riot.league_apex(platform, t, SOLO_QUEUE).await.map_err(|e| self.riot_error(e))?;
                puuids.extend(list.entries.into_iter().filter_map(|e| e.puuid));
            }
        } else {
            for division in ["I", "II"] {
                let page = riot
                    .league_entries_by_tier(platform, SOLO_QUEUE, tier, division, 1)
                    .await
                    .map_err(|e| self.riot_error(e))?;
                puuids.extend(page.into_iter().filter_map(|e| e.puuid));
            }
        }
        let n = puuids.len();
        let (t, pf) = (tier.to_owned(), platform.as_str().to_owned());
        let added = self
            .engine
            .db
            .call(move |c| {
                bench::clear_pending(c)?;
                bench::add_players(c, key_gen, &pf, &t, &puuids)
            })
            .await
            .map_err(|e| e.to_string())?;
        tracing::info!(tier, players = n, added, "benchmark ladder sampled");
        Ok(Outcome::Stepped(format!("Sampled {n} {} players from the ladder", rc_coach::benchmarks::tier_label(tier))))
    }

    async fn fetch(&mut self, match_id: &str, tier: &str, patch: &str, max_games: u32) -> Result<Outcome, String> {
        let db = &self.engine.db;
        let id = match_id.to_owned();
        let finish = |status: &'static str| {
            let id = match_id.to_owned();
            async move { db.call(move |c| bench::set_queued_status(c, &id, status)).await.map_err(|e| e.to_string()) }
        };
        if db.call(move |c| matches::has_match(c, &id)).await.map_err(|e| e.to_string())? {
            finish("done").await?;
            return Ok(Outcome::Stepped("Already stored".into()));
        }
        let Some(platform) = Platform::from_match_id(match_id) else {
            finish("skipped").await?;
            return Ok(Outcome::Stepped("Skipped an unroutable match id".into()));
        };
        let raw = match self.engine.riot.match_raw(platform.match_region(), match_id).await {
            Ok(r) => r,
            Err(RiotError::NotFound) => {
                finish("skipped").await?;
                return Ok(Outcome::Stepped("Skipped a missing match".into()));
            }
            Err(e) => return Err(self.riot_error(e)),
        };
        let mut value: Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        let dto: MatchDto = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        let game_patch = patch_of(&dto.info.game_version);
        if patch_order(&game_patch) < patch_order(patch) || dto.info.is_remake() {
            finish("skipped").await?;
            return Ok(Outcome::Stepped(format!("Skipped a game from patch {game_patch}")));
        }
        scrub_names(&mut value);
        let scrubbed = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        let (mut new_match, mut participants) = crate::handlers::to_rows(&dto, platform);
        for p in &mut participants {
            p.riot_id_name = None;
            p.riot_id_tag = None;
        }
        new_match.patch = game_patch.clone();
        let (id, t) = (match_id.to_owned(), tier.to_owned());
        db.call(move |c| {
            let key_gen = rc_store::accounts::latest_key_gen(c)?.unwrap_or(0);
            matches::insert_match(c, &new_match, &participants, key_gen, &scrubbed)?;
            // A tracked player in it makes it one of "your" games instead.
            if matches::link_tracked_accounts(c, &id)? == 0 {
                bench::set_benchmark_tier(c, &id, &t)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())?;
        match self.engine.riot.timeline_raw(platform.match_region(), match_id).await {
            Ok(tl) => {
                let id = match_id.to_owned();
                db.call(move |c| matches::insert_timeline(c, &id, &tl)).await.map_err(|e| e.to_string())?;
            }
            Err(RiotError::NotFound) => {}
            Err(e) => return Err(self.riot_error(e)),
        }
        finish("done").await?;
        self.engine.enqueue(Job::ComputeMetrics { match_id: match_id.to_owned() }).await.map_err(|e| e.to_string())?;
        self.fetched += 1;
        if self.fetched.is_multiple_of(50) {
            let pruned = db.call(move |c| bench::prune(c, KEEP_PATCHES, max_games)).await.map_err(|e| e.to_string())?;
            if pruned > 0 {
                tracing::info!(pruned, "pruned old benchmark games");
            }
        }
        Ok(Outcome::Stepped(format!("Fetched a game (patch {game_patch})")))
    }
}

/// Removes other players' names from a benchmark match before it's stored
/// (metrics don't need them).
fn scrub_names(v: &mut Value) {
    if let Some(ps) = v.pointer_mut("/info/participants").and_then(Value::as_array_mut) {
        for p in ps {
            if let Some(o) = p.as_object_mut() {
                for k in ["riotIdGameName", "riotIdTagline", "summonerName", "summonerId", "profileIcon"] {
                    o.remove(k);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_identities_but_keeps_stats() {
        let mut v = serde_json::json!({"info": {"participants": [
            {"riotIdGameName": "Someone", "riotIdTagline": "NA1", "summonerName": "x", "kills": 3, "puuid": "p"}
        ]}});
        scrub_names(&mut v);
        let p = &v["info"]["participants"][0];
        assert!(p.get("riotIdGameName").is_none() && p.get("summonerName").is_none());
        assert_eq!(p["kills"], 3);
    }
}
