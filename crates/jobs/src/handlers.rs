//! Job implementations. Each is idempotent: re-running a finished job only
//! re-checks what's stored.

use rc_riot::client::MatchIdQuery;
use rc_riot::dto::{MatchDto, TimelineDto, patch_of};
use rc_riot::{Platform, RiotError, RiotId};
use rc_store::accounts::{self, PuuidOutcome};
use rc_store::matches::{self, NewMatch, NewParticipant};

use crate::engine::{Engine, EngineEvent, KeyStatus};
use crate::job::Job;

/// Riot caps Match-V5 `ids` at 100 per page.
const MATCH_PAGE: u32 = 100;

#[derive(Debug)]
pub(crate) enum JobError {
    /// Needs a working API key; requeued without burning an attempt.
    Key(KeyStatus),
    /// Transient; retried with backoff.
    Retry(String),
    /// Retrying won't help.
    Permanent(String),
}

impl From<RiotError> for JobError {
    fn from(e: RiotError) -> Self {
        match e {
            RiotError::KeyMissing => Self::Key(KeyStatus::Missing),
            RiotError::KeyRejected { .. } => Self::Key(KeyStatus::Rejected),
            // Usually a match that isn't published yet.
            RiotError::NotFound => Self::Retry("not found (yet)".into()),
            e if e.is_transient() => Self::Retry(e.to_string()),
            e => Self::Permanent(e.to_string()),
        }
    }
}

impl From<rc_store::StoreError> for JobError {
    fn from(e: rc_store::StoreError) -> Self {
        Self::Retry(format!("storage: {e}"))
    }
}

impl From<rc_ddragon::DdragonError> for JobError {
    fn from(e: rc_ddragon::DdragonError) -> Self {
        Self::Retry(format!("Data Dragon: {e}"))
    }
}

type JobResult = Result<(), JobError>;

impl Engine {
    pub(crate) async fn handle(&self, job: &Job) -> JobResult {
        match job {
            Job::ResolveAccount { account_id, then_sync } => self.resolve_account(*account_id, *then_sync).await,
            Job::SyncMatchList { account_id, count } => self.sync_match_list(*account_id, *count).await,
            Job::FetchMatch { match_id } => self.fetch_match(match_id).await,
            Job::FetchTimeline { match_id } => self.fetch_timeline(match_id).await,
            Job::DdragonSync { patch } => self.ddragon_sync(patch).await,
        }
    }

    async fn resolve_account(&self, account_id: i64, then_sync: Option<u32>) -> JobResult {
        let account = self
            .db
            .call(move |c| accounts::get_account(c, account_id))
            .await?
            .ok_or_else(|| JobError::Permanent(format!("account {account_id} no longer exists")))?;
        let platform: Platform = account.platform.parse().map_err(JobError::Permanent)?;
        let riot_id = RiotId::new(&account.game_name, &account.tag_line);

        let dto = match self.riot.account_by_riot_id(platform.account_region(), &riot_id).await {
            Err(RiotError::NotFound) => {
                return Err(JobError::Permanent(format!("Riot ID {riot_id} not found")));
            }
            other => other?,
        };

        let puuid = dto.puuid.clone();
        let outcome = self
            .db
            .call(move |c| {
                if let (Some(name), Some(tag)) = (dto.game_name.as_deref(), dto.tag_line.as_deref()) {
                    accounts::update_riot_id(c, account_id, name, tag)?;
                }
                accounts::record_resolved_puuid(c, account_id, &puuid)
            })
            .await?;

        if let PuuidOutcome::KeyChanged { key_gen_id } = outcome {
            tracing::warn!(key_gen_id, "PUUID changed: API key changed; re-resolving all tracked accounts");
            self.emit(EngineEvent::KeyGenerationChanged { key_gen_id });
            let others: Vec<_> = self
                .db
                .call(|c| accounts::list_accounts(c))
                .await?
                .into_iter()
                .filter(|a| a.id != account_id)
                .map(|a| Job::ResolveAccount { account_id: a.id, then_sync: None })
                .collect();
            self.enqueue_many(others).await?;
        }
        if let Some(count) = then_sync {
            self.enqueue(Job::SyncMatchList { account_id, count }).await?;
        }
        Ok(())
    }

    async fn sync_match_list(&self, account_id: i64, count: u32) -> JobResult {
        let (account, puuid) = self
            .db
            .call(move |c| Ok((accounts::get_account(c, account_id)?, accounts::current_puuid(c, account_id)?)))
            .await?;
        let account = account.ok_or_else(|| JobError::Permanent(format!("account {account_id} no longer exists")))?;
        let Some(puuid) = puuid else {
            // Not resolved under the current key yet.
            self.enqueue(Job::ResolveAccount { account_id, then_sync: Some(count) }).await?;
            return Ok(());
        };
        let platform: Platform = account.platform.parse().map_err(JobError::Permanent)?;

        let mut ids = Vec::new();
        let mut start = 0;
        while start < count {
            let page = MATCH_PAGE.min(count - start);
            let got = self
                .riot
                .match_ids(platform.match_region(), &puuid, &MatchIdQuery { start, count: page, ..Default::default() })
                .await?;
            let done = (got.len() as u32) < page;
            ids.extend(got);
            if done {
                break;
            }
            start += page;
        }

        // Queue what's missing; link matches already stored (e.g. from
        // another tracked account) to this one.
        let total = ids.len();
        let jobs = self
            .db
            .call(move |c| {
                let mut jobs = Vec::new();
                for id in ids {
                    if !matches::has_match(c, &id)? {
                        jobs.push(Job::FetchMatch { match_id: id });
                    } else {
                        matches::link_tracked_accounts(c, &id)?;
                        if !matches::has_timeline(c, &id)? {
                            jobs.push(Job::FetchTimeline { match_id: id });
                        }
                    }
                }
                Ok(jobs)
            })
            .await?;
        let queued = self.enqueue_many(jobs).await?;
        tracing::info!(account_id, listed = total, queued, "match list synced");
        Ok(())
    }

