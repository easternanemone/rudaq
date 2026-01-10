# ADR: PVCAM Continuous Acquisition Mode Selection

**Status:** Accepted
**Date:** 2025-01-09
**Authors:** Investigation by Claude Code with hardware testing on Prime BSI

## Context

The PVCAM SDK provides multiple approaches for continuous frame acquisition from scientific cameras. During implementation of the Prime BSI driver, we encountered error 185 (`PL_ERR_CONFIGURATION_INVALID`) when attempting to use `CIRC_OVERWRITE` buffer mode. This ADR documents our systematic investigation and the rationale for our final implementation choice.

## Decision

Use **`CIRC_NO_OVERWRITE`** buffer mode with **`pl_exp_get_latest_frame_ex()`** for continuous acquisition on Prime BSI cameras.

```rust
// In acquisition.rs
const USE_SEQUENCE_MODE: bool = false;      // Use continuous mode
const USE_GET_LATEST_FRAME: bool = true;    // Use get_latest_frame (not get_oldest_frame)
```

## Investigation Summary

### Phase 1: CIRC_OVERWRITE Testing

We systematically tested all 9 combinations of exposure mode × expose-out mode with `CIRC_OVERWRITE`:

| exp_mode | expose_out | Combined | Result |
|----------|------------|----------|--------|
| 1792 (Internal Trigger) | 0 (First Row) | 1792 | Error 185 |
| 1792 (Internal Trigger) | 2 (Any Row) | 1794 | Error 185 |
| 1792 (Internal Trigger) | 3 (Rolling Shutter) | 1795 | Error 185 |
| 2304 (Edge Trigger) | 0 (First Row) | 2304 | Error 185 |
| 2304 (Edge Trigger) | 2 (Any Row) | 2306 | Error 185 |
| 2304 (Edge Trigger) | 3 (Rolling Shutter) | 2307 | Error 185 |
| 2048 (Trigger First) | 0 (First Row) | 2048 | Error 185 |
| 2048 (Trigger First) | 2 (Any Row) | 2050 | Error 185 |
| 2048 (Trigger First) | 3 (Rolling Shutter) | 2051 | Error 185 |

**Conclusion:** Prime BSI does NOT support `CIRC_OVERWRITE` mode. All combinations fail at `pl_exp_start_cont()` with error 185.

### Phase 2: CIRC_NO_OVERWRITE with Different Frame Retrieval

After confirming `CIRC_OVERWRITE` doesn't work, we tested `CIRC_NO_OVERWRITE` with different frame retrieval strategies:

| Buffer Mode | Retrieval Method | Unlock Required | Result |
|-------------|------------------|-----------------|--------|
| CIRC_NO_OVERWRITE | `get_oldest_frame` + `unlock_oldest_frame` | Yes | Stalls after ~85 frames |
| CIRC_NO_OVERWRITE | `get_latest_frame` | No | **Works at ~100 FPS** |

### Phase 3: Frame Timing Semantics Verification

We created a probe test with 500ms exposure to verify the semantic meaning of "oldest" vs "latest":

```
=== Phase 2: Retrieve frames using BOTH methods ===

--- Testing pl_exp_get_oldest_frame_ex ---
  [0] FrameNr=5, TimeStamp=30863, TimeStampBOF=30861
  [1] FrameNr=6, TimeStamp=35867, TimeStampBOF=35864

--- Testing pl_exp_get_latest_frame_ex ---
  [0] FrameNr=6, TimeStamp=35867, TimeStampBOF=35864
  [1] FrameNr=7, TimeStamp=40867, TimeStampBOF=40864

=== ANALYSIS ===
✓ get_oldest_frame returns LOWER FrameNr (5 < 6)
  → 'oldest' = chronologically older (captured earlier)
  → 'latest' = chronologically newer (captured later)

  NAMING IS CHRONOLOGICAL (as expected)
```

**Conclusion:** The naming is chronological, not stack-position based.

## Buffer Mode Comparison

### CIRC_OVERWRITE (Not Supported on Prime BSI)

```
Buffer: [Frame1] [Frame2] [Frame3] [Frame4] ... [FrameN]
                                                   ↑
                                            Overwrites oldest
                                            when buffer full
```

