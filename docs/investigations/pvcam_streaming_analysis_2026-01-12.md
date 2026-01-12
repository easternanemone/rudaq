# PVCAM Streaming Investigation - Comprehensive Analysis

**Date**: 2026-01-12
**Camera**: Photometrics Prime BSI
**Firmware Issue**: Rejects CIRC_OVERWRITE mode (error 185)
**Current Status**: Callbacks stop intermittently after 4-8 frames

---

## Executive Summary

The PVCAM driver successfully implements callback-based streaming with the CIRC_NO_OVERWRITE fallback pattern. The callback deregister fix (bd-nzcq-callback-rereg) resolved the initial registration corruption issue. However, EOF callbacks still stop firing intermittently after 4-8 frames, causing timeouts and involuntary acquisition stops.

---

## Fix #1: Callback Deregister (COMPLETE)

### Problem
During CIRC_OVERWRITE → CIRC_NO_OVERWRITE fallback, re-registering EOF callbacks without deregistering first caused PVCAM internal state corruption, manifesting as callbacks stopping after ~5 frames on first streaming attempt.

### Root Cause
The Rust code was calling `pl_cam_register_callback_ex3()` twice during fallback:
1. Before `pl_exp_setup_cont(CIRC_OVERWRITE)` - succeeded
2. After fallback to CIRC_NO_OVERWRITE - **re-registered without deregistering first**

SDK examples (LiveImage.cpp) only register callbacks ONCE per session.

### Solution Applied
**File**: `crates/daq-driver-pvcam/src/components/acquisition.rs`
**Lines**: 1664-1690

```rust
// CRITICAL FIX (bd-nzcq-callback-rereg): Deregister callback before re-registering.
// Re-registering without deregistering first causes PVCAM internal state corruption
// that manifests as callbacks stopping after ~5 frames. The SDK examples only
// register callbacks ONCE and never re-register during a session.
if use_callback {
    pl_cam_deregister_callback(h, PL_CALLBACK_EOF);
    tracing::info!("Deregistered EOF callback before fallback re-registration");
}

// Re-register callback after fallback setup
if use_callback {
    let result = pl_cam_register_callback_ex3(
        h,
        PL_CALLBACK_EOF,
        pvcam_eof_callback as *mut std::ffi::c_void,
        callback_ctx_ptr as *mut std::ffi::c_void,
    );
    if result == 0 {
        tracing::warn!(
            "Failed to re-register EOF callback after fallback: {}",
            get_pvcam_error()
        );
    } else {
        tracing::info!("EOF callback re-registered after fallback setup");
    }
}
```

### Validation
**Commit**: `7f665340` - fix(pvcam): deregister EOF callback before re-registration during fallback

**Test Results** (2026-01-12 02:52-02:53):
- ✅ Fix log messages appeared: "Deregistered EOF callback before fallback re-registration"
- ✅ Callbacks no longer stuck at exactly 5 frames
- ⚠️ Callbacks still stop intermittently after 4-8 frames

---

## Problem #2: Intermittent Callback Stops (ACTIVE)

### Observed Behavior

**First Streaming Attempt** (02:52:50):
```
[CALLBACK] Frame 1 ready, timestamp=403492
[CALLBACK] Frame 2 ready, timestamp=404484
[CALLBACK] Frame 3 ready, timestamp=405484
[CALLBACK] Frame 4 ready, timestamp=406484
[INFO] Frame loop iteration start iter=5 ... callback_pending=0
[INFO] Callback wait completed iter=5 pending_after_wait=0 wait_ms=2000  ← TIMEOUT
[INFO] Callback wait completed iter=6 pending_after_wait=0 wait_ms=2000  ← TIMEOUT
[INFO] Callback wait completed iter=7 pending_after_wait=0 wait_ms=348   ← TIMEOUT
[DEBUG] Frame loop exited: iter=7, streaming=false, shutdown=false
[INFO] PVCAM acquisition ended: 4 frames captured (no frame loss detected)
```