    async fn fetch_match(&self, match_id: &str) -> JobResult {
        let id = match_id.to_owned();
        if self.db.call(move |c| matches::has_match(c, &id)).await? {
            let id = match_id.to_owned();
            let need_timeline = self
                .db
                .call(move |c| {
                    matches::link_tracked_accounts(c, &id)?;
                    Ok(!matches::has_timeline(c, &id)?)
                })
                .await?;
            if need_timeline {
                self.enqueue(Job::FetchTimeline { match_id: match_id.to_owned() }).await?;
            }
            return Ok(());
        }

        let platform = Platform::from_match_id(match_id)
            .ok_or_else(|| JobError::Permanent(format!("can't route match id '{match_id}'")))?;
        let raw = self.riot.match_raw(platform.match_region(), match_id).await?;
        let dto: MatchDto =
            serde_json::from_slice(&raw).map_err(|e| JobError::Permanent(format!("unparseable match: {e}")))?;
        if dto.metadata.match_id != match_id {
            return Err(JobError::Permanent(format!("asked for {match_id}, got {}", dto.metadata.match_id)));
        }

        let (new_match, participants) = to_rows(&dto, platform);
        let patch = new_match.patch.clone();
        let id = match_id.to_owned();
        self.db
            .call(move |c| {
                let key_gen = accounts::latest_key_gen(c)?.ok_or_else(|| {
                    rc_store::StoreError::Io(std::io::Error::other("no key generation yet; resolve an account first"))
                })?;
                matches::insert_match(c, &new_match, &participants, key_gen, &raw)?;
                matches::link_tracked_accounts(c, &id)?;
                Ok(())
            })
            .await?;
        self.emit(EngineEvent::MatchStored { match_id: match_id.to_owned() });
        self.enqueue_many(vec![Job::FetchTimeline { match_id: match_id.to_owned() }, Job::DdragonSync { patch }])
            .await?;
        Ok(())
    }

    async fn fetch_timeline(&self, match_id: &str) -> JobResult {
        let id = match_id.to_owned();
        if self.db.call(move |c| matches::has_timeline(c, &id)).await? {
            return Ok(());
        }
        let platform = Platform::from_match_id(match_id)
            .ok_or_else(|| JobError::Permanent(format!("can't route match id '{match_id}'")))?;
        let raw = self.riot.timeline_raw(platform.match_region(), match_id).await?;
        let dto: TimelineDto =
            serde_json::from_slice(&raw).map_err(|e| JobError::Permanent(format!("unparseable timeline: {e}")))?;
        if dto.metadata.match_id != match_id {
            return Err(JobError::Permanent(format!("asked for {match_id} timeline, got {}", dto.metadata.match_id)));
        }
        let id = match_id.to_owned();
        self.db.call(move |c| matches::insert_timeline(c, &id, &raw)).await?;
        tracing::debug!(match_id, frames = dto.info.frames.len(), "timeline stored");
        self.emit(EngineEvent::TimelineStored { match_id: match_id.to_owned() });
        Ok(())
    }

    async fn ddragon_sync(&self, patch: &str) -> JobResult {
        let version = self.ddragon.version_for_patch(patch).await?;
        self.ddragon.ensure_version(&version).await?;
        Ok(())
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

fn to_rows(dto: &MatchDto, platform: Platform) -> (NewMatch, Vec<NewParticipant>) {
    let info = &dto.info;
    let new_match = NewMatch {
        match_id: dto.metadata.match_id.clone(),
        platform: platform.as_str().to_owned(),
        queue_id: info.queue_id,
        map_id: info.map_id,
        game_mode: info.game_mode.clone(),
        game_type: info.game_type.clone(),
        game_version: info.game_version.clone(),
        patch: patch_of(&info.game_version),
        game_start_ms: info.start_ms(),
        duration_s: info.duration_seconds(),
        is_remake: info.is_remake(),
    };
    let participants = info
        .participants
        .iter()
        .map(|p| NewParticipant {
            participant_id: p.participant_id,
            puuid: p.puuid.clone(),
            riot_id_name: p.riot_id_game_name.clone().filter(|s| !s.is_empty()),
            riot_id_tag: p.riot_id_tagline.clone().filter(|s| !s.is_empty()),
            team_id: p.team_id,
            champion_id: p.champion_id,
            champion_name: p.champion_name.clone(),
            team_position: non_empty(&p.team_position),
            individual_position: non_empty(&p.individual_position),
            win: p.win,
            kills: p.kills,
            deaths: p.deaths,
            assists: p.assists,
            champ_level: p.champ_level,
            gold_earned: p.gold_earned,
            cs: p.total_minions_killed + p.neutral_minions_killed,
            vision_score: p.vision_score,
        })
        .collect();
    (new_match, participants)
}
