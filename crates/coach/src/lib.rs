//! The coaching layer: compares a game's metrics with populations, assembles
//! the match detail view, and produces validated AI reviews.

pub mod analysis;
pub mod benchmarks;
pub mod detail;
pub mod facts;
pub mod goals;
pub mod live;
pub mod patterns;
pub mod pregame;
pub mod provider;
pub mod review;
pub mod sessions;
pub mod validate;
pub mod weekly;

#[derive(Debug, thiserror::Error)]
pub enum CoachError {
    #[error(transparent)]
    Store(#[from] rc_store::StoreError),
    #[error(transparent)]
    Metrics(#[from] rc_metrics::MetricsError),
    #[error(transparent)]
    Ai(#[from] rc_ai::AiError),
    #[error("not found: {0}")]
    NotFound(String),
    /// The monthly AI token cap is reached.
    #[error("{0}")]
    Budget(String),
    #[error("{0}")]
    Invalid(String),
}

impl From<rc_store::rusqlite::Error> for CoachError {
    fn from(e: rc_store::rusqlite::Error) -> Self {
        Self::Store(e.into())
    }
}
