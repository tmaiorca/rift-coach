//! One game's capture. A [`Session`] owns the frames and stats of a game and
//! (re)starts Windows Graphics Capture on the game window when asked; the
//! per-frame work (copy, downscale, encode, save) runs on the capture thread
//! only when a frame is due, so most delivered frames are dropped at once.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::clock::GameClock;
use crate::encode::{JPEG_QUALITY, RawFrame, downscale, jpeg};
use crate::frames::FrameStore;
use crate::{CaptureStats, VisionError};

/// A frame slower than this stretches the next interval.
const SLOW_MS: f64 = 120.0;
/// How much and for how many frames the interval stretches.
const SLOW_FACTOR: u32 = 2;
const SLOW_FRAMES: u32 = 5;
const MAX_INTERVAL: Duration = Duration::from_secs(15);
/// When no game time is known (paused), look again this soon.
const NO_CLOCK_RETRY: Duration = Duration::from_secs(1);
/// A game's frames stop at this size (a 40-minute game at 4 s uses ~70 MB).
const MAX_GAME_BYTES: u64 = 400 * 1024 * 1024;
/// Mean brightness below this is a black frame (exclusive fullscreen).
const BLACK_MEAN: f64 = 2.0;
const BLACK_RUN_WARN: u32 = 5;
/// The capture API delivers at most this often (Windows 11 24H2+).
#[cfg(windows)]
const UPDATE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub live_game_id: i64,
    pub interval: Duration,
    pub max_width: u32,
    pub store: FrameStore,
    pub clock: Arc<GameClock>,
    /// Capture the window whose title contains this instead of the game
    /// (development; see [`crate::WINDOW_ENV`]).
    pub window: Option<String>,
}

#[derive(Debug, Default)]
struct Pace {
    next_due: Option<Instant>,
    slow_left: u32,
    black_run: u32,
    full: bool,
}

/// A game's capture state, shared with the capture thread.
#[doc(hidden)]
#[derive(Debug)]
pub struct Shared {
    cfg: CaptureConfig,
    stats: Mutex<CaptureStats>,
    pace: Mutex<Pace>,
}

impl Shared {
    fn new(cfg: CaptureConfig) -> Self {
        let stats = CaptureStats { interval_s: cfg.interval.as_secs_f64(), ..Default::default() };
        Self { cfg, stats: Mutex::new(stats), pace: Mutex::new(Pace::default()) }
    }

    fn stats(&self) -> std::sync::MutexGuard<'_, CaptureStats> {
        self.stats.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn pace(&self) -> std::sync::MutexGuard<'_, Pace> {
        self.pace.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The game time to stamp a frame with, when one is due now.
    pub(crate) fn due(&self, now: Instant) -> Option<i64> {
        let mut p = self.pace();
        if p.full || p.next_due.is_some_and(|t| now < t) {
            return None;
        }
        let Some(game_ms) = self.cfg.clock.now_ms() else {
            p.next_due = Some(now + NO_CLOCK_RETRY);
            return None;
        };
        // Reserve the slot now; `process` sets the real next time.
        p.next_due = Some(now + self.cfg.interval);
        Some(game_ms)
    }

    /// Downscales, encodes and stores one frame. `started` is when the work
    /// for it began (before the GPU copy).
    pub(crate) fn process(&self, game_ms: i64, raw: &RawFrame<'_>, started: Instant) {
        let result =
            downscale(raw, self.cfg.max_width).ok_or_else(|| "unexpected frame layout".to_owned()).and_then(|img| {
                if img.mean() < BLACK_MEAN {
                    return Ok(None);
                }
                let bytes = jpeg(&img, JPEG_QUALITY)?;
                self.cfg.store.write(self.cfg.live_game_id, game_ms, &bytes).map_err(|e| e.to_string())?;
                Ok(Some((bytes.len(), img.width, img.height)))
            });
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut stats = self.stats();
        let mut pace = self.pace();
        match result {
            Ok(Some((len, w, h))) => {
                stats.add_frame(ms, len);
                (stats.width, stats.height) = (w, h);
                pace.black_run = 0;
                if stats.bytes >= MAX_GAME_BYTES {
                    pace.full = true;
                    stats.error = Some("frame limit for one game reached; capture stopped".into());
                }
            }
            Ok(None) => {
                stats.black += 1;
                pace.black_run += 1;
                if pace.black_run == BLACK_RUN_WARN {
                    stats.error = Some(
                        "frames are black: set League to Borderless or Windowed (Video settings → Window mode)".into(),
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "vision frame failed");
                stats.error = Some(e);
            }
        }
        if ms > SLOW_MS {
            pace.slow_left = SLOW_FRAMES;
        }
        let interval = if pace.slow_left > 0 {
            pace.slow_left -= 1;
            stats.slowed += 1;
            (self.cfg.interval * SLOW_FACTOR).min(MAX_INTERVAL)
        } else {
            self.cfg.interval
        };
        stats.interval_s = interval.as_secs_f64();
        pace.next_due = Some(Instant::now() + interval);
    }
}

/// One game's capture: frames go to the [`FrameStore`] under the game's id.
pub struct Session {
    shared: Arc<Shared>,
    #[cfg(windows)]
    control: Option<imp::Control>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("live_game_id", &self.shared.cfg.live_game_id).finish_non_exhaustive()
    }
}

