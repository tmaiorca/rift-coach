//! Non-secret key/value settings stored as JSON. Never put keys or tokens here.

use rc_common::time::now_ms;
use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Result;

pub fn get<T: DeserializeOwned>(conn: &Connection, key: &str) -> Result<Option<T>> {
    let raw: Option<String> =
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into)).transpose()
}

pub fn set<T: Serialize>(conn: &Connection, key: &str, value: &T) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, serde_json::to_string(value)?, now_ms()],
    )?;
    Ok(())
}

/// Settings key for [`AppSettings`].
pub const APP: &str = "app";
/// Settings key for the UI's own state (last page, open match, ...), which
/// the UI owns as free-form JSON.
pub const UI_STATE: &str = "ui_state";

/// User-adjustable options. Missing fields take defaults, so adding a field
/// never breaks an existing settings row. No secrets: API keys live in
/// Windows Credential Manager.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppSettings {
    /// Matches to fetch per sync / lookup.
    pub ingest_count: u32,
    /// "Look back" window: your last N games used for personal baselines,
    /// trends, and the dashboard.
    pub baseline_games: u32,
    /// Show ARAM, Arena, etc. in the match list (they're never in metrics).
    pub show_other_queues: bool,
    /// Account the UI is showing; `None` means the primary account.
    pub active_account_id: Option<i64>,
    /// Games less than this many minutes apart belong to one play session.
    pub session_gap_min: u32,
    pub ai: AiSettings,
    pub appearance: AppearanceSettings,
    pub live: LiveSettings,
    pub benchmarks: BenchmarkSettings,
    pub desktop: DesktopSettings,
    pub mcp: McpSettings,
    pub vision: VisionSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            ingest_count: 100,
            baseline_games: 30,
            show_other_queues: true,
            active_account_id: None,
            session_gap_min: 60,
            ai: AiSettings::default(),
            appearance: AppearanceSettings::default(),
            live: LiveSettings::default(),
            benchmarks: BenchmarkSettings::default(),
            desktop: DesktopSettings::default(),
            mcp: McpSettings::default(),
            vision: VisionSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BenchmarkSettings {
    /// Collect higher-rank games in the background (never during a game).
    pub enabled: bool,
    /// `AUTO` (two tiers above your solo rank), `GOLD`, `PLATINUM`, `EMERALD`,
    /// `DIAMOND` or `MASTER` (Master and above).
    pub tier: String,
    /// Champions to sample first (Data Dragon ids). Empty = your most played.
    pub champions: Vec<String>,
    /// Seconds between collection steps (each makes 1-2 Riot calls).
    pub step_seconds: u32,
    /// Most benchmark games kept on disk (about 60 KB each).
    pub max_games: u32,
}

impl Default for BenchmarkSettings {
    fn default() -> Self {
        Self { enabled: true, tier: "AUTO".into(), champions: Vec::new(), step_seconds: 5, max_games: 2000 }
    }
}

pub const BENCHMARK_TIERS: [&str; 6] = ["AUTO", "GOLD", "PLATINUM", "EMERALD", "DIAMOND", "MASTER"];

/// Window, tray, startup and update behavior. "Start with Windows" isn't
/// here: Windows itself (the Run registry key) is the source of truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DesktopSettings {
    /// Closing the window keeps Rift Coach running in the tray (so it can
    /// catch your next game).
    pub close_to_tray: bool,
    /// Look for a new version on start (GitHub releases).
    pub check_updates: bool,
    /// The first-run setup was finished or skipped.
    pub onboarded: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self { close_to_tray: true, check_updates: true, onboarded: false }
    }
}

/// The read-only MCP server for AI agents (Claude Desktop, your own agent).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct McpSettings {
    pub enabled: bool,
    pub port: u16,
    /// Listen on your local network too (always with the access token).
    pub lan: bool,
    /// Require the access token on this PC too.
    pub require_token_local: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self { enabled: false, port: 7337, lan: false, require_token_local: false }
    }
}

/// Vision review: screenshots of your game window during a game, reviewed
/// by a vision model only after the game (never during it). Frames are
/// deleted once reviewed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VisionSettings {
    /// Capture the game window during Summoner's Rift games.
    pub enabled: bool,
    /// Seconds between screenshots (slows down by itself if the PC is busy).
    pub interval_s: u32,
    /// Longest edge of a stored frame in pixels (960, 1280 or 1600).
    pub max_width: u32,
    /// Most frames sent to the AI per game.
    pub max_frames: u32,
    /// Review automatically once the game is analyzed.
    pub auto_review: bool,
    /// Separate provider/model from the text reviews (keys are shared per
    /// provider in Credential Manager).
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub max_tokens: u32,
}