**Second Streaming Attempt** (02:53:05):
```
[CALLBACK] Frame 1 ready, timestamp=557614
[CALLBACK] Frame 2 ready, timestamp=558607
[CALLBACK] Frame 3 ready, timestamp=559607
[CALLBACK] Frame 4 ready, timestamp=560607
[CALLBACK] Frame 5 ready, timestamp=561608  ← Continued past frame 4!
[CALLBACK] Frame 6 ready, timestamp=562608
[CALLBACK] Frame 7 ready, timestamp=563608
[CALLBACK] Frame 8 ready, timestamp=564609
[INFO] Client disconnected from frame stream device_id=prime_bsi
[DEBUG] Frame loop exited: iter=9, streaming=false, shutdown=false
[INFO] PVCAM acquisition ended: 8 frames captured (no frame loss detected)
```

### Analysis

1. **Callbacks Fire Correctly Initially**: Both attempts show callbacks firing at ~100ms intervals (100ms exposure time)

2. **Intermittent Failure**: First attempt stopped after frame 4, second attempt reached frame 8

3. **No PVCAM Errors**: SDK status remains `3` (READOUT_IN_PROGRESS), no error codes

4. **Timeout Logic**: After 5 consecutive 2-second timeouts (10 seconds total), the frame loop breaks with:
   ```rust
   if consecutive_timeouts >= max_consecutive_timeouts {
       tracing::warn!("Frame loop: max consecutive timeouts reached");
       let _ = error_tx.send(AcquisitionError::Timeout);
       break;
   }
   ```

### Comparison with SDK Ground Truth

**SDK LiveImage_CIRC_NO_OVERWRITE.cpp** (created 2026-01-11):
- ✅ All 20 frames acquired successfully
- ✅ Callbacks fired continuously: 0.008-0.038 ms duration
- ✅ `pl_exp_get_oldest_frame`: 2-3 microseconds
- ✅ `pl_exp_unlock_oldest_frame`: 0-1 microseconds
- ✅ No discontinuities, no stalls

**Rust Implementation**:
- ⚠️ Callbacks stop intermittently after 4-8 frames
- ✅ Callback pattern matches SDK (signal-only in CIRC_NO_OVERWRITE mode)
- ✅ Main loop uses `pl_exp_get_oldest_frame` + `pl_exp_unlock_oldest_frame`
- ✅ Single-frame drain mode for CIRC_NO_OVERWRITE + callbacks

---

## Hypotheses for Investigation

### Hypothesis 1: Frame Retrieval Timing
The Rust code waits for callbacks then retrieves frames. If frame retrieval is too slow, PVCAM's internal circular buffer might fill up, blocking further callbacks.

**Evidence**:
- Frame retrieval takes ~0.1-5ms (varies)
- SDK example has minimal processing in both callback and main loop
- Buffer count always shows `0` in logs

**Next Steps**:
- Add timing measurements for complete callback → retrieve → process cycle
- Compare with SDK timings
- Check if buffer is actually filling

### Hypothesis 2: Threading/Async Interaction
The frame loop runs in a spawned thread (`std::thread::spawn`), callbacks fire from PVCAM's internal thread, and the main code is async Rust. There might be a synchronization issue.

**Evidence**:
- Callbacks use `Mutex` and `Condvar` for signaling
- Frame loop is pure blocking code (no async)
- Second attempt worked better (environmental timing difference?)

**Next Steps**:
- Review thread synchronization in `CallbackContext`
- Check if mutex contention could block PVCAM callback thread
- Verify callback never blocks waiting for Rust code

### Hypothesis 3: Circular Buffer Management
CIRC_NO_OVERWRITE requires explicit frame unlocking. If frames aren't unlocked fast enough, the buffer fills and PVCAM might stop callbacks.

**Evidence**:
- Code calls `pl_exp_unlock_oldest_frame` after each `pl_exp_get_oldest_frame`
- SDK example does the same
- Buffer shows 32 frames, 8MB each, 256MB total

**Next Steps**:
- Verify unlock is actually happening for every frame
- Check PVCAM buffer state when callbacks stop
- Add debug logging for unlock failures

### Hypothesis 4: Hardware/USB Communication
USB communication glitch or camera firmware issue causing sporadic callback interruption.

**Evidence**:
- Problem is intermittent (works sometimes, fails other times)
- Hardware is working (SDK examples work perfectly)
- Same camera, same configuration

