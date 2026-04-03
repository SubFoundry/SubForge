use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const AUTH_FAILURE_THRESHOLD: u32 = 5;
const AUTH_FAILURE_COOLDOWN_SECONDS: u64 = 60;
const RATE_LIMIT_STALE_SECONDS: u64 = 300;
const RATE_LIMIT_CLEANUP_INTERVAL_SECONDS: u64 = 30;
const AUTH_FAILURE_STALE_SECONDS: u64 = 300;
const AUTH_FAILURE_CLEANUP_INTERVAL_SECONDS: u64 = 30;
const MAX_TRACKED_KEYS: usize = 8192;

#[derive(Debug, Default)]
pub(crate) struct RateLimiter {
    inner: Mutex<RateLimiterState>,
}

#[derive(Debug, Clone, Copy)]
struct RateWindow {
    started_at: Instant,
    last_seen_at: Instant,
    count: u32,
}

#[derive(Debug)]
struct RateLimiterState {
    windows: HashMap<String, RateWindow>,
    last_cleanup_at: Instant,
}

impl Default for RateLimiterState {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            last_cleanup_at: Instant::now(),
        }
    }
}

impl RateLimiter {
    pub(crate) fn is_allowed(&self, key: &str, limit: u32, window: Duration) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        let now = Instant::now();
        maybe_cleanup_rate_windows(&mut inner, now);
        if !inner.windows.contains_key(key) {
            ensure_rate_capacity(&mut inner);
        }
        let entry = inner.windows.entry(key.to_string()).or_insert(RateWindow {
            started_at: now,
            last_seen_at: now,
            count: 0,
        });
        if now.duration_since(entry.started_at) >= window {
            entry.started_at = now;
            entry.count = 0;
        }
        if entry.count >= limit {
            entry.last_seen_at = now;
            return false;
        }
        entry.count += 1;
        entry.last_seen_at = now;
        true
    }
}

#[derive(Debug, Default)]
pub(crate) struct AuthFailures {
    inner: Mutex<AuthFailureTrackerState>,
}

#[derive(Debug, Clone, Copy)]
struct AuthFailureWindow {
    failures: u32,
    cooldown_until: Option<Instant>,
    last_seen_at: Instant,
}

#[derive(Debug)]
struct AuthFailureTrackerState {
    windows: HashMap<String, AuthFailureWindow>,
    last_cleanup_at: Instant,
}

impl Default for AuthFailureTrackerState {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            last_cleanup_at: Instant::now(),
        }
    }
}

impl AuthFailures {
    pub(crate) fn is_in_cooldown(&self, key: &str) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return true,
        };
        let now = Instant::now();
        maybe_cleanup_auth_windows(&mut inner, now);
        if let Some(window) = inner.windows.get_mut(key) {
            window.last_seen_at = now;
            if let Some(deadline) = window.cooldown_until {
                if now < deadline {
                    return true;
                }
                window.cooldown_until = None;
                window.failures = 0;
            }
        }
        false
    }

    pub(crate) fn record_failure(&self, key: &str) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return true,
        };
        let now = Instant::now();
        maybe_cleanup_auth_windows(&mut inner, now);
        if !inner.windows.contains_key(key) {
            ensure_auth_capacity(&mut inner);
        }
        let window = inner
            .windows
            .entry(key.to_string())
            .or_insert(AuthFailureWindow {
                failures: 0,
                cooldown_until: None,
                last_seen_at: now,
            });
        window.last_seen_at = now;
        if let Some(deadline) = window.cooldown_until {
            if now < deadline {
                return true;
            }
            window.cooldown_until = None;
            window.failures = 0;
        }
        window.failures += 1;
        if window.failures >= AUTH_FAILURE_THRESHOLD {
            window.failures = 0;
            window.cooldown_until =
                Some(Instant::now() + Duration::from_secs(AUTH_FAILURE_COOLDOWN_SECONDS));
            return true;
        }
        false
    }

    pub(crate) fn reset(&self, key: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.windows.remove(key);
        }
    }

    pub(crate) fn reset_all(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.windows.clear();
        }
    }
}

fn maybe_cleanup_rate_windows(inner: &mut RateLimiterState, now: Instant) {
    if now.duration_since(inner.last_cleanup_at)
        < Duration::from_secs(RATE_LIMIT_CLEANUP_INTERVAL_SECONDS)
        && inner.windows.len() < MAX_TRACKED_KEYS
    {
        return;
    }
    inner.last_cleanup_at = now;
    let stale_after = Duration::from_secs(RATE_LIMIT_STALE_SECONDS);
    inner
        .windows
        .retain(|_, window| now.duration_since(window.last_seen_at) < stale_after);
}

fn maybe_cleanup_auth_windows(inner: &mut AuthFailureTrackerState, now: Instant) {
    if now.duration_since(inner.last_cleanup_at)
        < Duration::from_secs(AUTH_FAILURE_CLEANUP_INTERVAL_SECONDS)
        && inner.windows.len() < MAX_TRACKED_KEYS
    {
        return;
    }
    inner.last_cleanup_at = now;
    let stale_after = Duration::from_secs(AUTH_FAILURE_STALE_SECONDS);
    inner.windows.retain(|_, window| {
        if let Some(deadline) = window.cooldown_until
            && now < deadline
        {
            return true;
        }
        now.duration_since(window.last_seen_at) < stale_after
    });
}

fn ensure_rate_capacity(inner: &mut RateLimiterState) {
    if inner.windows.len() < MAX_TRACKED_KEYS {
        return;
    }
    if let Some(oldest_key) = inner
        .windows
        .iter()
        .min_by_key(|(_, window)| window.last_seen_at)
        .map(|(key, _)| key.clone())
    {
        inner.windows.remove(&oldest_key);
    }
}

fn ensure_auth_capacity(inner: &mut AuthFailureTrackerState) {
    if inner.windows.len() < MAX_TRACKED_KEYS {
        return;
    }
    if let Some(oldest_key) = inner
        .windows
        .iter()
        .min_by_key(|(_, window)| window.last_seen_at)
        .map(|(key, _)| key.clone())
    {
        inner.windows.remove(&oldest_key);
    }
}
