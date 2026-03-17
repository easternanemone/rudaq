//! Frame loss metrics and error tracking for PVCAM acquisition.

use super::AcquisitionError;
use super::PvcamAcquisition;
use std::sync::atomic::Ordering;

impl PvcamAcquisition {
    /// Reset frame loss metrics at the start of a new acquisition.
    pub fn reset_frame_loss_metrics(&self) {
        self.lost_frames.store(0, Ordering::SeqCst);
        self.discontinuity_events.store(0, Ordering::SeqCst);
        self.dropped_frames.store(0, Ordering::SeqCst);
        #[cfg(feature = "pvcam_sdk")]
        {
            self.last_hardware_frame_nr.store(-1, Ordering::SeqCst);
            // Reset callback context state (bd-ek9n.2)
            self.callback_context.reset();
        }
    }

    /// Get the current frame loss statistics.
    ///
    /// Returns a tuple of (lost_frames, discontinuity_events, dropped_frames).
    pub fn frame_loss_stats(&self) -> (u64, u64, u64) {
        (
            self.lost_frames.load(Ordering::Relaxed),
            self.discontinuity_events.load(Ordering::Relaxed),
            self.dropped_frames.load(Ordering::Relaxed),
        )
    }

    /// Get the number of frames dropped due to pool exhaustion (bd-dmbl).
    ///
    /// This counter is incremented when the buffer pool is exhausted and
    /// a frame must be dropped to maintain real-time performance.
    pub fn dropped_frame_count(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    /// Check if an error occurred during acquisition (bd-g9po).
    ///
    /// Returns true if the last acquisition ended due to an error rather than
    /// a normal stop. Use `last_error()` to get details.
    pub fn has_error(&self) -> bool {
        self.last_error
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// Get the last acquisition error, if any (bd-g9po).
    ///
    /// Returns the error type from the last failed acquisition. Errors are
    /// set when the frame loop exits due to SDK failures or timeouts.
    pub fn last_error(&self) -> Option<AcquisitionError> {
        self.last_error.lock().ok().and_then(|guard| *guard)
    }

    /// Clear the error state (bd-g9po).
    ///
    /// Call this before retrying an operation after an error, or as part of
    /// driver reinitialization.
    pub fn clear_error(&self) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = None;
        }
    }
}
