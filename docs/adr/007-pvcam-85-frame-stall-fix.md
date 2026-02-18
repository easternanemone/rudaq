# ADR: PVCAM 85-Frame Stall Fix

**Date:** 2026-01-11
**Status:** Implemented
**Commit:** 3fce81ba

## Context

The Rust PVCAM driver was stalling after acquiring approximately 85 frames during continuous streaming. A C++ reproduction using the SDK's `LiveImage` example proved the hardware was capable of sustained streaming (500+ frames), isolating the issue to the Rust implementation.

## Problem Analysis

### SDK Pattern Requirement

The PVCAM SDK documentation and example code (LiveImage.cpp, FastStreamingToDisk.cpp) demonstrate a specific pattern for continuous acquisition in CIRC_OVERWRITE mode:

```cpp
// SDK Pattern from LiveImage.cpp
void PV_DECL CustomEofHandler(FRAME_INFO* pFrameInfo, void* pContext) {
    ctx->eofFrameInfo = *pFrameInfo;
    // CRITICAL: Call pl_exp_get_latest_frame INSIDE the callback
    if (PV_OK != pl_exp_get_latest_frame(ctx->hcam, &ctx->eofFrame)) {
        ctx->eofFrame = nullptr;
    }
    ctx->eofEvent.cond.notify_all();
}
```

The key insight is that `pl_exp_get_latest_frame` must be called **inside the EOF callback** when using CIRC_OVERWRITE mode. This call:
1. Retrieves the frame pointer from the SDK's internal buffer
2. **Crucially, clears the SDK's internal buffer state** to allow the next frame to be written

### Root Cause

The Rust implementation was using a "pure signaling" callback pattern that did NOT call `pl_exp_get_latest_frame` inside the callback. Instead, it deferred frame retrieval to the main loop. This worked for individual frames but caused the SDK's internal circular buffer write pointer to stall after approximately 85 frames (the buffer size) because the "consumed" flag was never being set.

## Solution

Restore the SDK pattern by calling `pl_exp_get_latest_frame` inside the EOF callback when in CIRC_OVERWRITE mode.

### Implementation (acquisition.rs:407-426)

```rust
pub unsafe extern "system" fn pvcam_eof_callback(
    p_frame_info: *const FRAME_INFO,
    p_context: *mut std::ffi::c_void,
) {
    let _ = std::panic::catch_unwind(|| {
        // ... frame info handling ...

        // FIX (PVCAM_STALL_INVESTIGATION_2026_01_11):
        // Restore SDK Pattern: Call pl_exp_get_latest_frame INSIDE callback
        // if in OVERWRITE mode. This is crucial for clearing the internal
        // buffer state in CIRC_OVERWRITE mode.
        if ctx.circ_overwrite.load(Ordering::Acquire) {
            let hcam = ctx.hcam.load(Ordering::Acquire);
            if hcam != -1 {
                let mut frame_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                if pl_exp_get_latest_frame(hcam, &mut frame_ptr) != 0 {
                    ctx.store_frame_ptr(frame_ptr);
                } else {
                    ctx.store_frame_ptr(std::ptr::null_mut());
                }
            }
        } else {
            // In CIRC_NO_OVERWRITE mode, do NOT call get_latest_frame.
            // Main loop uses get_oldest_frame + unlock for FIFO order.
            ctx.store_frame_ptr(std::ptr::null_mut());
        }

        ctx.signal_frame_ready(frame_nr);
    });
}
```

### Key Design Decisions

1. **Mode-aware callback**: The callback checks `circ_overwrite` flag to determine behavior
   - CIRC_OVERWRITE: Call `pl_exp_get_latest_frame` in callback (SDK pattern)
   - CIRC_NO_OVERWRITE: Do not call; let main loop use `get_oldest_frame` + `unlock`

2. **Lock-free atomics**: Uses `AtomicBool` for `circ_overwrite` flag to avoid mutex locks in callback context

3. **Panic safety**: Wrapped in `catch_unwind` to prevent undefined behavior if Rust code panics in C callback context

4. **Frame pointer storage**: Uses `AtomicPtr` for lock-free frame pointer exchange between callback and main thread

## Verification

### Test Results

| Test | Frames | FPS | Result |
|------|--------|-----|--------|
| C++ stall_test | 200 | ~50 | No stall |
| Rust frame loop (post-fix) | 98 iterations, 44 frames | 4.4 | **No stall** |
| Rust mock tests | 63/63 | N/A | All pass |

The Rust driver now runs well beyond the previous 85-frame stall point.

## Known Limitations

### Performance Gap (bd-u1kx)

After fixing the stall, a separate performance issue was identified:
- **C++**: ~50 FPS sustained
- **Rust**: ~4.4 FPS with timeout errors in broadcast delivery

This is a different issue from the stall and requires separate investigation. The stall is definitively fixed - frames continue past 85 - but overall throughput is lower than the C++ reference.

## References

- **SDK Examples**: `/opt/pvcam/sdk/examples/LiveImage.cpp`
- **Handoff Document**: See git history for `docs/handoff/PVCAM_STALL_INVESTIGATION_2026_01_11.md` (removed during Diataxis restructure)
- **Performance Issue**: beads issue `bd-u1kx`

## Lessons Learned

1. **SDK examples are authoritative**: The PVCAM SDK examples demonstrate patterns that must be followed exactly. Deviating (e.g., "pure signaling" callbacks) can cause subtle failures.

2. **Buffer management is mode-dependent**: CIRC_OVERWRITE and CIRC_NO_OVERWRITE require fundamentally different handling strategies.

3. **Stalls vs performance are separate concerns**: Fixing a hard stall doesn't guarantee optimal performance.