- Frames are overwritten when buffer fills
- Designed for real-time preview where dropping old frames is acceptable
- **NOT SUPPORTED** on Prime BSI (error 185)

### CIRC_NO_OVERWRITE (Supported)

```
Buffer: [Frame1] [Frame2] [Frame3] [Frame4] ... [FrameN]
           ↑                                       ↑
        oldest                                  latest
        (first captured)                    (most recent)
```

- Buffer fills until full, then acquisition pauses until frames are consumed
- Requires frame retrieval to make room for new frames
- Two retrieval strategies available (see below)

## Frame Retrieval Strategies

### Strategy A: get_oldest_frame + unlock (FIFO Queue)

```
Timeline:
  Frame 1 captured → Frame 2 captured → Frame 3 captured
       ↓
  get_oldest → returns Frame 1
  unlock_oldest → removes Frame 1, advances pointer
  get_oldest → returns Frame 2
  ...
```

**Characteristics:**
- FIFO ordering - process frames in capture order
- Must call `pl_exp_unlock_oldest_frame()` after processing each frame
- If processing takes too long, buffer fills and acquisition stalls
- Good for: Applications requiring every frame (no drops allowed)

**Why it stalled:** At high frame rates, the unlock-acquire cycle timing can fall behind, causing the buffer to fill.

### Strategy B: get_latest_frame (Newest-Wins)

```
Timeline:
  Frame 1 captured → Frame 2 captured → Frame 3 captured
                                              ↓
                              get_latest → returns Frame 3
                              (Frame 1, 2 implicitly skipped)
```

**Characteristics:**
- Always returns the most recently captured frame
- No unlock required - buffer management is automatic
- Frames may be skipped if processing is slower than capture rate
- Good for: Real-time display, streaming, low-latency applications

**Why it works:** The camera continues capturing while we process; we always get the freshest data without explicit buffer management.

## Implementation

### SDK-Matching Callback Pattern (bd-ffi-sdk-match)

**Update 2025-01:** The implementation now matches the official SDK examples (`LiveImage.cpp`, `FastStreamingToDisk.cpp`) by retrieving frame pointers **inside** the EOF callback.

#### SDK Example Pattern (from `LiveImage.cpp`)

```cpp
void PV_DECL CustomEofHandler(FRAME_INFO* pFrameInfo, void* pContext) {
    auto ctx = static_cast<CameraContext*>(pContext);
    ctx->eofFrameInfo = *pFrameInfo;

    // CRITICAL: Frame retrieval happens INSIDE the callback
    if (PV_OK != pl_exp_get_latest_frame(ctx->hcam, &ctx->eofFrame)) {
        PrintErrorMessage(pl_error_code(), "pl_exp_get_latest_frame() error");
        ctx->eofFrame = nullptr;
    }

    // Signal main thread
    ctx->eofEvent.cond.notify_all();
}
```

#### Rust Implementation (matching SDK)

```rust
pub unsafe extern "system" fn pvcam_eof_callback(
    p_frame_info: *const FRAME_INFO,
    p_context: *mut std::ffi::c_void,
) {
    let ctx = &*(p_context as *const CallbackContext);

    // SDK Pattern Step 1: Store FRAME_INFO
    if !p_frame_info.is_null() {
        ctx.store_frame_info(*p_frame_info);
    }

    // SDK Pattern Step 2: Retrieve frame pointer INSIDE callback
    let hcam = ctx.hcam.load(Ordering::Acquire);
    let mut frame_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let result = pl_exp_get_latest_frame(hcam, &mut frame_ptr);

    if result != 0 && !frame_ptr.is_null() {
        ctx.store_frame_ptr(frame_ptr);
    }

    // SDK Pattern Step 3: Signal main thread
    ctx.signal_frame_ready(frame_nr);
}
```

### CallbackContext Structure

The `CallbackContext` stores frame data captured by the callback:

```rust
pub struct CallbackContext {
    pub pending_frames: AtomicU32,
    pub latest_frame_nr: AtomicI32,
    pub condvar: Condvar,
    pub mutex: Mutex<bool>,
    pub shutdown: AtomicBool,

    // SDK Pattern Fields (bd-ffi-sdk-match)
    pub hcam: AtomicI16,                    // Camera handle for SDK calls
    pub frame_ptr: AtomicPtr<c_void>,       // Frame pointer (lock-free)
    pub frame_info: Mutex<FRAME_INFO>,      // Frame metadata
}
```

### Frame Retrieval in Drain Loop

The main thread retrieves the stored frame from the callback context:

```rust
// Primary path: Use callback-stored frame (SDK pattern)
let callback_frame_ptr = callback_ctx.take_frame_ptr();

let frame_ptr = if !callback_frame_ptr.is_null() {
    frame_info = callback_ctx.take_frame_info();
    callback_frame_ptr
} else if !circ_overwrite {
    // Fallback for CIRC_NO_OVERWRITE: FIFO drain
    match ffi_safe::get_oldest_frame(hcam, &mut frame_info) {
        Ok(ptr) => ptr,
        Err(()) => break,
    }
} else {
    break;
};
```

### Why This Pattern Works

1. **Timing Precision:** Frame pointer is captured at the exact moment PVCAM signals EOF
2. **Thread Safety:** `AtomicPtr` provides lock-free storage from callback thread
3. **SDK Alignment:** Matches official examples (LiveImage.cpp, FastStreamingToDisk.cpp)
4. **Fallback Support:** CIRC_NO_OVERWRITE mode can still use FIFO draining if needed

## PyVCAM Reference

Our solution aligns with PyVCAM's implementation (the official Python wrapper):

```cpp
// From PyVCAM pvcmodule.cpp
// PyVCAM uses get_latest_frame_ex in its callback, not get_oldest_frame
void callback_handler(FRAME_INFO* frame_info, void* context) {
    void* address;
    FRAME_INFO fi;
    if (pl_exp_get_latest_frame_ex(hcam, &address, &fi) == PV_OK) {
        // Process frame...
    }
}
```

Key PyVCAM patterns we adopted:
1. Use `pl_exp_get_latest_frame_ex()` for frame retrieval
2. No unlock calls needed with `get_latest_frame`
3. Register callback AFTER `pl_exp_setup_cont()`
4. 4096-byte aligned buffers (optional optimization)

## Test Files

The investigation produced several diagnostic test files:

| Test File | Purpose |
|-----------|---------|
| `tests/exp_mode_probe.rs` | Systematic test of all 9 exp_mode × expose_out combinations |
| `tests/pyvcam_style_probe.rs` | PyVCAM-style test with aligned buffers |
| `tests/frame_timing_probe.rs` | Verifies oldest/latest semantic meaning |
| `tests/circ_buffer_diagnostic.rs` | Original diagnostic test (17 scenarios) |

Run on maitai with:
```bash
ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
  export PVCAM_SDK_DIR=/opt/pvcam/sdk && \
  export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
  export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
  cd ~/rust-daq && cargo test --release -p daq-driver-pvcam --features pvcam_hardware \
    --test <test_name> -- --nocapture --test-threads=1'
```

## Performance Results

With `CIRC_NO_OVERWRITE` + `get_latest_frame`:
- **Frame rate:** ~100 FPS sustained
- **Test duration:** 2 seconds
- **Frames captured:** 199
- **Errors:** 0
- **ROI:** 256×256 (test), full sensor supported

## Consequences

### Positive
- Reliable continuous acquisition at high frame rates
- No buffer stalls or timing-dependent failures
- Aligns with PyVCAM reference implementation
- Simpler code (no unlock management needed)

### Negative
- Frames may be skipped under heavy load (acceptable for streaming)
- Cannot guarantee every frame is processed (use sequence mode if needed)
- `CIRC_OVERWRITE` mode unavailable (hardware limitation)

### Neutral
- Different cameras may have different mode support
- This decision is specific to Prime BSI; other cameras should be tested

## References

- [PVCAM SDK Documentation](https://www.photometrics.com/support/software/) - Teledyne Vision Solutions
- [PyVCAM Source Code](https://github.com/Photometrics/PyVCAM) - Official Python wrapper
- Prime BSI Camera Manual - GS2020 sensor specifications
- Test results from maitai@100.117.5.12 (January 2025)
