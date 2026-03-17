//! Orphan-plan watchdog.
//!
//! Encapsulates the activity timestamp and timeout configuration used to
//! detect plans whose clients disconnected mid-execution. The data lives
//! here; the background task that polls it is spawned by `RunEngine`
//! (see `RunEngine::spawn_watchdog` in `mod.rs`).

use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

/// Default watchdog timeout for orphaned plan detection (5 minutes).
const DEFAULT_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(300);

/// Manages the activity timestamp and timeout for orphan-plan detection.
///
/// The `RunEngine` owns a `WatchdogManager` and calls [`touch`] on every
/// meaningful command execution (MoveTo, Read, Trigger, etc.) and state
/// transition. A separate background task (spawned via
/// `RunEngine::spawn_watchdog`) periodically checks [`elapsed`] against
/// [`timeout`] to detect stale plans.
pub(crate) struct WatchdogManager {
    /// Timestamp of the last meaningful activity.
    last_activity: Arc<RwLock<Instant>>,
    /// How long a plan may remain active with no activity before abort.
    timeout: Duration,
}

impl WatchdogManager {
    pub(crate) fn new() -> Self {
        Self {
            last_activity: Arc::new(RwLock::new(Instant::now())),
            timeout: DEFAULT_WATCHDOG_TIMEOUT,
        }
    }

    /// Override the watchdog timeout.
    pub(crate) fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Get the configured timeout.
    pub(crate) fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Compute the check interval (1/10 of timeout, minimum 1 second).
    pub(crate) fn check_interval(&self) -> Duration {
        self.timeout.div_f64(10.0).max(Duration::from_secs(1))
    }

    /// Record meaningful activity (resets the timer).
    pub(crate) async fn touch(&self) {
        *self.last_activity.write().await = Instant::now();
    }

    /// How long since the last activity.
    pub(crate) async fn elapsed(&self) -> Duration {
        self.last_activity.read().await.elapsed()
    }

    /// Whether the timeout has been exceeded.
    pub(crate) async fn is_expired(&self) -> bool {
        self.elapsed().await > self.timeout
    }
}
