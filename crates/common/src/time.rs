use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock time as unix milliseconds (the unit used in every DB column).
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)).unwrap_or(0)
}
