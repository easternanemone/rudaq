# Code Analysis Report - 2025-11-25

**Analyzer:** Codex (via clink)
**Scope:** Recent gRPC and hardware development work
**Files Analyzed:**
- `src/grpc/hardware_service.rs` (923 lines, new)
- `src/grpc/scan_service.rs` (747 lines, new)
- `src/hardware/registry.rs` (752 lines, new)
- `src/hardware/pvcam.rs` (modified)
- `src/hardware/capabilities.rs` (modified)
- `src/hardware/mock.rs` (modified)
- `proto/daq.proto` (modified)

---

## Critical Issues

### HIGH: Progress streaming never sends (scan_service.rs:371-384)

**Problem:** `ScanService` builds `ScanProgress` messages but drops the `mpsc::Sender::send` future without awaiting it. No progress or data points ever reach `StreamScanProgress`; clients will stall/timeout and scans appear stuck.

**Location:** `src/grpc/scan_service.rs:371-384`

**Fix Required:**
```rust
// Instead of dropping the send future:
if let Err(e) = progress_tx.send(progress).await {
    // Handle error - scan should transition to error state if stream is gone
    log::error!("Failed to send progress: {}", e);
}
```

**Impact:** All scan progress streaming is broken.

---

### HIGH: Cameras cannot be registered (registry.rs)

**Problem:** `DriverType` and capabilities only cover `Movable`/`Readable` variants (MockStage, MockPowerMeter, Ell14, Esp300, etc.) and never expose `FrameProducer + Triggerable + ExposureControl`. No PVCAM/MockCamera variant exists.

**Location:** `src/hardware/registry.rs` - missing driver variants

**Impact:**
- Registry cannot produce any frame-capable device
- gRPC camera endpoints (`StartStream`, `StreamFrames`, `SetExposure`, `Trigger`) are unreachable in real deployments

**Fix Required:** Add driver variants:
```rust
pub enum DriverType {
    // ... existing variants ...
    MockCamera,
    Pvcam { camera_name: String },
}
```

And implement capability detection for `FrameProducer`, `Triggerable`, `ExposureControl`.

---

## Medium Issues

### MEDIUM: Mock streaming ignores binning (pvcam.rs:746-778)

**Problem:** In the non-hardware streaming path, frames are generated with dimensions `roi.width/height`, ignoring `set_binning`. Single-frame acquisition applies binning, but streaming does not.

**Location:** `src/hardware/pvcam.rs:746-778`

**Impact:** Clients see different dimensions depending on API path (single-frame vs streaming).

**Fix Required:** Apply bin factors to frame size and pixel generation in the mock streaming loop.

---

### MEDIUM: Trigger workflow incomplete for real hardware (pvcam.rs:496, :863)

**Problem:** Trigger wait and hardware arm setup are left as TODOs. With the hardware feature enabled, `arm()`/`wait_for_trigger()` become no-ops.

**Location:**
- `src/hardware/pvcam.rs:496`
- `src/hardware/pvcam.rs:863`

**Impact:** External trigger mode won't work and may hang acquisitions.

**Fix Required:**
- Implement PVCAM trigger setup (`pl_exp_setup_seq` with `TRIGGER_FIRST_MODE`)
- Implement real trigger-wait path
- Or explicitly gate/disable trigger mode until implemented

---

## Low Issues

### LOW: Stream stop lacks frame count (hardware_service.rs:711)

**Problem:** Returns `frames_captured: 0 // TODO` regardless of actual capture.

**Location:** `src/grpc/hardware_service.rs:711`

**Impact:** Clients can't verify stream output quantity.

**Fix Required:** Track frame_count from the producer and populate the response.

---

## Positive Findings

The analysis identified several well-implemented patterns:

1. **Lock scoping in HardwareService** - Cleanly releases the registry before awaiting device calls
2. **Capability-filtered discovery** - Clear and well-structured
3. **Scan validation** - Checks device presence/capabilities up front
4. **Good unit coverage** - Registry and capability traits have solid tests
5. **Stream handling** - `start_stream`/`stream_frames` avoids holding locks while streaming

---

## Test Status

- Overall: **PASS** (with one unrelated failure)
- Failed: `tests/scripting_standalone.rs::test_large_but_valid_loop` - assert `result.is_ok()` failed
  - This is unrelated to the reviewed modules

---

## Recommended Issue Tracking

| Priority | Issue | Location | Effort |
|----------|-------|----------|--------|
| HIGH | Fix progress streaming await | scan_service.rs:371-384 | Small |
| HIGH | Add camera driver variants to registry | registry.rs | Medium |
| MEDIUM | Apply binning to mock streaming | pvcam.rs:746-778 | Small |
| MEDIUM | Implement hardware trigger workflow | pvcam.rs:496, :863 | Large |
| LOW | Track frame count on stream stop | hardware_service.rs:711 | Small |

---

## Summary

The recent development work is architecturally sound with good patterns for lock handling, validation, and capability-based design. However, there are two high-priority bugs that need immediate attention:

1. **Scan progress streaming is completely broken** due to unawaited send futures
2. **Camera devices cannot be registered**, making all camera-related gRPC endpoints unreachable

These should be addressed before the gRPC API is considered production-ready.
