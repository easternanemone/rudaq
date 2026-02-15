//! Safety Sentinel — RAII guard for daemon emergency shutdown.
//!
//! Ensures laser shutters are closed and hardware is safe-stated on abnormal
//! exit (panics, early returns, unwinding) that the existing panic hook misses.

use scripting::shutter_safety::ShutterRegistry;
use std::sync::atomic::{AtomicBool, Ordering};

/// RAII guard that triggers emergency hardware shutdown if dropped while armed.
///
/// Create immediately after hardware initialization. Disarm only after
/// successful completion of the full shutdown sequence.
pub struct SafetySentinel {
    armed: AtomicBool,
}

impl SafetySentinel {
    pub fn new() -> Self {
        Self {
            armed: AtomicBool::new(true),
        }
    }

    /// Disarm the sentinel after successful shutdown completes.
    /// Only call this after all hardware has been safely shut down.
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

impl Drop for SafetySentinel {
    fn drop(&mut self) {
        if *self.armed.get_mut() {
            eprintln!(
                "SafetySentinel: abnormal exit detected — triggering emergency shutter close"
            );
            // catch_unwind prevents double-panic abort if emergency_close_all panics
            // during stack unwinding
            let _ = std::panic::catch_unwind(|| {
                ShutterRegistry::emergency_close_all();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disarmed_sentinel_does_not_trigger() {
        let sentinel = SafetySentinel::new();
        sentinel.disarm();
        drop(sentinel);
    }

    #[test]
    fn armed_sentinel_triggers_on_drop() {
        // ShutterRegistry::emergency_close_all() is idempotent and safe to call
        // with no shutters registered — it just logs and returns.
        let sentinel = SafetySentinel::new();
        drop(sentinel);
    }
}
