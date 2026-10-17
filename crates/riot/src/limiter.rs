//! Central rate limiter.
//!
//! Riot enforces limits per routing value (`na1`, `americas`, ...) at two
//! levels: the application (all calls with this key) and each method. Limits
//! are fixed windows that start at the first call, with boundaries we can't
//! observe. A sliding log never puts more than `limit` calls into *any*
//! interval of length `period`, so it stays inside every possible fixed
//! window. A small padding and safety factor absorb clock skew and latency.
//!
//! Limits are learned from `X-App-Rate-Limit` / `X-Method-Rate-Limit`
//! headers. The `-Count` headers are used to catch up when another process
//! (e.g. the CLI next to the app) shares the same key. 429s block the
//! offending scope for `Retry-After`.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tokio::time::Instant;

/// Extra time added to each window to absorb skew between our clock and
/// Riot's window boundaries.
const WINDOW_PADDING: Duration = Duration::from_millis(100);
/// Fraction of each published limit we allow ourselves to use.
const DEFAULT_SAFETY: f64 = 0.95;

/// One `limit:period` pair, e.g. `100:120` = 100 calls per 120 seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LimitWindow {
    pub limit: u32,
    pub period_s: u64,
}

impl LimitWindow {
    pub const fn new(limit: u32, period_s: u64) -> Self {
        Self { limit, period_s }
    }
}

/// Published Personal-key application limits (20/1s, 100/2min).
pub const PERSONAL_KEY_LIMITS: [LimitWindow; 2] = [LimitWindow::new(20, 1), LimitWindow::new(100, 120)];

/// Which bucket a 429 applies to, from the `X-Rate-Limit-Type` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitScope {
    Application,
    Method,
    /// `service`, or a 429 without the header (an underlying service limit).
    /// Treated like a method block, since it is shared by everyone, not us.
    Service,
}

impl LimitScope {
    pub fn from_header(value: Option<&str>) -> Self {
        match value.map(str::to_ascii_lowercase).as_deref() {
            Some("application") => Self::Application,
            Some("method") => Self::Method,
            _ => Self::Service,
        }
    }
}

/// Parses `"20:1,100:120"` into windows. Malformed parts are skipped.
pub fn parse_windows(header: &str) -> Vec<LimitWindow> {
    header
        .split(',')
        .filter_map(|part| {
            let (n, p) = part.trim().split_once(':')?;
            Some(LimitWindow::new(n.trim().parse().ok()?, p.trim().parse().ok()?))
        })
        .filter(|w| w.limit > 0 && w.period_s > 0)
        .collect()
}

#[derive(Debug)]
struct Window {
    limit: u32,
    period: Duration,
    log: VecDeque<Instant>,
}

impl Window {
    fn new(spec: LimitWindow) -> Self {
        Self { limit: spec.limit, period: Duration::from_secs(spec.period_s), log: VecDeque::new() }
    }

    fn capacity(&self, safety: f64) -> usize {
        ((f64::from(self.limit) * safety).floor() as usize).max(1)
    }

    fn prune(&mut self, now: Instant) {
        while self.log.front().is_some_and(|&t| t + self.period + WINDOW_PADDING <= now) {
            self.log.pop_front();
        }
    }

    /// How long until one more call fits in this window.
    fn wait(&mut self, now: Instant, safety: f64) -> Duration {
        self.prune(now);
        let cap = self.capacity(safety);
        if self.log.len() < cap {
            return Duration::ZERO;
        }
        let oldest_blocking = self.log[self.log.len() - cap];
        (oldest_blocking + self.period + WINDOW_PADDING).saturating_duration_since(now)
    }

    /// Server says it has seen `count` calls in this window. If that's more
    /// than we know about, assume the extra calls happened just now.
    fn sync_count(&mut self, count: u32, now: Instant) {
        self.prune(now);
        while self.log.len() < count as usize {
            self.log.push_back(now);
        }
    }
}

#[derive(Debug, Default)]
struct Bucket {
    windows: Vec<Window>,
    blocked_until: Option<Instant>,
}

