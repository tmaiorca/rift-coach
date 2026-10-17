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
}

impl Job {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ResolveAccount { .. } => "resolve_account",
            Self::SyncMatchList { .. } => "sync_match_list",
            Self::FetchMatch { .. } => "fetch_match",
            Self::FetchTimeline { .. } => "fetch_timeline",
            Self::DdragonSync { .. } => "ddragon_sync",
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
        }
    }

    /// Higher runs first. Matches before timelines, so the match list fills
    /// quickly and timelines follow.
    pub fn priority(&self) -> i32 {
        match self {
            Self::ResolveAccount { .. } => 100,
            Self::SyncMatchList { .. } => 90,
            Self::DdragonSync { .. } => 80,
            Self::FetchMatch { .. } => 50,
            Self::FetchTimeline { .. } => 40,
        }
    }

    pub fn max_attempts(&self) -> i32 {
        match self {
            // A just-finished match can take several minutes to appear.
            Self::FetchMatch { .. } | Self::FetchTimeline { .. } => 8,
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
        }
    }

    pub fn uses_riot_api(&self) -> bool {
        !matches!(self, Self::DdragonSync { .. })
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
