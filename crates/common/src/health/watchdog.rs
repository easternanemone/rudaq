//! Hardware watchdog thread for daemon liveness monitoring (bd-qa36.4.1).
//!
//! A dedicated OS thread that monitors the daemon's main loop. If the daemon
//! fails to "kick" the watchdog within the configured timeout, the watchdog
//! fires an emergency action (typically closing shutters and stopping motors).
//!
//! # Why a std::thread?
//!
//! The watchdog MUST be a separate OS thread, not a tokio task. If the tokio
//! runtime deadlocks or hangs, a tokio task would also be stuck. A separate
//! OS thread can detect this and trigger emergency shutdown independently.
//!
//! # Usage
//!
//! ```rust,ignore
//! use common::health::watchdog::{HardwareWatchdog, WatchdogConfig};
//!
//! let (watchdog, kicker) = HardwareWatchdog::start(
//!     WatchdogConfig::default(),
//!     || {
//!         eprintln!("WATCHDOG FIRED! Emergency shutdown!");
//!     },
//! );
//!
//! // In the daemon's main loop (tokio task):
//! loop {
//!     kicker.kick();
//!     do_work().await;
//! }
//!
//! // During graceful shutdown:
//! kicker.disarm();
//! watchdog.stop();
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Default watchdog timeout (30 seconds).
pub const DEFAULT_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(30);

/// Minimum allowed watchdog timeout (1 second).
pub const MIN_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(1);

/// Configuration for the hardware watchdog.
#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    /// Maximum time between kicks before firing the emergency action.
    pub timeout: Duration,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_WATCHDOG_TIMEOUT,
        }
    }
}

/// Handle for kicking the watchdog from the daemon's main loop.
///
/// The daemon holds this handle and calls [`kick()`](WatchdogKicker::kick)
/// periodically to indicate it is still alive. If `kick()` is not called
/// within the configured timeout, the watchdog fires its emergency action.
#[derive(Clone)]
pub struct WatchdogKicker {
    kick_tx: SyncSender<()>,
    disarmed: Arc<AtomicBool>,
}

impl WatchdogKicker {
    /// Signal the watchdog that the daemon is alive.
    ///
    /// Returns `true` if the kick was sent, `false` if the watchdog has stopped.
    pub fn kick(&self) -> bool {
        match self.kick_tx.try_send(()) {
            Ok(()) => true,
            Err(mpsc::TrySendError::Full(())) => true, // kick already pending
            Err(mpsc::TrySendError::Disconnected(())) => false,
        }
    }

    /// Disarm the watchdog (e.g., during intentional shutdown).
    ///
    /// After disarming, the watchdog will NOT fire the emergency action even
    /// if kicks stop. This prevents false alarms during graceful shutdown.
    pub fn disarm(&self) {
        self.disarmed.store(true, Ordering::SeqCst);
    }

    /// Check whether the watchdog has been disarmed.
    pub fn is_disarmed(&self) -> bool {
        self.disarmed.load(Ordering::SeqCst)
    }
}

/// Hardware watchdog that monitors daemon liveness from a separate OS thread.
///
/// Created via [`HardwareWatchdog::start`], which spawns a dedicated thread
/// named `daq-watchdog`. The watchdog fires an emergency action if no
/// [`kick()`](WatchdogKicker::kick) is received within the configured timeout.
pub struct HardwareWatchdog {
    thread: Option<JoinHandle<()>>,
    disarmed: Arc<AtomicBool>,
    kick_tx: SyncSender<()>,
}