impl Bucket {
    fn with_limits(specs: &[LimitWindow]) -> Self {
        Self { windows: specs.iter().copied().map(Window::new).collect(), blocked_until: None }
    }

    /// Replace limits, keeping call history for windows whose period is unchanged.
    fn set_limits(&mut self, specs: &[LimitWindow]) {
        let mut old = std::mem::take(&mut self.windows);
        for spec in specs {
            let period = Duration::from_secs(spec.period_s);
            match old.iter().position(|w| w.period == period) {
                Some(i) => {
                    let mut w = old.swap_remove(i);
                    w.limit = spec.limit;
                    self.windows.push(w);
                }
                None => self.windows.push(Window::new(*spec)),
            }
        }
    }

    fn sync_counts(&mut self, counts: &[LimitWindow], now: Instant) {
        for c in counts {
            let period = Duration::from_secs(c.period_s);
            if let Some(w) = self.windows.iter_mut().find(|w| w.period == period) {
                w.sync_count(c.limit, now);
            }
        }
    }

    fn wait(&mut self, now: Instant, safety: f64) -> Duration {
        let blocked = self.blocked_until.map_or(Duration::ZERO, |t| t.saturating_duration_since(now));
        self.windows.iter_mut().map(|w| w.wait(now, safety)).fold(blocked, Duration::max)
    }

    fn record(&mut self, now: Instant) {
        for w in &mut self.windows {
            w.log.push_back(now);
        }
    }

    fn block(&mut self, until: Instant) {
        self.blocked_until = Some(self.blocked_until.map_or(until, |t| t.max(until)));
    }
}

#[derive(Debug, Default)]
struct State {
    apps: HashMap<String, Bucket>,
    methods: HashMap<(String, String), Bucket>,
}

/// Usage of one bucket, for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct BucketSnapshot {
    pub routing: String,
    /// `"app"` or the method ID.
    pub scope: String,
    pub windows: Vec<WindowSnapshot>,
    pub blocked_for_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowSnapshot {
    pub limit: u32,
    pub period_s: u64,
    pub used: usize,
}

#[derive(Debug)]
pub struct RateLimiter {
    state: Mutex<State>,
    default_app_limits: Vec<LimitWindow>,
    safety: f64,
}

impl RateLimiter {
    pub fn new(default_app_limits: &[LimitWindow]) -> Self {
        Self::with_safety(default_app_limits, DEFAULT_SAFETY)
    }

    pub fn with_safety(default_app_limits: &[LimitWindow], safety: f64) -> Self {
        Self {
            state: Mutex::new(State::default()),
            default_app_limits: default_app_limits.to_vec(),
            safety: safety.clamp(0.1, 1.0),
        }
    }

