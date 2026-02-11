//! Callback context for PVCAM EOF notifications (bd-ek9n.2)
//!
//! Manages the callback context and EOF callback function for frame-ready
//! signaling from the PVCAM SDK.

#[cfg(feature = "pvcam_sdk")]
use pvcam_sys::*;
#[cfg(feature = "pvcam_sdk")]
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicI32, AtomicPtr, AtomicU32, Ordering};
#[cfg(feature = "pvcam_sdk")]
use std::time::Duration;
#[cfg(feature = "pvcam_sdk")]
use tokio::time::timeout;

/// bd-3gnv: Prefer continuous FIFO streaming; keep sequence mode as a last-resort fallback.
/// Sequence mode is slower but can be toggled for diagnostics if continuous mode regresses.
#[cfg(feature = "pvcam_sdk")]
pub(super) const USE_SEQUENCE_MODE: bool = false;

/// Batch size for sequence mode streaming (bd-3gnv).
///
/// Smaller batches = lower latency, more restarts
/// Larger batches = higher throughput, less restart overhead
///
/// 10 frames at 10ms exposure = ~150ms batch time (balances latency + throughput)
#[cfg(feature = "pvcam_sdk")]
pub(super) const SEQUENCE_BATCH_SIZE: u16 = 10;

/// Callback context for EOF notifications (bd-ek9n.2)
///
/// This structure is passed to the PVCAM callback and shared with the frame
/// retrieval loop. The callback increments `pending_frames` and notifies the condvar;
/// the loop waits on the condvar and drains all available frames.
///
/// Uses AtomicU32 counter instead of AtomicBool to avoid losing events when
/// multiple EOF callbacks fire while the loop is processing.
///
/// # Safety and Lifetime Contract
///
/// This struct MUST remain valid for the lifetime of the acquisition and MUST
/// outlive the camera handle it is associated with. Violating this contract
/// causes undefined behavior (use-after-free in the PVCAM callback).
///
/// **Lifetime Guarantee:**
/// - Wrapped in `Arc<Pin<Box<CallbackContext>>>` to prevent moves and ensure stable address
/// - Arc stored as a field in `PvcamAcquisition` to tie lifetime to the acquisition component
/// - `PvcamAcquisition::Drop` waits for poll thread exit, stops camera, and deregisters
///   callback BEFORE the Arc is dropped, ensuring PVCAM cannot access freed memory
///
/// **Callback Safety:**
/// - Passed to PVCAM SDK as raw pointer via `GLOBAL_CALLBACK_CTX` static
/// - The callback (`pvcam_eof_callback`) dereferences this pointer on each frame
/// - PVCAM has no mechanism to signal "context is now invalid", so we must ensure
///   the callback is deregistered (`pl_cam_deregister_callback`) before the context
///   is freed
///
/// **Drop Ordering (CRITICAL):**
/// 1. Set shutdown flag and signal callback context to wake waiters
/// 2. Wait for poll thread to exit completely (prevents SDK races)
/// 3. Call `pl_exp_stop_cont` to halt camera operation
/// 4. Call `pl_cam_deregister_callback` to remove callback registration
/// 5. Clear `GLOBAL_CALLBACK_CTX` to null out the static pointer
/// 6. Arc drops, freeing the context (only after SDK can no longer access it)
#[cfg(feature = "pvcam_sdk")]
pub struct CallbackContext {
    /// Count of pending frames (incremented by callback, decremented by consumer)
    pub pending_frames: std::sync::atomic::AtomicU32,
    /// Latest frame info from callback (informational, not for loss detection)
    pub latest_frame_nr: AtomicI32,
    /// Notify for efficient async waiting (bd-8hlm)
    /// Replaces Condvar/Mutex to avoid blocking the async runtime
    pub notify: tokio::sync::Notify,
    /// Shutdown signal to exit the wait loop
    pub shutdown: AtomicBool,