impl HardwareWatchdog {
    /// Start the watchdog on a dedicated OS thread.
    ///
    /// Returns the watchdog handle and a kicker for the daemon's main loop.
    ///
    /// The `emergency_action` closure is called if the watchdog times out.
    /// It runs on the watchdog's OS thread, so it must not depend on the
    /// tokio runtime being responsive (use bridge threads for async calls).
    pub fn start<F>(config: WatchdogConfig, emergency_action: F) -> (Self, WatchdogKicker)
    where
        F: FnOnce() + Send + 'static,
    {
        let timeout = config.timeout.max(MIN_WATCHDOG_TIMEOUT);
        let disarmed = Arc::new(AtomicBool::new(false));

        // Bounded(1) — at most one pending kick. Extra kicks are coalesced.
        let (kick_tx, kick_rx) = mpsc::sync_channel::<()>(1);

        let wd_disarmed = disarmed.clone();
        let thread = std::thread::Builder::new()
            .name("daq-watchdog".into())
            .spawn(move || {
                Self::watchdog_loop(wd_disarmed, kick_rx, timeout, emergency_action);
            })
            .expect("Failed to spawn watchdog thread");

        let kicker = WatchdogKicker {
            kick_tx: kick_tx.clone(),
            disarmed: disarmed.clone(),
        };

        let watchdog = Self {
            thread: Some(thread),
            disarmed,
            kick_tx,
        };

        (watchdog, kicker)
    }

    /// The watchdog's main loop, running on a dedicated OS thread.
    fn watchdog_loop<F>(
        disarmed: Arc<AtomicBool>,
        kick_rx: mpsc::Receiver<()>,
        timeout: Duration,
        emergency_action: F,
    ) where
        F: FnOnce(),
    {
        tracing::info!(
            timeout_secs = timeout.as_secs(),
            "Hardware watchdog started (thread: daq-watchdog)"
        );

        loop {
            // Check disarmed before blocking
            if disarmed.load(Ordering::SeqCst) {
                tracing::info!("Watchdog disarmed — exiting");
                return;
            }

            match kick_rx.recv_timeout(timeout) {
                Ok(()) => {
                    // Kick received — check if we should exit
                    if disarmed.load(Ordering::SeqCst) {
                        tracing::info!("Watchdog disarmed after kick — exiting");
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if disarmed.load(Ordering::SeqCst) {
                        tracing::info!(
                            "Watchdog timeout but disarmed — exiting (graceful shutdown)"
                        );
                        return;
                    }

                    tracing::error!(
                        timeout_secs = timeout.as_secs(),
                        "WATCHDOG TIMEOUT: No kick received! Firing emergency action"
                    );
                    emergency_action();
                    return;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    if disarmed.load(Ordering::SeqCst) {
                        tracing::info!("Watchdog channel disconnected (disarmed) — exiting");
                    } else {
                        tracing::warn!(
                            "Watchdog channel disconnected unexpectedly — all kickers dropped"
                        );
                    }
                    return;
                }
            }
        }
    }

    /// Stop the watchdog thread gracefully.
    ///
    /// Disarms the watchdog and sends a wake-up kick so the thread exits
    /// without waiting for the full timeout period. Uses a bounded wait
    /// (5 seconds) to avoid blocking indefinitely if the emergency action hangs.
    pub fn stop(mut self) {
        self.disarmed.store(true, Ordering::SeqCst);
        // Wake the thread so it sees the disarmed flag immediately
        let _ = self.kick_tx.try_send(());
        if let Some(thread) = self.thread.take() {
            // Use a bounded wait to avoid blocking forever if the thread hangs
            // (e.g., emergency_action is stuck). Spawn a helper thread that
            // performs the join, then wait on it with a timeout via channel.
            let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<()>(1);
            std::thread::Builder::new()
                .name("daq-watchdog-join".into())
                .spawn(move || {
                    let _ = thread.join();
                    let _ = done_tx.send(());
                })
                .expect("Failed to spawn watchdog join helper");

            match done_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(()) => {}
                Err(_) => {
                    tracing::warn!("Watchdog thread did not exit within 5s — detaching");
                }
            }
        }
    }
}

