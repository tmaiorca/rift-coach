//! Vision review: screenshots of the game window, taken every few seconds
//! during a game and only stored (no analysis, no AI, nothing shown in
//! game). After the game, [`select`] picks the frames around the moments
//! that matter and the coach sends those to a vision model. Frames are
//! deleted once reviewed.
//!
//! Capture uses Windows Graphics Capture on the game window only (no other
//! windows or monitors), without the capture border or the cursor, at a
//! reduced update rate. Each frame is downscaled and JPEG-encoded on the
//! capture thread; the interval stretches by itself when that takes long.

pub mod capture;
pub mod clock;
pub mod encode;
pub mod frames;
pub mod select;

pub use capture::{CaptureConfig, Session};
pub use clock::GameClock;
pub use frames::{DiskUsage, FrameStore};
pub use select::{Anchor, AnchorKind, Moment, select};

use serde::{Deserialize, Serialize};

/// The in-game window of League of Legends.
pub const GAME_WINDOW_TITLE: &str = "League of Legends (TM) Client";
/// Overrides the window to capture by a title substring (development: lets
/// any window stand in for the game).
pub const WINDOW_ENV: &str = "RIFT_COACH_VISION_WINDOW";

/// What a game's capture did. Stored with the live game and shown in the app.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CaptureStats {
    pub frames: u32,
    pub bytes: u64,
    /// Frame size stored (after downscaling).
    pub width: u32,
    pub height: u32,
    /// Time to copy, downscale, encode and save one frame.
    pub avg_ms: f64,
    pub max_ms: f64,
    /// Frames taken at a stretched interval because the last one was slow.
    pub slowed: u32,
    /// Frames dropped because they were black (exclusive fullscreen).
    pub black: u32,
    /// Capture sessions started (more than 1: the window was re-created).
    pub sessions: u32,
    /// The interval in effect at the end, in seconds.
    pub interval_s: f64,
    /// Why capture stopped or never started, if it did.
    pub error: Option<String>,
}

impl CaptureStats {
    fn add_frame(&mut self, ms: f64, bytes: usize) {
        let n = f64::from(self.frames);
        self.avg_ms = (self.avg_ms * n + ms) / (n + 1.0);
        self.max_ms = self.max_ms.max(ms);
        self.frames += 1;
        self.bytes += bytes as u64;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VisionError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Capture(String),
}
