use serde::{Deserialize, Serialize};

/// Every unit of background work. Serialized as the job row's payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Job {
    /// Riot ID → PUUID under the current key (detects key changes). Then
    /// optionally syncs the newest `then_sync` matches.
    ResolveAccount {
        account_id: i64,
        then_sync: Option<u32>,
    },
    /// Lists the account's newest `count` match IDs and queues missing ones.
    SyncMatchList {
        account_id: i64,
        count: u32,
    },
    FetchMatch {
        match_id: String,
    },
    FetchTimeline {
        match_id: String,
    },
    /// Cache Data Dragon static data for a game patch (e.g. "16.20").
    DdragonSync {
        patch: String,
    },
    /// (Re)compute metrics for all ten participants from stored raw data.
    ComputeMetrics {
        match_id: String,
    },
    /// Automatic post-game AI review (only queued for fresh games when the
    /// setting is on; on-demand reviews run directly from the UI).
    AiReview {
        account_id: i64,
        match_id: String,
    },
    /// Automatic AI review of a finished play session (`scope` from
    /// `rc_coach::sessions::scope_of`).
    SessionReview {
        account_id: i64,
        scope: String,
    },
    /// After a live game ends: find it in Match-V5 (it takes a few minutes
    /// to appear), link it, and fetch it. Retries until Riot publishes it.
    PostGame {
        account_id: i64,
        live_game_id: i64,
    },
    /// Ranked standing (League-V4) of a tracked account.
    FetchRank {
        account_id: i64,
    },
    /// AI weekly report for the week starting `week` (local date, YYYY-MM-DD).
    WeeklyReport {
        account_id: i64,
        week: String,
    },
    /// Vision review of a live game's screenshots (after it is analyzed).
    /// Deletes the frames once the review is stored.
    VisionReview {
        account_id: i64,
        live_game_id: i64,
    },
}

impl Job {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ResolveAccount { .. } => "resolve_account",
            Self::SyncMatchList { .. } => "sync_match_list",
            Self::FetchMatch { .. } => "fetch_match",
            Self::FetchTimeline { .. } => "fetch_timeline",
            Self::DdragonSync { .. } => "ddragon_sync",
            Self::ComputeMetrics { .. } => "compute_metrics",
            Self::AiReview { .. } => "ai_review",
            Self::SessionReview { .. } => "session_review",
            Self::PostGame { .. } => "post_game",
            Self::FetchRank { .. } => "fetch_rank",
            Self::WeeklyReport { .. } => "weekly_report",
            Self::VisionReview { .. } => "vision_review",
        }
    }

    /// At most one active job per key.
    pub fn dedupe_key(&self) -> String {
        match self {
            Self::ResolveAccount { account_id, .. } => format!("resolve:{account_id}"),
            Self::SyncMatchList { account_id, .. } => format!("sync:{account_id}"),
            Self::FetchMatch { match_id } => format!("match:{match_id}"),
            Self::FetchTimeline { match_id } => format!("timeline:{match_id}"),
            Self::DdragonSync { patch } => format!("ddragon:{patch}"),
            Self::ComputeMetrics { match_id } => format!("metrics:{match_id}"),
            Self::AiReview { account_id, match_id } => format!("review:{account_id}:{match_id}"),
            Self::SessionReview { account_id, scope } => format!("session_review:{account_id}:{scope}"),
            Self::PostGame { live_game_id, .. } => format!("post_game:{live_game_id}"),
            Self::FetchRank { account_id } => format!("rank:{account_id}"),
            Self::WeeklyReport { account_id, week } => format!("weekly:{account_id}:{week}"),
            Self::VisionReview { live_game_id, .. } => format!("vision:{live_game_id}"),
        }
    }

    /// Higher runs first. Matches before timelines, so the match list fills
    /// quickly and timelines follow.
    pub fn priority(&self) -> i32 {
        match self {
            // The game you just played comes before any backfill.
            Self::PostGame { .. } => 110,
            Self::ResolveAccount { .. } => 100,
            Self::FetchRank { .. } => 85,
            Self::SyncMatchList { .. } => 90,
            Self::DdragonSync { .. } => 80,
            // Local and cheap: keeps the UI's numbers current while fetching.
            Self::ComputeMetrics { .. } => 70,
            Self::FetchMatch { .. } => 50,
            Self::FetchTimeline { .. } => 40,
            Self::AiReview { .. }
            | Self::SessionReview { .. }
            | Self::WeeklyReport { .. }
            | Self::VisionReview { .. } => 20,
        }
    }

    pub fn max_attempts(&self) -> i32 {
        match self {
            // A just-finished match can take several minutes to appear.
            Self::FetchMatch { .. } | Self::FetchTimeline { .. } => 8,
            // With the default backoff this keeps looking for about 1.5 hours.
            Self::PostGame { .. } => 10,
            Self::AiReview { .. }
            | Self::SessionReview { .. }
            | Self::WeeklyReport { .. }
            | Self::VisionReview { .. } => 3,
            _ => 5,
        }
    }

    /// Short human label for logs and UI.
    pub fn label(&self) -> String {
        match self {
            Self::ResolveAccount { account_id, .. } => format!("resolve account {account_id}"),
            Self::SyncMatchList { account_id, count } => format!("list {count} matches for account {account_id}"),
            Self::FetchMatch { match_id } => format!("match {match_id}"),
            Self::FetchTimeline { match_id } => format!("timeline {match_id}"),
            Self::DdragonSync { patch } => format!("Data Dragon {patch}"),
            Self::ComputeMetrics { match_id } => format!("metrics {match_id}"),
            Self::AiReview { match_id, .. } => format!("AI review {match_id}"),
            Self::SessionReview { .. } => "AI session review".to_owned(),
            Self::PostGame { .. } => "find your last game".to_owned(),
            Self::FetchRank { account_id } => format!("rank for account {account_id}"),
            Self::WeeklyReport { week, .. } => format!("AI weekly report ({week})"),
            Self::VisionReview { .. } => "vision review of your last game".to_owned(),
        }
    }

    pub fn uses_riot_api(&self) -> bool {
        matches!(
            self,
            Self::ResolveAccount { .. }
                | Self::SyncMatchList { .. }
                | Self::FetchMatch { .. }
                | Self::FetchTimeline { .. }
                | Self::PostGame { .. }
                | Self::FetchRank { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_as_tagged_json() {
        let j = Job::FetchMatch { match_id: "NA1_1".into() };
        let s = serde_json::to_string(&j).expect("ser");
        assert_eq!(s, r#"{"kind":"fetch_match","match_id":"NA1_1"}"#);
        assert_eq!(serde_json::from_str::<Job>(&s).expect("de"), j);
        assert_eq!(j.dedupe_key(), "match:NA1_1");
    }
}
