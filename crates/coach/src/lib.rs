//! The coaching layer: compares a game's metrics with populations, assembles
//! the match detail view, and produces validated AI reviews.

pub mod analysis;
pub mod detail;
pub mod facts;
pub mod provider;
pub mod review;
pub mod validate;

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
    #[error("{0}")]
    Invalid(String),
}

impl From<rc_store::rusqlite::Error> for CoachError {
    fn from(e: rc_store::rusqlite::Error) -> Self {
        Self::Store(e.into())
    }
}