    pub fn personal_key() -> Self {
        Self::new(&PERSONAL_KEY_LIMITS)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic while holding this lock can't leave the state inconsistent
        // in a way that matters (worst case: a slightly stale log).
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Waits until a call to `method` on `routing` fits every applicable
    /// window, then records it.
    pub async fn acquire(&self, routing: &str, method: &str) {
        loop {
            let wait = {
                let mut st = self.lock();
                let now = Instant::now();
                let defaults = &self.default_app_limits;
                let app_wait = st
                    .apps
                    .entry(routing.to_owned())
                    .or_insert_with(|| Bucket::with_limits(defaults))
                    .wait(now, self.safety);
                let method_wait = st
                    .methods
                    .get_mut(&(routing.to_owned(), method.to_owned()))
                    .map_or(Duration::ZERO, |b| b.wait(now, self.safety));
                let wait = app_wait.max(method_wait);
                if wait.is_zero() {
                    if let Some(b) = st.apps.get_mut(routing) {
                        b.record(now);
                    }
                    if let Some(b) = st.methods.get_mut(&(routing.to_owned(), method.to_owned())) {
                        b.record(now);
                    }
                    return;
                }
                wait
            };
            tracing::trace!(routing, method, wait_ms = wait.as_millis() as u64, "rate limiter waiting");
            tokio::time::sleep(wait).await;
        }
    }

    /// Feed the rate-limit headers of any response (including 429s) back in.
    pub fn update_from_headers(
        &self,
        routing: &str,
        method: &str,
        app_limit: Option<&str>,
        app_count: Option<&str>,
        method_limit: Option<&str>,
        method_count: Option<&str>,
    ) {
        let now = Instant::now();
        let mut st = self.lock();
        if let Some(limits) = app_limit.map(parse_windows).filter(|v| !v.is_empty()) {
            let bucket = st.apps.entry(routing.to_owned()).or_default();
            bucket.set_limits(&limits);
            if let Some(counts) = app_count.map(parse_windows) {
                bucket.sync_counts(&counts, now);
            }
        }
        if let Some(limits) = method_limit.map(parse_windows).filter(|v| !v.is_empty()) {
            let key = (routing.to_owned(), method.to_owned());
            let is_new = !st.methods.contains_key(&key);
            let bucket = st.methods.entry(key).or_default();
            bucket.set_limits(&limits);
            if is_new {
                // This call already happened; count it.
                bucket.record(now);
            }
            if let Some(counts) = method_count.map(parse_windows) {
                bucket.sync_counts(&counts, now);
            }
        }
    }

    /// Block the scope a 429 applied to for `retry_after`.
    pub fn on_rate_limited(&self, routing: &str, method: &str, scope: LimitScope, retry_after: Duration) {
        let until = Instant::now() + retry_after;
        let mut st = self.lock();
        match scope {
            LimitScope::Application => st.apps.entry(routing.to_owned()).or_default().block(until),
            LimitScope::Method | LimitScope::Service => {
                st.methods.entry((routing.to_owned(), method.to_owned())).or_default().block(until)
            }
        }
    }

    pub fn snapshot(&self) -> Vec<BucketSnapshot> {
        let now = Instant::now();
        let mut st = self.lock();
        let State { apps, methods } = &mut *st;
        let snap = |routing: &str, scope: &str, b: &mut Bucket| BucketSnapshot {
            routing: routing.to_owned(),
            scope: scope.to_owned(),
            windows: b
                .windows
                .iter_mut()
                .map(|w| {
                    w.prune(now);
                    WindowSnapshot { limit: w.limit, period_s: w.period.as_secs(), used: w.log.len() }
                })
                .collect(),
            blocked_for_ms: b.blocked_until.map_or(0, |t| t.saturating_duration_since(now).as_millis() as u64),
        };
        let mut out: Vec<_> = apps.iter_mut().map(|(r, b)| snap(r, "app", b)).collect();
        out.extend(methods.iter_mut().map(|((r, m), b)| snap(r, m, b)));
        out.sort_by(|a, b| (&a.routing, &a.scope).cmp(&(&b.routing, &b.scope)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_window_headers() {
        assert_eq!(parse_windows("20:1,100:120"), vec![LimitWindow::new(20, 1), LimitWindow::new(100, 120)]);
        assert_eq!(parse_windows(" 5:10 , junk, 0:5, 3:x"), vec![LimitWindow::new(5, 10)]);
        assert!(parse_windows("").is_empty());
    }

    #[test]
    fn scope_from_header() {
        assert_eq!(LimitScope::from_header(Some("application")), LimitScope::Application);
        assert_eq!(LimitScope::from_header(Some("Method")), LimitScope::Method);
        assert_eq!(LimitScope::from_header(Some("service")), LimitScope::Service);
        assert_eq!(LimitScope::from_header(None), LimitScope::Service);
    }

    #[tokio::test(start_paused = true)]
    async fn short_window_spreads_calls() {
        let rl = RateLimiter::with_safety(&[LimitWindow::new(3, 1)], 1.0);
        let start = Instant::now();
        for _ in 0..7 {
            rl.acquire("na1", "m").await;
        }
        // 3 now, 3 after ~1.1s, 1 after ~2.2s.
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(2200), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(2400), "{elapsed:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn personal_key_long_window_is_respected() {
        let rl = RateLimiter::personal_key();
        let start = Instant::now();
        // Capacity with safety 0.95 is 95 per 120s, so call 96 must wait for
        // the first call to age out of the 2-minute window.
        for _ in 0..95 {
            rl.acquire("americas", "m").await;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        rl.acquire("americas", "m").await;
        assert!(start.elapsed() >= Duration::from_secs(120));
    }

    #[tokio::test(start_paused = true)]
    async fn routing_values_are_independent() {
        let rl = RateLimiter::with_safety(&[LimitWindow::new(1, 10)], 1.0);
        let start = Instant::now();
        rl.acquire("na1", "m").await;
        rl.acquire("americas", "m").await;
        assert!(start.elapsed() < Duration::from_millis(1));
    }

    #[tokio::test(start_paused = true)]
    async fn method_limits_learned_from_headers() {
        let rl = RateLimiter::with_safety(&[LimitWindow::new(100, 1)], 1.0);
        rl.acquire("americas", "match").await;
        rl.update_from_headers("americas", "match", None, None, Some("2:10"), Some("1:10"));
        let start = Instant::now();
        rl.acquire("americas", "match").await; // second of 2: immediate
        assert!(start.elapsed() < Duration::from_millis(1));
        rl.acquire("americas", "match").await; // third: waits for the window
        assert!(start.elapsed() >= Duration::from_secs(10));
        // Other methods are unaffected by this method's limit.
        let t = Instant::now();
        rl.acquire("americas", "other").await;
        assert!(t.elapsed() < Duration::from_millis(1));
    }

    #[tokio::test(start_paused = true)]
    async fn server_counts_from_other_processes_are_absorbed() {
        let rl = RateLimiter::with_safety(&[LimitWindow::new(5, 10)], 1.0);
        rl.acquire("na1", "m").await;
        // Server has seen 5 in this window (another process used the key).
        rl.update_from_headers("na1", "m", Some("5:10"), Some("5:10"), None, None);
        let start = Instant::now();
        rl.acquire("na1", "m").await;
        assert!(start.elapsed() >= Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn retry_after_blocks_the_right_scope() {
        let rl = RateLimiter::with_safety(&[LimitWindow::new(100, 1)], 1.0);
        rl.on_rate_limited("na1", "league", LimitScope::Method, Duration::from_secs(7));
        let t = Instant::now();
        rl.acquire("na1", "mastery").await;
        assert!(t.elapsed() < Duration::from_millis(1), "other methods not blocked");
        rl.acquire("na1", "league").await;
        assert!(t.elapsed() >= Duration::from_secs(7));

        rl.on_rate_limited("na1", "league", LimitScope::Application, Duration::from_secs(3));
        let t = Instant::now();
        rl.acquire("na1", "mastery").await;
        assert!(t.elapsed() >= Duration::from_secs(3), "app block applies to all methods");
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_callers_never_exceed_the_window() {
        use std::sync::Arc;
        let rl = Arc::new(RateLimiter::with_safety(&[LimitWindow::new(4, 1)], 1.0));
        let start = Instant::now();
        let handles: Vec<_> = (0..12)
            .map(|_| {
                let rl = rl.clone();
                tokio::spawn(async move {
                    rl.acquire("na1", "m").await;
                    start.elapsed()
                })
            })
            .collect();
        let mut times = Vec::new();
        for h in handles {
            times.push(h.await.expect("task"));
        }
        times.sort();
        // In any 1s interval, at most 4 calls.
        for (i, t) in times.iter().enumerate().skip(4) {
            assert!(*t - times[i - 4] >= Duration::from_secs(1), "{times:?}");
        }
    }

    #[test]
    fn snapshot_reports_usage() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("rt");
        rt.block_on(async {
            let rl = RateLimiter::personal_key();
            rl.acquire("na1", "m").await;
            let snap = rl.snapshot();
            assert_eq!(snap.len(), 1);
            assert_eq!(snap[0].scope, "app");
            assert_eq!(snap[0].windows.iter().map(|w| w.used).collect::<Vec<_>>(), vec![1, 1]);
        });
    }
}