impl Session {
    pub fn new(cfg: CaptureConfig) -> Self {
        Self {
            shared: Arc::new(Shared::new(cfg)),
            #[cfg(windows)]
            control: None,
        }
    }

    pub fn live_game_id(&self) -> i64 {
        self.shared.cfg.live_game_id
    }

    pub fn stats(&self) -> CaptureStats {
        self.shared.stats().clone()
    }

    #[cfg(windows)]
    pub fn running(&self) -> bool {
        self.control.as_ref().is_some_and(|c| !c.is_finished())
    }

    #[cfg(not(windows))]
    pub fn running(&self) -> bool {
        false
    }

    /// Starts capture on the game window unless it is running. Returns
    /// whether it started now. Fails when the window isn't there (yet).
    #[cfg(windows)]
    pub fn ensure_running(&mut self) -> Result<bool, VisionError> {
        if self.running() {
            return Ok(false);
        }
        if let Some(old) = self.control.take() {
            // The window closed or was re-created (resolution change).
            let _ = old.stop();
        }
        let control =
            imp::start(self.shared.clone(), self.shared.cfg.window.as_deref()).map_err(VisionError::Capture)?;
        self.control = Some(control);
        let mut stats = self.shared.stats();
        stats.sessions += 1;
        stats.error = None;
        Ok(true)
    }

    #[cfg(not(windows))]
    pub fn ensure_running(&mut self) -> Result<bool, VisionError> {
        Err(VisionError::Capture("vision capture needs Windows".into()))
    }

    /// Stops capture (waits for the capture thread) and returns the stats.
    pub fn stop(self) -> CaptureStats {
        #[cfg(windows)]
        if let Some(c) = self.control
            && let Err(e) = c.stop()
        {
            tracing::warn!(error = %e, "vision capture did not stop cleanly");
        }
        self.shared.stats().clone()
    }
}

/// Is the game window there to capture? (For a status line; cheap.)
#[cfg(windows)]
pub fn game_window_present() -> bool {
    imp::find_window(None).is_some()
}

#[cfg(not(windows))]
pub fn game_window_present() -> bool {
    false
}

#[cfg(windows)]
mod imp {
    use std::sync::Arc;
    use std::time::Instant;

