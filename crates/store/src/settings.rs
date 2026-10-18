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
        }
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
