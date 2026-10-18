//! In-game support that stays inside Riot's rules: the Live Client Data API
//! is read for the player's **own** numbers only (other players are dropped
//! while parsing), the live view shows facts and pre-set targets, and
//! nothing here reacts to game state with advice, alerts, or input.
//!
//! - [`client`]: the loopback HTTPS client.
//! - [`tracker`]: idle → loading → in game → ended, from polls.
//! - [`view`]: CS pacing, build progress, focus-goal values.
//! - [`footprint`]: the app's CPU/memory while a game runs.
//! - [`mock`]: a scripted game for tests and `rc-cli mock-live`.

pub mod client;
pub mod footprint;
pub mod mock;
pub mod model;
pub mod tracker;
pub mod view;

pub use client::{LiveClient, LiveError, Observation};
pub use model::OwnSnapshot;
pub use tracker::{EndReason, Phase, Tracker, Transition};