**Next Steps**:
- Run longer tests with SDK example (100+ frames)
- Check USB error logs during Rust streaming
- Monitor system load during streaming

---

## Code Locations

### Callback Implementation
**File**: `crates/daq-driver-pvcam/src/components/acquisition.rs`
**Lines**: 382-443

```rust
pub unsafe extern "system" fn pvcam_eof_callback(
    p_frame_info: *const FRAME_INFO,
    p_context: *mut std::ffi::c_void,
) {
    // ... store frame info ...

    // CIRC_NO_OVERWRITE: Do NOT call get_latest_frame
    if ctx.circ_overwrite.load(Ordering::Acquire) {
        // CIRC_OVERWRITE path (not used after fallback)
    } else {
        // CIRC_NO_OVERWRITE: Just signal
        ctx.store_frame_ptr(std::ptr::null_mut());
    }

    // Signal main thread
    ctx.signal_frame_ready(frame_nr);
}
```

### Frame Loop
**File**: `crates/daq-driver-pvcam/src/components/acquisition.rs`
**Lines**: 2454-2548

```rust
// Wait for callback
let has_frames = if use_callback {
    let pending = callback_ctx.wait_for_frames(CALLBACK_WAIT_TIMEOUT_MS);
    pending > 0
} else {
    // Polling fallback
};

if !has_frames {
    consecutive_timeouts += 1;
    if consecutive_timeouts >= max_consecutive_timeouts {
        tracing::warn!("Frame loop: max consecutive timeouts reached");
        let _ = error_tx.send(AcquisitionError::Timeout);
        break;  // Exit loop after 5 timeouts (10 seconds)
    }
    continue;
}
```

### Timeout Configuration
**File**: `crates/daq-driver-pvcam/src/components/acquisition.rs`
**Line**: 2382

```rust
let max_consecutive_timeouts: u32 = 5; // 10 seconds total (5 × 2sec timeouts)
```

**Line**: 139
```rust
pub const CALLBACK_WAIT_TIMEOUT_MS: u32 = 2000; // 2 seconds per wait
```

---

## Recommended Next Steps

1. **Add Comprehensive Timing Instrumentation**
   - Measure callback → signal → wait → retrieve → unlock full cycle
   - Compare timing between first and second attempts
   - Identify if there's a performance cliff

2. **Extended SDK Testing**
   - Run LiveImage_CIRC_NO_OVERWRITE.cpp for 100+ frames
   - Verify hardware doesn't have intermittent issues
   - Test multiple start/stop cycles in C++

3. **Thread Synchronization Review**
   - Audit `CallbackContext` mutex usage
   - Verify callback thread never blocks
   - Check if Condvar wake-ups are reliable

4. **Buffer State Diagnostics**
   - Log PVCAM buffer count when callbacks stop
   - Verify frames are being unlocked
   - Check if internal buffer state is consistent

5. **USB/System Monitoring**
   - Monitor USB errors during streaming
   - Check system load and interrupts
   - Test on different USB ports/hubs

---

## Test Environment

**Hardware**:
- Camera: Photometrics Prime BSI (pvcamUSB_0)
- Connection: USB 3.0
- Machine: maitai (100.117.5.12)

**Software**:
- PVCAM SDK: 7.1.1.118
- Rust: 1.89.0
- Daemon Build: 2026-01-12 20:41:32 (with callback deregister fix)

**Configuration**:
- ROI: 2048x2048 (full sensor)
- Binning: 1x1
- Exposure: 100ms
- Buffer: 32 frames, 256MB total

---

## Related Documentation

- **ADR**: `docs/architecture/adr-pvcam-continuous-acquisition.md` (lines 153-256) - SDK callback patterns
- **SDK Examples**: `/tmp/pvcam_debug/LiveImage_CIRC_NO_OVERWRITE.cpp` - Ground truth implementation
- **Previous Analysis**: `/tmp/pvcam_callback_fix_summary.md` - Callback registration fix
- **Commit**: `7f665340` - fix(pvcam): deregister EOF callback before re-registration during fallback

---

## Issue Tracking

- **bd-nzcq-callback-rereg**: Callback re-registration fix (COMPLETE)
- **bd-[TBD]**: Intermittent callback stops (ACTIVE - needs new issue)