    use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
        SecondaryWindowSettings, Settings,
    };
    use windows_capture::window::Window;

    use super::{Shared, UPDATE_INTERVAL};
    use crate::encode::RawFrame;

    pub struct Handler {
        shared: Arc<Shared>,
    }

    pub type Control = CaptureControl<Handler, String>;

    impl GraphicsCaptureApiHandler for Handler {
        type Flags = Arc<Shared>;
        type Error = String;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self { shared: ctx.flags })
        }

        fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), String> {
            let now = Instant::now();
            let Some(game_ms) = self.shared.due(now) else { return Ok(()) };
            // Crops the title bar in windowed mode (borderless has none).
            let mut buf = frame.buffer_without_title_bar().map_err(|e| e.to_string())?;
            let (width, height, row_pitch) = (buf.width(), buf.height(), buf.row_pitch() as usize);
            let raw = RawFrame { data: buf.as_raw_buffer(), width, height, row_pitch, bgra: true };
            self.shared.process(game_ms, &raw, now);
            Ok(())
        }
    }

    pub fn find_window(wanted: Option<&str>) -> Option<Window> {
        let wanted = wanted.map(str::trim).filter(|s| !s.is_empty());
        Window::enumerate().ok()?.into_iter().find(|w| {
            let Ok(title) = w.title() else { return false };
            let sized = w.width().is_ok_and(|x| x > 0) && w.height().is_ok_and(|y| y > 0);
            sized
                && match wanted {
                    Some(sub) => title.contains(sub) && !title.contains("Rift Coach"),
                    None => title == crate::GAME_WINDOW_TITLE,
                }
        })
    }

    fn settings(
        window: Window,
        shared: Arc<Shared>,
        interval: MinimumUpdateIntervalSettings,
    ) -> Settings<Arc<Shared>, Window> {
        Settings::new(
            window,
            CursorCaptureSettings::WithoutCursor,
            // No yellow capture border on the game.
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            interval,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            shared,
        )
    }

    pub fn start(shared: Arc<Shared>, wanted: Option<&str>) -> Result<Control, String> {
        let window = find_window(wanted).ok_or_else(|| "the game window isn't open".to_owned())?;
        let limited = MinimumUpdateIntervalSettings::Custom(UPDATE_INTERVAL);
        match Handler::start_free_threaded(settings(window, shared.clone(), limited)) {
            Ok(c) => Ok(c),
            Err(first) => {
                // Older Windows 11 builds can't limit the update rate; fine.
                tracing::info!(error = %first, "retrying capture without an update interval");
                Handler::start_free_threaded(settings(window, shared, MinimumUpdateIntervalSettings::Default)).map_err(
                    |e| match e.to_string() {
                        // Never capture with the yellow border drawn on the game.
                        s if s.contains("capture border") => {
                            "this version of Windows can't hide the capture border (Windows 11 needed)".to_owned()
                        }
                        s => format!("could not start capture: {s}"),
                    },
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(dir: &std::path::Path, clock: Arc<GameClock>) -> Arc<Shared> {
        Arc::new(Shared::new(CaptureConfig {
            live_game_id: 7,
            interval: Duration::from_secs(4),
            max_width: 32,
            store: FrameStore::new(dir),
            clock,
            window: None,
        }))
    }

    #[test]
    fn paces_and_stores_frames() {
        let tmp = tempfile::tempdir().expect("tmp");
        let clock = Arc::new(GameClock::default());
        let s = session(tmp.path(), clock.clone());
        let now = Instant::now();
        assert_eq!(s.due(now), None, "no game time yet");
        clock.set(60.0);
        // Still waiting out the no-clock retry.
        assert_eq!(s.due(now), None);
        let later = now + Duration::from_secs(2);
        let t = s.due(later).expect("due");
        assert!(t >= 60_000);
        assert_eq!(s.due(later), None, "slot reserved");

        let data = vec![90u8; 64 * 36 * 4];
        let raw = RawFrame { data: &data, width: 64, height: 36, row_pitch: 64 * 4, bgra: true };
        s.process(t, &raw, Instant::now());
        let st = s.stats().clone();
        assert_eq!((st.frames, st.width, st.height, st.black), (1, 32, 18, 0));
        assert!(st.bytes > 0);
        assert_eq!(FrameStore::new(tmp.path()).list(7), vec![t]);

        // A black frame is counted, not stored.
        let black = vec![0u8; 64 * 36 * 4];
        s.process(t + 4000, &RawFrame { data: &black, ..raw }, Instant::now());
        assert_eq!(s.stats().black, 1);
        assert_eq!(FrameStore::new(tmp.path()).list(7).len(), 1);
    }

    #[test]
    fn slow_frames_stretch_the_interval() {
        let tmp = tempfile::tempdir().expect("tmp");
        let s = session(tmp.path(), Arc::new(GameClock::default()));
        let data = vec![90u8; 8 * 8 * 4];
        let raw = RawFrame { data: &data, width: 8, height: 8, row_pitch: 32, bgra: true };
        let long_ago = Instant::now().checked_sub(Duration::from_millis(500)).expect("instant");
        s.process(1000, &raw, long_ago);
        let st = s.stats().clone();
        assert_eq!(st.slowed, 1);
        assert!((st.interval_s - 8.0).abs() < 1e-9);
    }
}