impl Default for VisionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_s: 4,
            max_width: 1280,
            max_frames: 24,
            auto_review: true,
            provider: "anthropic".into(),
            model: "claude-sonnet-5-5".into(),
            base_url: None,
            max_tokens: 6000,
        }
    }
}

pub const VISION_INTERVALS: [u32; 6] = [3, 4, 5, 6, 8, 10];
pub const VISION_WIDTHS: [u32; 3] = [960, 1280, 1600];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LiveSettings {
    /// Watch for a running game (Live Client Data API on 127.0.0.1:2999).
    pub enabled: bool,
    /// Hold match fetching, metrics, and AI jobs while a game is running.
    pub pause_background: bool,
    /// CS per minute to pace against. `None` = an active CS/min goal, else
    /// your own average in the role.
    pub cs_target_pm: Option<f64>,
    /// Days to keep a game's live snapshots after it's analyzed (0 = delete
    /// as soon as it is).
    pub keep_snapshots_days: u32,
    /// Windows notification when a finished game's analysis or AI review is ready.
    pub notify: bool,
}

impl Default for LiveSettings {
    fn default() -> Self {
        Self { enabled: true, pause_background: true, cs_target_pm: None, keep_snapshots_days: 7, notify: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AiSettings {
    /// `anthropic` | `open_ai` | `gemini` | `open_ai_compatible`.
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    /// Review each new game automatically once its timeline arrives
    /// (only games that ended in the last few hours, never a backfill).
    pub auto_post_game: bool,
    /// Review a play session automatically once it has ended.
    pub auto_session: bool,
    /// Write a weekly report automatically for the week that just ended.
    pub auto_weekly: bool,
    /// Write an AI game plan at the loading screen (from champions only).
    pub auto_pregame: bool,
    pub max_tokens: u32,
    /// Stop generating reports once this many tokens were used in the last
    /// 30 days. `None` = no cap.
    pub monthly_token_limit: Option<u64>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            model: "claude-sonnet-5-5".into(),
            base_url: None,
            auto_post_game: false,
            auto_session: false,
            auto_weekly: false,
            auto_pregame: false,
            max_tokens: 4000,
            monthly_token_limit: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppearanceSettings {
    pub animated_background: bool,
    /// 0.0–1.0: how visible the splash art and particles are.
    pub background_intensity: f64,
    /// Data Dragon champion id (e.g. `Leblanc`) to always show; `None`
    /// follows the open match, else your most-played champion.
    pub background_champion: Option<String>,
    pub particles: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self { animated_background: true, background_intensity: 0.6, background_champion: None, particles: true }
    }
}

pub fn app(conn: &Connection) -> Result<AppSettings> {
    Ok(get(conn, APP)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_settings_fill_missing_fields() {
        let db = crate::Db::open_in_memory().expect("db");
        db.with(|c| {
            assert_eq!(super::app(c)?, super::AppSettings::default());
            super::set(c, super::APP, &serde_json::json!({"baseline_games": 12, "ai": {"provider": "gemini"}}))?;
            let s = super::app(c)?;
            assert_eq!(s.baseline_games, 12);
            assert_eq!(s.ingest_count, 100);
            assert_eq!(s.ai.provider, "gemini");
            assert_eq!(s.ai.max_tokens, 4000);
            assert_eq!(s.session_gap_min, 60);
            assert_eq!(s.ai.monthly_token_limit, None);
            assert!(s.live.enabled && s.live.pause_background);
            assert_eq!(s.live.keep_snapshots_days, 7);
            assert_eq!(s.benchmarks.tier, "AUTO");
            assert!(s.benchmarks.enabled && !s.ai.auto_weekly && !s.ai.auto_pregame);
            assert!(s.desktop.close_to_tray && !s.desktop.onboarded && !s.mcp.enabled && !s.mcp.lan);
            assert!(!s.vision.enabled && s.vision.auto_review);
            assert_eq!((s.vision.interval_s, s.vision.max_width, s.vision.max_frames), (4, 1280, 24));
            Ok(())
        })
        .expect("ok");
    }

    #[test]
    fn round_trips() {
        let db = crate::Db::open_in_memory().expect("db");
        db.with(|c| {
            assert_eq!(super::get::<u32>(c, "x")?, None);
            super::set(c, "x", &5u32)?;
            super::set(c, "x", &7u32)?;
            assert_eq!(super::get::<u32>(c, "x")?, Some(7));
            Ok(())
        })
        .expect("ok");
    }
}