    // === SDK Pattern Fields (bd-ffi-sdk-match) ===
    // These fields enable calling pl_exp_get_latest_frame INSIDE the callback,
    // matching the SDK examples (LiveImage.cpp, FastStreamingToDisk.cpp).
    /// Camera handle for callback to call pl_exp_get_latest_frame
    pub hcam: AtomicI16,
    /// Frame pointer retrieved in callback (SDK pattern: ctx->eofFrame)
    /// AtomicPtr provides lock-free access from callback thread
    pub frame_ptr: AtomicPtr<std::ffi::c_void>,
    /// Frame info from callback (SDK pattern: ctx->eofFrameInfo = *pFrameInfo)
    /// Uses std::sync::Mutex (not tokio) because callback runs on PVCAM thread
    pub frame_info: std::sync::Mutex<FRAME_INFO>,
    /// Buffer mode flag: true = CIRC_OVERWRITE, false = CIRC_NO_OVERWRITE
    /// In CIRC_NO_OVERWRITE mode, callback MUST NOT call get_latest_frame
    /// because main loop needs get_oldest_frame for proper FIFO order (bd-nzcq)
    pub circ_overwrite: AtomicBool,
}

#[cfg(feature = "pvcam_sdk")]
impl CallbackContext {
    /// Create a new CallbackContext with camera handle for SDK pattern frame retrieval.
    ///
    /// # Arguments
    /// * `hcam` - Camera handle from pl_cam_open, used by callback to call pl_exp_get_latest_frame
    pub fn new(hcam: i16) -> Self {
        Self {
            pending_frames: std::sync::atomic::AtomicU32::new(0),
            latest_frame_nr: AtomicI32::new(-1),
            notify: tokio::sync::Notify::new(),
            shutdown: AtomicBool::new(false),
            // SDK pattern fields
            hcam: AtomicI16::new(hcam),
            frame_ptr: AtomicPtr::new(std::ptr::null_mut()),
            // SAFETY: FRAME_INFO is a plain-old-data (POD) C struct from the PVCAM SDK
            // containing only primitive types (i32, u32, etc.) with no pointers, references,
            // or drop semantics. Zero-initialization is valid as all fields accept 0 as a
            // sentinel value meaning "uninitialized" or "no frame yet". The struct is fully
            // overwritten by the SDK when pl_exp_get_oldest_frame_ex populates it.
            frame_info: std::sync::Mutex::new(unsafe { std::mem::zeroed() }),
            // Default to CIRC_OVERWRITE (true); updated by set_circ_overwrite() after fallback
            circ_overwrite: AtomicBool::new(true),
        }
    }

    /// Set the buffer mode flag. Must be called after fallback to CIRC_NO_OVERWRITE (bd-nzcq).
    ///
    /// When circ_overwrite=false, the callback will NOT call get_latest_frame,
    /// forcing the main loop to use get_oldest_frame for proper FIFO order.
    pub fn set_circ_overwrite(&self, overwrite: bool) {
        self.circ_overwrite.store(overwrite, Ordering::Release);
        tracing::debug!(
            circ_overwrite = overwrite,
            "CallbackContext buffer mode updated"
        );
    }

    /// Signal that a frame is ready (called from EOF callback)
    ///
    /// Increments the pending frame counter and notifies waiting threads.
    /// Must lock the mutex to avoid missed wakeups with condvar.
    ///
    /// # PVCAM Callback Reliability Fix (bd-callback-reliability-2026-01-12)
    ///
    /// This method MUST always notify, even if the mutex is poisoned.
    /// Previous implementation silently skipped notification on poisoned mutex,
    /// causing the main loop to wait forever for frames that were already counted.
    #[inline]
    pub fn signal_frame_ready(&self, frame_nr: i32) {
        self.latest_frame_nr.store(frame_nr, Ordering::Release);
        let new_pending = self.pending_frames.fetch_add(1, Ordering::AcqRel) + 1;
        // Trace logging - frequent but useful for diagnosing callback issues
        if frame_nr <= 10 || frame_nr % 50 == 0 {
            tracing::trace!(
                frame_nr,
                new_pending,
                "CallbackContext::signal_frame_ready - incrementing pending_frames"
            );
        }
        // Notify async waiter (bd-8hlm)
        // Notify::notify_one() is non-blocking and safe to call from any context
        self.notify.notify_one();
    }

