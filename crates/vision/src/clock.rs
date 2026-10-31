//! Game time for frames. The live poller sets it from each Live Client poll;
//! the capture thread reads it to stamp frames with game time (so they line
//! up with Riot's timeline after the game).

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Older than this, the game is paused or the poller stopped: no frames.
const STALE: Duration = Duration::from_secs(12);

#[derive(Debug, Default)]
pub struct GameClock {
    last: Mutex<Option<(f64, Instant)>>,
}

impl GameClock {
    pub fn set(&self, game_time_s: f64) {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        // A repeated value means the game clock isn't moving (paused); keep
        // the old instant so it goes stale.
        if last.is_none_or(|(t, _)| (t - game_time_s).abs() > f64::EPSILON) {
            *last = Some((game_time_s, Instant::now()));
        }
    }

    /// Current game time in milliseconds, if the clock is fresh.
    pub fn now_ms(&self) -> Option<i64> {
        let last = *self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let (t, at) = last?;
        let since = at.elapsed();
        (since < STALE).then(|| ((t + since.as_secs_f64()) * 1000.0).round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrapolates_from_last_poll() {
        let c = GameClock::default();
        assert_eq!(c.now_ms(), None);
        c.set(100.0);
        let t = c.now_ms().expect("fresh");
        assert!((100_000..101_000).contains(&t));
    }
}
