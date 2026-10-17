//! Shared building blocks: filesystem layout, logging, secret handling, and
//! game constants that every other crate needs.

pub mod credentials;
pub mod logging;
pub mod paths;
pub mod queues;
pub mod secret;
pub mod time;

pub use paths::AppPaths;
pub use secret::{KeySource, Secret};
