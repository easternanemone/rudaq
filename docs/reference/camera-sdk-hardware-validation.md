# Camera SDK Hardware Validation Report

Consolidated findings from instrument manual validation (NotebookLM) and multi-model
code review (Gemini model, OpenAI model, OpenAI code model) of the `camera-drivers` branch.

**Date:** 2026-02-10
**Branch:** `camera-drivers`
**Scope:** `crates/driver-andor-sdk3/`, `crates/andor-sdk3-sys/`, `crates/driver-pvcam/`
**Tests:** 213/213 passing (mock/SimCam)

---

## Table of Contents

1. [Instrument Manual Validation (NotebookLM)](#1-instrument-manual-validation-notebooklm)
   - [Andor SDK3 (iStar ICCD)](#11-andor-sdk3-istar-iccd)
   - [PVCAM 3.x (Prime 95B)](#12-pvcam-3x-prime-95b)
   - [Shamrock Spectrograph](#13-shamrock-spectrograph)
2. [Code Review Findings](#2-code-review-findings)
   - [HIGH Severity](#21-high-severity)
   - [MEDIUM Severity](#22-medium-severity)
   - [LOW Severity](#23-low-severity)
3. [Confirmed Correct Implementations](#3-confirmed-correct-implementations)
4. [Hardware Test Plan](#4-hardware-test-plan)

---

## 1. Instrument Manual Validation (NotebookLM)

Sources: Andor SDK3 Manual, PVCAM 3.x Programmer's Manual, Shamrock SDK Manual.

### 1.1 Andor SDK3 (iStar ICCD)

#### FFI Signatures — All Confirmed Correct

All 22 `atcore.h` function signatures and 30 Shamrock function signatures in
`andor-sdk3-sys` match the official SDK documentation exactly.

Key validations:
- `AT_QueueBuffer(AT_H, AT_U8*, int)` — correct parameter types
- `AT_WaitBuffer(AT_H, AT_U8**, int*, unsigned int)` — timeout is `unsigned int` (see review finding below)
- `AT_RegisterFeatureCallback(AT_H, AT_WC*, FeatureCallback, void*)` — correct signature
- `AT_ConvertBuffer(AT_U8*, AT_U8*, AT_64, AT_64, AT_64, AT_WC*, AT_WC*)` — available but not yet used

#### 8-Byte Buffer Alignment — Confirmed Required

The SDK manual explicitly states: *"Buffers passed to AT_QueueBuffer must be aligned
to an 8-byte boundary."* Our `AlignedBuffer` implementation uses `Layout::from_size_align(size, 8)`
which satisfies this requirement. Error code `AT_ERR_INVALIDALIGNMENT` (16) is returned
if this is violated.

#### Acquisition Sequence — Confirmed Correct

Our implementation follows the documented sequence:
1. Query `ImageSizeBytes`
2. Allocate 8-byte aligned buffers
3. `AT_QueueBuffer()` for each buffer (we use `DEFAULT_BUFFER_COUNT = 10`)
4. `AT_Command("AcquisitionStart")`
5. Loop: `AT_WaitBuffer()` → process → re-queue
6. `AT_Command("AcquisitionStop")` → `AT_Flush()`

#### DDG Timing — Confirmed: SDK Expects Seconds

The manual confirms: *"DDGOutputDelay and DDGOutputWidth are float features specified
in seconds."* Our driver correctly converts from picoseconds to seconds:
```rust
let delay_seconds = delay_ps as f64 * 1e-12;
AT_SetFloat(handle, L"DDGOutputDelay", delay_seconds);
```
This was a bug fix during the sprint — the original code passed raw picoseconds.

#### MCP Gain Range — Confirmed: 0-4095

The manual states MCP (Micro-Channel Plate) gain is an integer feature with range
0-4095 for iStar cameras. Our implementation correctly uses `AT_SetInt`/`AT_GetInt`
for `MCPGain` and validates the range.

#### Pixel Encodings — Confirmed Complete

| Encoding | Bits | Bytes/Pixel | Status |
|----------|------|-------------|--------|
| Mono12 | 12 | 2 (in 16-bit container) | Supported |
| Mono12Packed | 12 | 1.5 (packed) | Supported (manual unpack, not AT_ConvertBuffer) |
| Mono16 | 16 | 2 | Supported (default) |
| Mono32 | 32 | 4 | Supported (diagnostic) |

**Note:** Mono12Packed uses manual byte unpacking rather than `AT_ConvertBuffer`. This
works correctly but the SDK function would be more efficient (see LOW finding below).

#### Feature Discovery — Confirmed Pattern

`AT_IsImplemented()` returns `AT_TRUE`/`AT_FALSE` for each feature per camera model.
Our `FeatureSupport` struct probes features at initialization, matching the documented
approach of checking availability before access.

#### Trigger Modes — Confirmed Complete

| Mode | SDK String | Status |
|------|-----------|--------|
| Internal | `"Internal"` | Implemented |
| External | `"External"` | Implemented |
| External Start | `"External Start"` | Implemented |
| External Exposure | `"External Exposure"` | Implemented |
| Software | `"Software"` | Implemented |

All modes confirmed present in the SDK3 TriggerMode enumeration.

### 1.2 PVCAM 3.x (Prime 95B)

#### pl_cam_get_diags — PVCAM 2.x Legacy (Class 0)

**Finding:** `pl_cam_get_diags()` is a PVCAM 2.x Class 0 function. In PVCAM 3.x,
diagnostic information is obtained through `PARAM_*` parameters instead. The function
may exist for backward compatibility but is not the recommended approach.

**Impact:** Our driver calls it in `connection.rs` as a health check after `pl_cam_open()`.
It likely returns `PV_OK` on modern SDKs but should be replaced with `PARAM_DD_INFO`
queries for forward compatibility.

**Recommendation:** Replace with `pl_get_param(hcam, PARAM_DD_INFO, ATTR_CURRENT, ...)`
or simply remove the check (the `pl_cam_open` return value is sufficient).

#### pl_io_script_control — PVCAM 2.x Legacy (Class 3)

**Finding:** `pl_io_script_control()` is a PVCAM 2.x Class 3 function for TTL I/O
scripting. In PVCAM 3.x, I/O control uses `PARAM_IO_TYPE`, `PARAM_IO_DIRECTION`,
`PARAM_IO_STATE`, and `PARAM_IO_BITDEPTH` parameters instead.

**Impact:** Our `io_script_cmd` parameter in `features/mod.rs` wraps this function.
It may not function correctly on PVCAM 3.x and should be migrated to the parameter
API.

**Recommendation:** Replace with:
```rust
// PVCAM 3.x I/O control
pl_set_param(hcam, PARAM_IO_STATE, &output_value);
pl_get_param(hcam, PARAM_IO_STATE, ATTR_CURRENT, &current_value);
```

#### pl_exp_unravel — Correctly Removed

**Confirmed:** `pl_exp_unravel` was removed/superseded in PVCAM 3.x. Our driver
correctly uses `pl_md_frame_decode()` → `md_frame.roiArray` for multi-ROI data
extraction, which is the PVCAM 3.x approach.

#### pl_md_frame_decode — Confirmed Correct Usage

The manual confirms the metadata decode sequence:
1. `pl_md_create_frame_struct_cont(&md_frame, roi_count)` — allocate structure
2. `pl_md_frame_decode(md_frame, buffer, buffer_size)` — decode embedded metadata
3. Access `md_frame->roiArray[i]` for per-ROI data
4. `pl_md_release_frame_struct(md_frame)` — cleanup

Our `ffi_safe.rs` wraps this sequence correctly.

#### PARAM_PMODE Values — Confirmed

| Value | Constant | Description |
|-------|----------|-------------|
| 0 | `PMODE_NORMAL` | Normal readout |
| 1 | `PMODE_FT` | Frame Transfer |
| 2 | `PMODE_MPP` | MPP mode |
| 3 | `PMODE_FT_MPP` | Frame Transfer + MPP |
| 4 | `PMODE_ALT_NORMAL` | Alt Normal |
| 5 | `PMODE_ALT_FT` | Alt Frame Transfer |
| 6 | `PMODE_ALT_MPP` | Alt MPP |
| 7 | `PMODE_ALT_FT_MPP` | Alt Frame Transfer + MPP |

Our `speed_table.rs` frame transfer mode support is compatible with these values.

#### Bit Depth — Hardware-Dependent

**Finding:** `PARAM_BIT_DEPTH` should be queried at runtime rather than hardcoded.
The Prime 95B supports 16-bit, but other cameras in the Teledyne lineup support
10, 13, and 14-bit depths. Hardcoding `bit_depth = 16` in frame creation will be
incorrect for multi-camera setups.

### 1.3 Shamrock Spectrograph

#### Slit Ports — Confirmed: 4 Ports

The Shamrock SDK defines exactly 4 slit positions:
1. `SHAMROCK_INPUT_SLIT_SIDE` — Input Side
2. `SHAMROCK_INPUT_SLIT_DIRECT` — Input Direct
3. `SHAMROCK_OUTPUT_SLIT_SIDE` — Output Side
4. `SHAMROCK_OUTPUT_SLIT_DIRECT` — Output Direct

Our `SlitPort` enum matches this exactly.

#### Grating Control — Confirmed

- `ShamrockGetNumberGratings()` returns count (typically 1-3)
- `ShamrockSetGrating()` / `ShamrockGetGrating()` — 1-based index
- `ShamrockGetGratingInfo()` returns lines/mm, blaze, home, offset
- `ShamrockWavelengthIsPresent()` confirms motorized wavelength control

#### Wavelength Limits — Confirmed

`ShamrockGetWavelengthLimits(device, grating, &min, &max)` provides per-grating
wavelength boundaries. Our `WavelengthLimits` struct captures these correctly.

#### Filter Wheel — Partially Documented

`ShamrockFilterIsPresent()`, `ShamrockGetFilter()`, `ShamrockSetFilter()`,
`ShamrockGetFilterInfo()` are present in the SDK. Our implementation covers
get/set operations. The `ShamrockGetFilterInfo()` return values (type, description)
were not fully documented in the available manual sources.

#### Focus and Detector Offset — Partially Documented

`ShamrockFocusMirrorIsPresent()`, `ShamrockGetFocusMirror()`, `ShamrockSetFocusMirror()`
are confirmed present. `ShamrockDetectorOffsetIsPresent()`, `ShamrockGetDetectorOffset()`,
`ShamrockSetDetectorOffset()` are similarly confirmed. Exact range limits were not
specified in available documentation — must be queried at runtime.

---

## 2. Code Review Findings

### 2.1 HIGH Severity

#### H1: Timestamp Source Inconsistency Between Drivers
**Reviewer:** OpenAI code model (Architecture)
**File:** `driver-andor-sdk3/src/camera.rs`, `driver-pvcam/src/components/acquisition/`
**Description:** Andor driver uses `SystemTime::now()` wall-clock timestamps while
PVCAM uses `FRAME_INFO.TimeStamp` hardware timestamps. When both cameras are used
in the same experiment, timestamps are not comparable.
**Recommendation:** Both drivers should use their respective SDK hardware timestamps
and convert to a common epoch. Andor SDK3 provides `TimestampClock` and
`TimestampClockFrequency` features for hardware-based timing.
**Beads:** bd-z54k

#### H2: PVCAM primary_output Not Wired in Hardware Path
**Reviewer:** OpenAI code model (Architecture)
**File:** `driver-pvcam/src/lib.rs`
**Description:** `register_primary_output()` accepts a `Pool<FrameData>` but the
hardware acquisition path (both sequence mode and callback mode) does not use it
for frame delivery. Frames are broadcast via internal channels but never through
the pool-based primary output.
**Recommendation:** Wire `primary_tx` to acquire frames from the pool, copy data
into `LoanedFrame`, and send through the primary output channel.
**Beads:** bd-r8ux

#### H3: CIRC_OVERWRITE Buffer Unlock Timing
**Reviewer:** OpenAI code model (Architecture)
**File:** `driver-pvcam/src/components/acquisition/mod.rs`
**Description:** In CIRC_OVERWRITE mode, `pl_exp_unlock_oldest_frame()` is called
but the SDK may overwrite the buffer before the application finishes copying. The
unlock timing relative to the copy operation must be carefully ordered.
**Recommendation:** Ensure full frame data copy completes BEFORE calling unlock.
Add explicit sequencing guarantees in the acquisition loop.
**Beads:** bd-g3ap

### 2.2 MEDIUM Severity

#### M1: AT_WaitBuffer Timeout Sign Mismatch
**Reviewer:** g3-pro (Andor SDK3)
**File:** `driver-andor-sdk3/src/camera.rs`
**Description:** `AT_WaitBuffer` expects `unsigned int` for timeout but Rust's
`c_int` is signed. Values > `i32::MAX` would be truncated. In practice, timeouts
are small (1000-5000ms) so this is unlikely to cause issues, but it's technically
incorrect.
**Recommendation:** Use `c_uint` for the timeout parameter in the FFI call.

#### M2: Feature Callbacks Not Unregistered in Drop
**Reviewer:** g3-pro (Andor SDK3)
**File:** `driver-andor-sdk3/src/camera.rs`
**Description:** `AT_RegisterFeatureCallback` is called for ExposureTime, Temperature,
and FrameRate, but `AT_UnregisterFeatureCallback` is never called in the `Drop`
implementation. This could cause callbacks to fire after the camera struct is dropped.
**Recommendation:** Track registered callbacks and call `AT_UnregisterFeatureCallback`
in `Drop` before `AT_Close`.

#### M3: Hardcoded bit_depth=16 in PVCAM Frame Creation
**Reviewer:** OpenAI code model (Architecture)
**File:** `driver-pvcam/src/components/acquisition/mod.rs`
**Description:** Frame metadata sets `bit_depth = 16` regardless of actual camera
configuration. Should query `PARAM_BIT_DEPTH` at setup time.
**Recommendation:** Query `PARAM_BIT_DEPTH` during acquisition setup and store for
frame creation.

#### M4: Triggerable::trigger() Swallows Acquisition Failures
**Reviewer:** OpenAI code model (Architecture)
**File:** `driver-andor-sdk3/src/camera.rs`
**Description:** The `trigger()` implementation calls `AT_Command("SoftwareTrigger")`
but doesn't propagate the SDK error code to the caller in all code paths.
**Recommendation:** Ensure all error paths in trigger() surface SDK errors.

#### M5: Mock Camera Never Notifies Observers
**Reviewer:** OpenAI code model (Architecture)
**File:** `driver-andor-sdk3/src/mock.rs`
**Description:** The mock camera generates frames but doesn't call
`tap_registry.notify()` for observer notification. This means observer-based tests
don't exercise the real notification path.
**Recommendation:** Add `tap_registry.notify(&frame_view)` in the mock frame loop.

#### M6: No USB/PCIe Reconnection Handling
**Reviewer:** OpenAI code model (Architecture)
**File:** Both drivers
**Description:** Neither driver handles USB disconnect/reconnect scenarios. A cable
reseat requires full daemon restart.
**Recommendation:** Implement connection health monitoring and automatic reconnection
with exponential backoff.

#### M7: No Frame Rate Limiting / Backpressure
**Reviewer:** OpenAI code model (Architecture)
**File:** `driver-andor-sdk3/src/camera.rs`
**Description:** If the consumer can't keep up, `try_send` silently drops frames
without feedback to the acquisition loop.
**Recommendation:** Implement backpressure signaling so the acquisition loop can
slow down or log warnings when consumers can't keep pace.

#### M8: No Spectrograph-Camera Synchronization
**Reviewer:** OpenAI code model (Architecture)
**File:** `driver-andor-sdk3/src/spectrograph.rs`
**Description:** Grating changes and wavelength scans are not synchronized with
camera acquisition. Automated spectral scanning requires coordinated stop-move-start.
**Recommendation:** Add a `ScanController` that coordinates spectrograph movement
with camera exposure windows.

#### M9: pl_cam_get_diags is PVCAM 2.x
**Reviewer:** OpenAI model (PVCAM), confirmed by NotebookLM
**File:** `driver-pvcam/src/components/connection.rs`
**Description:** See Section 1.2 above. This is a legacy function that should be
replaced with PVCAM 3.x parameter queries.

#### M10: pl_io_script_control is PVCAM 2.x
**Reviewer:** OpenAI model (PVCAM), confirmed by NotebookLM
**File:** `driver-pvcam/src/components/features/mod.rs`
**Description:** See Section 1.2 above. The I/O scripting API should be migrated
to PARAM_IO_* parameters.

### 2.3 LOW Severity

#### L1: No AT_ConvertBuffer for Mono12Packed
**Reviewer:** g3-pro (Andor SDK3)
**Description:** Manual byte-level unpacking instead of using SDK's `AT_ConvertBuffer`.
The SDK function would handle edge cases and future encoding changes.

#### L2: Wall-Clock Timestamps Instead of Hardware TimestampClock
**Reviewer:** g3-pro (Andor SDK3)
**Description:** Related to H1 above. Andor SDK3 provides `TimestampClock` and
`TimestampClockFrequency` for precise hardware-based timing.

#### L3: AOI Dimensions Not Validated Against Sensor Limits
**Reviewer:** g3-pro (Andor SDK3)
**Description:** ROI/binning callbacks don't validate against `AOIWidth`/`AOIHeight`
min/max before sending to SDK. The SDK will reject invalid values, but better UX
to validate client-side.

#### L4: PixelEncoding Hardcoded to Mono16
**Reviewer:** g3-pro (Andor SDK3)
**Description:** Default encoding is Mono16. Should auto-negotiate based on camera
capabilities and pre-amp gain setting.

#### L5: Multi-ROI Logged But Not Delivered as Separate Frames
**Reviewer:** OpenAI model (PVCAM)
**Description:** When multiple ROIs are configured, the driver logs multi-ROI data
but delivers only the first ROI as the output frame. Full multi-ROI support would
require delivering each ROI as a separate frame or as a composite.

#### L6: Sequence Mode Lacks Double-Buffering
**Reviewer:** OpenAI model (PVCAM)
**Description:** In sequence mode streaming, there's a gap between batch completion
and the next batch start. Double-buffering (setup next batch while processing current)
would reduce latency.

#### L7: pl_error_message Buffer Truncation
**Reviewer:** OpenAI model (PVCAM)
**Description:** Error message buffer size may truncate long error descriptions.
Should use a larger buffer or query message length first.

#### L8: Buffer Configuration Not Queried from SDK
**Reviewer:** OpenAI code model (Architecture)
**Description:** Frame buffer count is hardcoded rather than using
`PARAM_FRAME_BUFFER_SIZE` min/max to determine optimal sizing.

#### L9: No Health Monitoring Loop
**Reviewer:** OpenAI code model (Architecture)
**Description:** Neither driver implements periodic health checks (temperature drift,
connection status, error rates).

#### L10: Inconsistent Error Format Between Drivers
**Reviewer:** OpenAI code model (Architecture)
**Description:** Andor driver uses `DriverError::Sdk(code, msg)` while PVCAM uses
`PvcamError::SdkError(code)`. A unified error type would simplify higher-level
error handling.

---

## 3. Confirmed Correct Implementations

These implementation decisions were validated against instrument manuals:

| Feature | Driver | Validation |
|---------|--------|------------|
| 8-byte buffer alignment | Andor | Manual: "must be aligned to 8-byte boundary" |
| DDG timing in seconds | Andor | Manual: "DDGOutputDelay in seconds" |
| MCP gain 0-4095 | Andor | Manual: integer feature, range 0-4095 |
| 10-buffer queue depth | Andor | Manual: "10+ recommended" |
| AcqStart→WaitBuffer→AcqStop→Flush | Andor | Manual: exact documented sequence |
| Feature callbacks + rules | Andor | Manual: "never modify features inside callback" |
| AT_IsImplemented guard | Andor | Manual: "always check availability" |
| pl_md_frame_decode for metadata | PVCAM | Manual: PVCAM 3.x metadata API |
| CIRC_NO_OVERWRITE + get_oldest | PVCAM | Manual: correct for Prime BSI |
| Sequence mode streaming | PVCAM | Validated workaround for Error 185 |
| FrameNr-based loss detection | PVCAM | Manual: "1-based frame counter" |
| 4 slit ports (SlitPort enum) | Shamrock | Manual: exact 4 positions |
| Per-grating wavelength limits | Shamrock | Manual: ShamrockGetWavelengthLimits |
| 1-based grating index | Shamrock | Manual: "1 to NumberGratings" |

---

## 4. Hardware Test Plan

Before merging `camera-drivers` to main, validate on maitai hardware:

### Andor iStar ICCD
- [ ] `ANDOR_SMOKE_TEST=1` smoke tests pass with real hardware
- [ ] DDG timing produces correct output delay (verify with oscilloscope)
- [ ] MCP gain control responds correctly across 0-4095 range
- [ ] Continuous streaming at expected frame rates
- [ ] Temperature monitoring reads valid values
- [ ] Feature discovery matches expected iStar capabilities

### Teledyne Prime 95B
- [ ] Sequence mode streaming produces frames at ~23 FPS (10ms exposure)
- [ ] Frame metadata timestamps are monotonically increasing
- [ ] Multi-ROI configuration and metadata decode work correctly
- [ ] Speed table enumeration matches known camera speeds
- [ ] Frame transfer mode (PARAM_PMODE) configuration works

### Shamrock Spectrograph
- [ ] Grating selection and wavelength calibration
- [ ] Slit width control on all 4 ports
- [ ] Filter wheel positioning
- [ ] Focus mirror adjustment
- [ ] Coordinated camera-spectrograph operation (manual for now)

### Cross-Driver
- [ ] Verify FrameData compatibility between Andor and PVCAM frames
- [ ] Both drivers register with daemon device registry
- [ ] Concurrent operation of both cameras + spectrograph

---

## 5. Deep Review Findings (Round 2)

Additional findings from continued model conversations with targeted code examination.

### 5.1 CRITICAL ESCALATION: PVCAM Unlock-Before-Copy (H3 → P0)
**Reviewer:** OpenAI model (PVCAM Deep Review)
**File:** `driver-pvcam/src/components/acquisition/mod.rs` lines ~2283-2520
**Description:** The hardware frame loop calls `pl_exp_unlock_oldest_frame()` BEFORE
completing the frame data copy and `pl_md_frame_decode()`. Code contains the comment
`"frame_ptr is NO LONGER VALID after unlock"` but then proceeds to use `frame_ptr`
for metadata decode. In CIRC_OVERWRITE mode, the SDK can overwrite the DMA buffer
during copy, causing data corruption. Even in CIRC_NO_OVERWRITE, fast frame rates
with small buffer counts could trigger the same issue.
**Fix:** Reorder to: `get_oldest_frame` → copy pixels → `pl_md_frame_decode` →
THEN `release_oldest_frame`.
**Beads:** bd-g3ap (escalated from P1 to P0)

### 5.2 Gemini Model: Andor Deep Validation

All 5 targeted items validated:

| Item | Status | Detail |
|------|--------|--------|
| AT_WaitBuffer timeout type | **CONFIRMED BUG** | `build.rs:418` uses `c_int`, SDK expects `c_uint`. Safe at current 10s timeout but technically incorrect. Beads: bd-y5a1 |
| Feature callback lifecycle | **CONFIRMED BUG** | `camera.rs:1160-1187` registers 3 callbacks, Drop at 1734-1772 never unregisters. Should insert `AT_UnregisterFeatureCallback` between `AT_Flush` and `AT_Close`. Beads: bd-cytq |
| Buffer alignment | **CONFIRMED CORRECT** | `Layout::from_size_align(size, 8)` with `alloc_zeroed` guarantees 8-byte alignment. Tests verify. |
| Acquisition loop error recovery | **CONFIRMED CORRECT** | `is_timeout()` → continue, other errors → `set_error()` + break. `spawn_blocking` handles task cancellation. |
| Drop ordering | **MISSING STEP** | Current: AcqStop → Flush → Close. Should be: AcqStop → Flush → **UnregisterCallbacks** → Close → FinaliseLibrary. |

### 5.3 OpenAI Model: PVCAM Deep Validation

| Item | Status | Detail |
|------|--------|--------|
| primary_tx wiring | **CONFIRMED NO-OP** | `register_primary_output()` sets `primary_tx` but no `primary_tx.send()` in hardware frame loop. Explicit TODO at line ~2821: `"TODO(bd-5oss): Wire primary_tx for LoanedFrame delivery"`. Beads: bd-r8ux |
| Buffer unlock ordering | **CRITICAL** | See escalation above (5.1). Beads: bd-g3ap |
| md_frame lifecycle | **CORRECT but fragile** | No RAII wrapper — panic/early-return would leak. Add `MdFrameGuard`. Beads: bd-u602 |
| Sequence mode gaps | **CONFIRMED TRADE-OFF** | Per-batch `pl_exp_setup_seq` + Vec allocation creates dead time. Reusing buffer and hoisting setup would reduce gap. |
| bit_depth hardcoded | **CONFIRMED** | `Frame::from_bytes(width, height, 16, ...)` hardcodes 16. `get_bit_depth()` exists but isn't used in frame creation. Beads: bd-w5az |

### 5.4 OpenAI Code Model: Architecture Deep Validation

| Item | Status | Detail |
|------|--------|--------|
| Timestamp unification | **CONFIRMED** | Andor: `SystemTime::now().as_nanos()` at camera.rs:1484-1492. PVCAM: hardware `FRAME_INFO.TimeStamp`. Not comparable. Beads: bd-z54k |
| FrameProducer compliance | **PVCAM NON-COMPLIANT** | Andor correctly sends LoanedFrame through primary_tx. PVCAM never does. |
| Backpressure | **CONFIRMED SILENT** | Andor: `try_send` drops silently, no counter. PVCAM: broadcast lags slow receivers. Beads: bd-79da |
| Error type unification | **DIVERGENT** | Andor: `Option<String>`. PVCAM: `AcquisitionError` enum. Both map to DaqError at boundaries. |
| Mock fidelity | **CONFIRMED INCOMPLETE** | No observer notification, no error injection, no feature callbacks in mock. Beads: bd-xqj1 |

### 5.5 Consolidated Beads Issue Tracker

| ID | Priority | Title |
|----|----------|-------|
| bd-g3ap | **P0** | PVCAM unlock-before-copy data corruption risk |
| bd-z54k | P1 | Timestamp inconsistency between drivers |
| bd-r8ux | P1 | PVCAM primary_output not wired in hardware path |
| bd-y5a1 | P2 | AT_WaitBuffer timeout sign mismatch |
| bd-cytq | P2 | Unregister Andor feature callbacks in Drop |
| bd-w5az | P2 | Query PARAM_BIT_DEPTH instead of hardcoding 16 |
| bd-c094 | P2 | Surface SDK errors from trigger() |
| bd-xqj1 | P2 | Add observer notification to Andor mock |
| bd-lyfw | P2 | Replace pl_cam_get_diags with PVCAM 3.x API |
| bd-lkci | P2 | Migrate pl_io_script_control to PARAM_IO_* |
| bd-u602 | P2 | Add RAII guard for md_frame lifecycle |
| bd-79da | P2 | Track backpressure frame drops in Andor |
| bd-9id0 | P3 | USB/PCIe reconnection handling |
| bd-jwyv | P3 | Backpressure signaling in acquisition |
| bd-zp33 | P3 | Spectrograph-camera synchronization |
