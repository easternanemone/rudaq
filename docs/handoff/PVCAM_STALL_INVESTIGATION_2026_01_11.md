# PVCAM 85-Frame Stall Investigation Handoff
**Date:** 2026-01-11
**Status:** ✅ FIXED (commit 3fce81ba)

## Executive Summary
The Prime BSI camera on `maitai` stalled after acquiring approximately 85 frames when using the `rust-daq` driver. **This issue has been fixed** by restoring the SDK pattern of calling `pl_exp_get_latest_frame` inside the EOF callback when in CIRC_OVERWRITE mode.

### Fix Details (commit 3fce81ba)
The fix in `acquisition.rs:407-426` calls `pl_exp_get_latest_frame` inside the callback to clear the SDK's internal buffer state:
```rust
// FIX (PVCAM_STALL_INVESTIGATION_2026_01_11):
if ctx.circ_overwrite.load(Ordering::Acquire) {
    if pl_exp_get_latest_frame(hcam, &mut frame_ptr) != 0 {
        ctx.store_frame_ptr(frame_ptr);
    }
}
```

### Verification Results (2026-01-11)
| Test | Frames | FPS | Result |
|------|--------|-----|--------|
| C++ stall_test | 200 | ~50 | ✅ No stall |
| Rust frame loop | 98 iterations, 44 captured | 4.4 | ✅ No stall (but low FPS) |

The Rust driver now runs 98+ iterations without stalling. The original 85-frame hard stop is resolved.

### Known Issue: Low FPS (bd-u1kx)
A separate performance issue was discovered: Rust achieves ~4.4 FPS vs C++'s ~50 FPS, and broadcast subscriber delivery has issues. This is tracked as bd-u1kx.

## Findings

### 1. Hardware & SDK Verification
- **Test:** `~/temp_repro/stall_test.cpp` (based on SDK `LiveImage` example).
- **Result:** Successfully acquired 500 frames at ~47 FPS without stalling.
- **Conclusion:** Hardware is functional; SDK environment is correct.

### 2. Rust Driver Status
- **Issue:** `rust-daq-daemon` fails to acquire more than ~93 frames (85 frames + detection buffer).
- **Connection:** Initial connection failures ("No PVCAM cameras detected") were resolved by ensuring `PVCAM_VERSION` and paths are exported in `~/.zshenv`.
- **Modes Tested:**
    - `CIRC_OVERWRITE` with `get_latest_frame` in main loop (Stalls)
    - `CIRC_OVERWRITE` with `get_latest_frame` in callback (Mirror Pattern) (Stalls)
    - `CIRC_NO_OVERWRITE` with `get_oldest_frame` + `unlock` (Robust FIFO) (Stalls)
- **Observations:** The stall occurs consistently at ~85 frames regardless of the buffer management strategy. This suggests the SDK's internal write pointer is wrapping or hitting a limit that isn't being cleared.

### 3. Debugging Performed
- **Strace:** Confirmed `rust-daq` loads the same libraries (`libpvcam.so.2`) and USB driver (`pvcam_usb.x86_64.umd`) as the working C++ test.
- **Bindgen:** Suppressed noisy warnings; no safety warnings found. Struct layouts verified to match C++ (`FRAME_INFO` offset 20/24).
- **Callbacks:** Verified callbacks are firing and `get_latest_frame` returns success (until the stall). `catch_unwind` added to prevent UB.

## Recommendations for Next Engineer
1.  **Memory Layout:** Investigate `PageAlignedBuffer` vs `new uns8[]`. Try using a standard `Vec<u8>` (even if unaligned) to see if the allocator is the issue.
2.  **Threading:** The Rust callback interacts with `Arc`/`Mutex` logic. Try a minimal Rust example that *only* does FFI (no Tokio, no Channels) to rule out runtime interference.
3.  **Buffer Size:** Hardcode the buffer size to exactly matches `LiveImage.cpp` (20 frames) to see if the stall happens earlier (at 17 frames?). This would confirm a buffer wrap issue.
4.  **FFI Structs:** Double-check the `bindgen` generated struct layout for `FRAME_INFO` and `rgn_type`. A misalignment here could cause the SDK to read garbage data.

## Environment Notes
- **Machine:** `maitai` (100.117.5.12)
- **PVCAM Version:** 7.1.1.118
- **SDK Path:** `/opt/pvcam/sdk`
- **Config:** `config/maitai_hardware.toml`
