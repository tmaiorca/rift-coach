//! Layer 1 of the analytics engine: deterministic per-game metrics from
//! Match-V5 matches and timelines, for all ten participants. Pure functions,
//! no I/O; callers load the JSON and store the results.

pub mod compute;
pub mod defs;
pub mod detail;
pub mod model;
pub mod percentile;
pub mod zones;

pub use compute::{METRICS_VERSION, MatchMetrics, MetricMap, MetricsError, compute, parse_match, parse_timeline};
pub use defs::{METRICS, MetricDef};
pub use model::ItemCatalog;