    /// Wait for frames to be available with timeout
    ///
    /// Returns the number of pending frames (0 on shutdown or timeout).
    /// Does NOT decrement the counter - caller should drain frames and call `consume_one()` for each.
    ///
    /// # PVCAM Callback Reliability Fix (bd-callback-reliability-2026-01-12)
    ///
    /// This method handles poisoned mutex gracefully by using `into_inner()` to
    /// continue operation. This ensures the frame loop doesn't deadlock if the
    /// mutex was poisoned by an earlier panic elsewhere.
    pub async fn wait_for_frames(&self, timeout_ms: u64) -> u32 {
        if self.shutdown.load(Ordering::Acquire) {
            tracing::trace!("wait_for_frames: shutdown requested, returning 0");
            return 0;
        }

        // bd-fast-path-2026-01-17: CRITICAL FIX - Check pending_frames FIRST.
        // Fast path: if pending frames exist, return immediately
        let pending = self.pending_frames.load(Ordering::Acquire);
        if pending > 0 {
            tracing::trace!(
                pending,
                "wait_for_frames: fast path - pending frames available"
            );
            return pending;
        }

        // No pending frames - wait for notification
        // Async wait for notification with timeout
        let timeout_duration = Duration::from_millis(timeout_ms);
        match timeout(timeout_duration, self.notify.notified()).await {
            Ok(_) => {
                // Notified - return pending count
                // If spuriously notified, this returns 0 which is handled by caller
                self.pending_frames.load(Ordering::Acquire)
            }
            Err(_) => {
                tracing::trace!(timeout_ms, "wait_for_frames: timed out");
                0 // Return 0 on timeout
            }
        }
    }
    /// Decrement the pending frames counter after successfully retrieving a frame
    #[inline]
    pub fn consume_one(&self) {
        // Saturating decrement to avoid underflow
        let _ = self
            .pending_frames
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                if n > 0 {
                    Some(n - 1)
                } else {
                    None
                }
            });
    }

    /// Signal shutdown to wake waiting threads
    pub fn signal_shutdown(&self) {
        tracing::debug!(
            pending_frames = self.pending_frames.load(Ordering::Acquire),
            latest_frame_nr = self.latest_frame_nr.load(Ordering::Acquire),
            "CallbackContext::signal_shutdown called"
        );
        self.shutdown.store(true, Ordering::Release);
        // Notify all waiters
        self.notify.notify_waiters();
        tracing::debug!("CallbackContext::signal_shutdown - notify_waiters called");
    }

    /// Reset context state for new acquisition
    pub fn reset(&self) {
        tracing::debug!(
            old_pending = self.pending_frames.load(Ordering::Acquire),
            old_frame_nr = self.latest_frame_nr.load(Ordering::Acquire),
            old_shutdown = self.shutdown.load(Ordering::Acquire),
            "CallbackContext::reset called - clearing state"
        );
        self.pending_frames.store(0, Ordering::SeqCst);
        self.latest_frame_nr.store(-1, Ordering::SeqCst);
        self.shutdown.store(false, Ordering::SeqCst);
        // No need to reset notify manually as it's stateless
        // Reset SDK pattern fields
        self.frame_ptr.store(std::ptr::null_mut(), Ordering::SeqCst);
        if let Ok(mut guard) = self.frame_info.lock() {
            // SAFETY: FRAME_INFO is a POD C struct with only primitive fields (i32, u32, etc.).
            // Zero-initialization resets all fields to sentinel values indicating "no frame".
            // This is safe because the struct has no pointers, references, or drop semantics.
            *guard = unsafe { std::mem::zeroed() };
        }
        tracing::debug!("CallbackContext::reset completed");
    }

    // === SDK Pattern Methods (bd-ffi-sdk-match) ===
    // These methods enable the callback to store frame data and the main thread to retrieve it,
    // matching the SDK examples (LiveImage.cpp, FastStreamingToDisk.cpp).

    /// Store frame info from callback (called from PVCAM thread).
    ///
    /// Uses try_lock to avoid blocking the callback. If the lock is held by the
    /// main thread, we skip storing this frame's info (the frame pointer is still
    /// stored atomically and the main thread can still process the frame).
    #[inline]
    pub fn store_frame_info(&self, info: FRAME_INFO) {
        if let Ok(mut guard) = self.frame_info.try_lock() {
            *guard = info;
        }
        // If lock fails, we're in contention - skip this frame's info
        // Main thread will still get the pointer via store_frame_ptr
    }

    /// Store frame pointer from callback (called from PVCAM thread).
    ///
    /// This is lock-free and always succeeds. The frame pointer is retrieved
    /// immediately in the callback using pl_exp_get_latest_frame (SDK pattern).
    #[inline]
    pub fn store_frame_ptr(&self, ptr: *mut std::ffi::c_void) {
        self.frame_ptr.store(ptr, Ordering::Release);
    }

    /// Take stored frame pointer (called from main thread).
    ///
    /// Returns the frame pointer and resets it to null. This ensures each frame
    /// pointer is only consumed once. Returns null if no frame is available.
    #[inline]
    pub fn take_frame_ptr(&self) -> *mut std::ffi::c_void {
        self.frame_ptr.swap(std::ptr::null_mut(), Ordering::Acquire)
    }

    /// Take stored frame info (called from main thread).
    ///
    /// Returns a copy of the FRAME_INFO stored by the callback.
    /// Note: This does NOT reset the stored info (unlike take_frame_ptr).
    #[inline]
    pub fn take_frame_info(&self) -> FRAME_INFO {
        match self.frame_info.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    /// Update the camera handle (called before callback registration).
    ///
    /// The CallbackContext is created with -1 (invalid) as initial hcam.
    /// This method must be called with the real camera handle before
    /// registering the EOF callback, so pl_exp_get_latest_frame can work.
    #[inline]
    pub fn set_hcam(&self, hcam: i16) {
        self.hcam.store(hcam, Ordering::Release);
    }
}