impl Drop for HardwareWatchdog {
    fn drop(&mut self) {
        self.disarmed.store(true, Ordering::SeqCst);
        // Wake the thread if possible so it exits promptly
        let _ = self.kick_tx.try_send(());
        // Note: we don't join here to avoid blocking in Drop.
        // The thread will exit on the next iteration when it sees disarmed=true.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_watchdog_normal_operation() {
        // Watchdog with 500ms timeout, kicked every 100ms — should NOT fire
        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();

        let (watchdog, kicker) = HardwareWatchdog::start(
            WatchdogConfig {
                timeout: Duration::from_millis(500),
            },
            move || {
                fired_clone.store(true, Ordering::SeqCst);
            },
        );

        // Kick 5 times over 500ms
        for _ in 0..5 {
            assert!(kicker.kick());
            std::thread::sleep(Duration::from_millis(100));
        }

        // Disarm and stop
        watchdog.stop();
        assert!(
            !fired.load(Ordering::SeqCst),
            "Watchdog should not fire when kicked regularly"
        );
    }

    #[test]
    fn test_watchdog_fires_on_timeout() {
        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();

        let (_watchdog, _kicker) = HardwareWatchdog::start(
            WatchdogConfig {
                timeout: Duration::from_secs(1),
            },
            move || {
                fired_clone.store(true, Ordering::SeqCst);
            },
        );

        // Don't kick — wait for timeout + buffer
        std::thread::sleep(Duration::from_millis(1500));

        assert!(
            fired.load(Ordering::SeqCst),
            "Watchdog should fire after timeout"
        );
    }

    #[test]
    fn test_watchdog_disarm_prevents_firing() {
        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();

        let (watchdog, kicker) = HardwareWatchdog::start(
            WatchdogConfig {
                timeout: Duration::from_secs(1),
            },
            move || {
                fired_clone.store(true, Ordering::SeqCst);
            },
        );

        // Disarm before timeout
        kicker.disarm();
        assert!(kicker.is_disarmed());

        // Wait past timeout
        std::thread::sleep(Duration::from_millis(1500));

        assert!(
            !fired.load(Ordering::SeqCst),
            "Disarmed watchdog should not fire"
        );

        watchdog.stop();
    }

    #[test]
    fn test_watchdog_stop_is_clean() {
        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();

        let (watchdog, kicker) = HardwareWatchdog::start(
            WatchdogConfig {
                timeout: Duration::from_secs(30),
            },
            move || {
                fired_clone.store(true, Ordering::SeqCst);
            },
        );

        kicker.kick();
        // stop() should return promptly without waiting 30s
        watchdog.stop();

        assert!(
            !fired.load(Ordering::SeqCst),
            "Stopped watchdog should not fire"
        );
    }

    #[test]
    fn test_kicker_clone() {
        let (_watchdog, kicker) = HardwareWatchdog::start(
            WatchdogConfig {
                timeout: Duration::from_secs(30),
            },
            || {},
        );

        let kicker2 = kicker.clone();
        assert!(kicker.kick());
        assert!(kicker2.kick());

        kicker.disarm();
        assert!(
            kicker2.is_disarmed(),
            "Disarm should be visible across clones"
        );
    }

    #[test]
    fn test_min_timeout_clamp() {
        // Timeout below MIN_WATCHDOG_TIMEOUT should be clamped
        let fired = Arc::new(AtomicBool::new(false));
        let fired_clone = fired.clone();

        let (watchdog, kicker) = HardwareWatchdog::start(
            WatchdogConfig {
                timeout: Duration::from_millis(10), // below 1s minimum
            },
            move || {
                fired_clone.store(true, Ordering::SeqCst);
            },
        );

        // Kick within 1s (the clamped minimum)
        kicker.kick();
        std::thread::sleep(Duration::from_millis(500));
        kicker.kick();

        watchdog.stop();
        assert!(
            !fired.load(Ordering::SeqCst),
            "Watchdog should use clamped timeout"
        );
    }
}
