//! Background job engine: a persistent SQLite-backed queue plus workers that
//! run ingestion jobs (resolve account → list matches → fetch match →
//! fetch timeline → cache Data Dragon for the patch). Every job is
//! idempotent, so restarts and duplicate enqueues are safe.

mod bootstrap;
mod engine;
mod handlers;
mod job;

pub use bootstrap::{BootstrapError, build_engine};
pub use engine::{Engine, EngineConfig, EngineEvent, EngineStatus, IdleOutcome, KeyStatus};
pub use job::Job;