/// Global callback context used by the PVCAM EOF callback.
///
/// bd-static-ctx-2026-01-12: We use a global static pointer rather than p_context
/// because the SDK appears to stop calling the callback when the context pointer
/// points to a non-static address.
///
/// Must be set before callback registration and cleared after deregistration.
#[cfg(feature = "pvcam_sdk")]
pub static GLOBAL_CALLBACK_CTX: std::sync::atomic::AtomicPtr<CallbackContext> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Set the global callback context pointer (call before registering callback)
#[cfg(feature = "pvcam_sdk")]
pub fn set_global_callback_ctx(ctx: *const CallbackContext) {
    GLOBAL_CALLBACK_CTX.store(
        ctx as *mut CallbackContext,
        std::sync::atomic::Ordering::Release,
    );
}

/// Clear the global callback context pointer (call after deregistering callback)
#[cfg(feature = "pvcam_sdk")]
pub fn clear_global_callback_ctx() {
    GLOBAL_CALLBACK_CTX.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
}

/// PVCAM EOF (End-of-Frame) callback invoked by the SDK when a frame is ready.
///
/// # Safety
///
/// This function is called from C code (PVCAM SDK) and must:
/// - Never unwind (panic) across the FFI boundary - this is Undefined Behavior
/// - Be reentrant-safe (called from PVCAM's internal thread)
/// - Complete quickly to avoid blocking the camera's frame pipeline
///
/// The entire callback body is wrapped in `catch_unwind` to prevent any panic
/// from unwinding into C code. If a panic occurs (e.g., mutex poisoning, formatting
/// error in eprintln!), it is caught and silently discarded. The frame will be
/// missed, but the process won't crash with UB.
#[cfg(feature = "pvcam_sdk")]
pub unsafe extern "system" fn pvcam_eof_callback(
    p_frame_info: *const FRAME_INFO,
    _p_context: *mut std::ffi::c_void, // bd-static-ctx-2026-01-12: IGNORED - use static global instead
) {
    // SAFETY: catch_unwind prevents panics from unwinding across FFI boundary (bd-ga6p.3).
    // AssertUnwindSafe is needed because raw pointers are not UnwindSafe, but we
    // acknowledge this is an FFI callback where we must handle any panic gracefully.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // bd-debug-2026-01-12: Trace callback entry BEFORE any checks
        static CALLBACK_ENTRY_COUNT: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(0);
        let entry_count =
            CALLBACK_ENTRY_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

        // bd-static-ctx-2026-01-12: Use static global context like minimal test
        // This avoids the SDK p_context issue that stops callbacks after ~19 frames
        let ctx_ptr = GLOBAL_CALLBACK_CTX.load(std::sync::atomic::Ordering::Acquire);

        if entry_count <= 25 || entry_count % 50 == 0 {
            eprintln!(
                "[PVCAM CALLBACK ENTRY] #{}, static_ctx={:?}",
                entry_count, ctx_ptr
            );
        }

        if ctx_ptr.is_null() {
            eprintln!(
                "[PVCAM CALLBACK] static context is NULL at entry #{}",
                entry_count
            );
            return;
        }

        // SAFETY: Dereferencing the context pointer is safe because:
        // 1. Context is Arc<Pin<Box<>>> - pinned to prevent moves, Arc ensures stable lifetime
        // 2. Context lifetime is tied to PvcamAcquisition via Arc field
        // 3. PvcamAcquisition::Drop deregisters callback BEFORE Arc drops
        // 4. GLOBAL_CALLBACK_CTX is cleared after deregistration, so this code path
        //    cannot execute after the context is freed
        // 5. Debug assertions below verify the pointer is not dangling (release builds skip)
        //
        // If this assertion fires, it means PvcamAcquisition::Drop did not properly
        // deregister the callback or clear GLOBAL_CALLBACK_CTX before the context was freed.
        #[cfg(debug_assertions)]
        {
            // Verify the pointer is properly aligned for CallbackContext
            debug_assert!(
                (ctx_ptr as usize) % std::mem::align_of::<CallbackContext>() == 0,
                "CallbackContext pointer is misaligned: {:p}",
                ctx_ptr
            );
            // Basic sanity check: pending_frames counter should be < 1 million
            // (if it's a huge value, the memory is likely corrupted/freed)
            let pending = (*ctx_ptr)
                .pending_frames
                .load(std::sync::atomic::Ordering::Relaxed);
            debug_assert!(
                pending < 1_000_000,
                "CallbackContext appears corrupted (pending_frames={})",
                pending
            );
        }

        let ctx = &*ctx_ptr;

        // Extract frame number (infallible)
        let frame_nr = if !p_frame_info.is_null() {
            let info = *p_frame_info;

            // Trace callbacks (eprintln is thread-safe, no allocation)
            // Print first 25, then every 50th
            if info.FrameNr <= 25 || info.FrameNr % 50 == 0 {
                eprintln!(
                    "[PVCAM CALLBACK] Frame {} ready, timestamp={}",
                    info.FrameNr, info.TimeStamp
                );
            }

            info.FrameNr
        } else {
            -1
        };

        // Signal main thread - this is the ONLY essential operation
        // In CIRC_NO_OVERWRITE mode, main loop uses get_oldest_frame for retrieval
        ctx.signal_frame_ready(frame_nr);
    }))
    .is_err()
    {
        // Log panic without extracting payload (which could itself panic)
        eprintln!("[PVCAM CALLBACK] Panic caught in EOF callback - frame may be missed");
    }
}
