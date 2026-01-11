# ADR: PVCAM SDK Pattern Compliance

**Status:** In Progress
**Date:** 2025-01-10
**Author:** Architecture Review (bd-ng5p)
**Related Issues:** bd-ng5p, bd-ffi-sdk-match

---

## Context

During verification that the Rust PVCAM driver reproduces SDK patterns, a significant gap was discovered: the SDK mandates dynamic parameter discovery using `ATTR_AVAIL` before accessing camera-dependent parameters, but the Rust implementation was missing these checks in most places.

This ADR documents the SDK pattern requirements and tracks compliance.

---

## Decision

**Implement SDK-matching parameter availability checks throughout the driver.**

---

## SDK Pattern: IsParamAvailable

### SDK Reference Implementation

From `PVCAM SDK/examples/code_samples/src/CommonFiles/Common.cpp`:

```cpp
bool IsParamAvailable(int16 hcam, uns32 paramID, const char* paramName)
{
    if (!paramName)
        return false;

    rs_bool isAvailable;
    if (PV_OK != pl_get_param(hcam, paramID, ATTR_AVAIL, (void*)&isAvailable))
    {
        printf("Error reading ATTR_AVAIL of %s\n", paramName);
        return false;
    }
    if (isAvailable == FALSE)
    {
        printf("Parameter %s is not available\n", paramName);
        return false;
    }

    return true;
}
```

### SDK Usage Pattern

Every SDK example calls `IsParamAvailable()` before accessing camera-dependent parameters:

```cpp
// From FanSpeedAndTemperature.cpp
if (!IsParamAvailable(ctx->hcam, PARAM_TEMP, "PARAM_TEMP"))
    return false;

// From Centroids.cpp
if (!IsParamAvailable(ctx->hcam, PARAM_CENTROIDS_ENABLED, "PARAM_CENTROIDS_ENABLED"))
    return false;

// From Common.cpp (speed table enumeration)
if (!IsParamAvailable(ctx->hcam, PARAM_SPDTAB_INDEX, "PARAM_SPDTAB_INDEX"))
    return false;
```

---

## Rust Implementation

### Helper Functions Added (bd-ng5p)

Location: `crates/daq-driver-pvcam/src/components/features.rs`

```rust
/// Check if a parameter is available on the connected camera.
#[cfg(feature = "pvcam_hardware")]
pub fn is_param_available(hcam: i16, param_id: u32) -> bool {
    let mut avail: rs_bool = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_AVAIL as i16,
            &mut avail as *mut _ as *mut std::ffi::c_void,
        ) != 0
        {
            avail != 0
        } else {
            false
        }
    }
}

/// Check if a parameter is available, returning an error with context if not.
#[cfg(feature = "pvcam_hardware")]
pub fn require_param_available(hcam: i16, param_id: u32, param_name: &str) -> Result<()> {
    if Self::is_param_available(hcam, param_id) {
        Ok(())
    } else {
        Err(anyhow!(
            "Parameter {} (0x{:08X}) is not available on this camera",
            param_name,
            param_id
        ))
    }
}
```

---

## Compliance Status

### Parameters Requiring Availability Checks

Based on SDK examples, these parameters MUST have availability checks:

| Parameter | SDK Example | Rust Status | File |
|-----------|-------------|-------------|------|
| PARAM_TEMP | FanSpeedAndTemperature.cpp | ✅ Updated | features.rs |
| PARAM_TEMP_SETPOINT | FanSpeedAndTemperature.cpp | ✅ Updated | features.rs |
| PARAM_DD_VERSION | Common.cpp | ✅ Complete (bd-qijv) | features.rs |
| PARAM_CHIP_NAME | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_CAM_FW_VERSION | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_SER_SIZE | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_PAR_SIZE | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_SPDTAB_INDEX | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_PIX_TIME | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_GAIN_INDEX | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_BIT_DEPTH | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_READOUT_PORT | Common.cpp | ✅ Complete (bd-sk6z) | features.rs |
| PARAM_CLEAR_CYCLES | Common.cpp | ✅ Complete (bd-0yho) | features.rs |
| PARAM_PMODE | Common.cpp | ✅ Complete (bd-0yho) | features.rs |
| PARAM_EXPOSURE_MODE | Common.cpp | ✅ Complete (bd-smn3) | features.rs |
| PARAM_EXPOSE_OUT_MODE | Common.cpp | ✅ Complete (bd-smn3) | features.rs |
| PARAM_CENTROIDS_ENABLED | Centroids.cpp | ✅ Complete (bd-cq4y) | features.rs |
| PARAM_CENTROIDS_MODE | Centroids.cpp | ✅ Complete (bd-l35g) | features.rs |
| PARAM_CENTROIDS_THRESHOLD | Centroids.cpp | ✅ Complete (bd-cq4y) | features.rs |
| PARAM_METADATA_ENABLED | MultipleRegions.cpp | ✅ Complete (bd-l35g) | features.rs |
| PARAM_ROI_COUNT | MultipleRegions.cpp | ✅ Complete (bd-vcbd) | acquisition.rs |
| PARAM_PP_INDEX | PostProcessing.cpp | ✅ Complete (bd-l35g) | features.rs |
| PARAM_SMART_STREAM_MODE | Common.cpp | ✅ Complete (bd-l35g) | features.rs |
| PARAM_SMART_STREAM_MODE_ENABLED | Common.cpp | ✅ Complete (bd-l35g) | features.rs |
| PARAM_FRAME_BUFFER_SIZE | acquisition.rs | ✅ Existing | acquisition.rs |
| PARAM_CIRC_BUFFER | acquisition.rs | ✅ Existing | acquisition.rs |

### Summary

- **Total parameters requiring checks:** 26
- **Phase 0 (bd-ng5p thermal):** 2 ✅ (8%)
- **Phase 1 (bd-sk6z core):** 9 ✅ (35%)
- **Phase 2 (bd-l35g advanced):** 5 ✅ (19%)
- **Phase 3 (bd-smn3 remaining):** 2 ✅ (8%)
- **Phase 4 (bd-aowg missing functions):** 6 ✅ (23%)
- **Complete:** 26 of 26 (100%)

### Dynamic Discovery Functions (bd-q4wz)

The following dynamic discovery functions were added using the SDK pattern:

| Function | Parameter | Status |
|----------|-----------|--------|
| `list_available_cameras()` | N/A (camera enumeration) | ✅ SDK checked |
| `list_exposure_modes()` | `PARAM_EXPOSURE_MODE` | ✅ Availability checked |
| `list_clear_modes()` | `PARAM_CLEAR_MODE` | ✅ Availability checked |
| `list_expose_out_modes()` | `PARAM_EXPOSE_OUT_MODE` | ✅ Availability checked |

All list functions follow the SDK pattern by calling `is_param_available()` before accessing the parameter enumeration.

---

## FFI Layer Separation

### Current Architecture

```
┌─────────────────────────────────────────────────────────┐
│                  daq-driver-pvcam                        │
│  ┌─────────────────────────────────────────────────────┐ │
│  │                   lib.rs (Driver API)               │ │
│  │   PvcamDriver with Parameter<T> reactive system     │ │
│  └──────────────────────┬──────────────────────────────┘ │
│                         │                                │
│  ┌──────────────────────┼──────────────────────────────┐ │
│  │              components/                             │ │
│  │  ┌─────────────┐ ┌─────────────┐ ┌────────────────┐ │ │
│  │  │ connection  │ │ acquisition │ │   features     │ │ │
│  │  │   .rs       │ │    .rs      │ │     .rs        │ │ │
│  │  │ SDK init    │ │ Streaming   │ │ Parameters     │ │ │
│  │  │ lifecycle   │ │ callbacks   │ │ get/set        │ │ │
│  │  └─────────────┘ └─────────────┘ └────────────────┘ │ │
│  └──────────────────────┬──────────────────────────────┘ │
│                         │                                │
│  ┌──────────────────────┴──────────────────────────────┐ │
│  │               pvcam-sys (FFI Layer)                  │ │
│  │   - Bindgen-generated PVCAM SDK bindings            │ │
│  │   - Manual constant definitions (ATTR_*, etc.)      │ │
│  │   - Callback type definitions                       │ │
│  └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### Separation Verification

| Concern | Location | Status |
|---------|----------|--------|
| FFI bindings | pvcam-sys/src/lib.rs | ✅ Clean |
| SDK constants | pvcam-sys/src/lib.rs | ✅ Manual definitions for missing enums |
| Callback types | pvcam-sys/src/lib.rs | ✅ PvcamCallback type defined |
| Connection lifecycle | components/connection.rs | ✅ Isolated |
| Frame acquisition | components/acquisition.rs | ✅ Matches SDK pattern |
| Parameter access | components/features.rs | 🔄 Availability checks 62% complete |
| Driver API | lib.rs | ✅ Parameter<T> reactive system |

---

## Implementation Notes

### Phase 1 (P0) - Core Parameters Approach

The Phase 1 implementation (bd-sk6z) uses a systematic approach to add availability checks for core sensor parameters:

**Pattern for Parameter Functions:**

1. **Check availability first:** Call `is_param_available()` before `pl_get_param()`
2. **Return Result type:** Functions return `Result<T, DaqError>` with context
3. **Document rationale:** Add comments explaining why the parameter is queried

**Example pattern for sensor info parameters:**

```rust
/// Get the serial port array size (number of columns in sensor array).
/// Required before accessing column-related parameters.
pub fn get_param_ser_size(hcam: i16) -> Result<u16> {
    if !is_param_available(hcam, PARAM_SER_SIZE) {
        return Err(DaqError::ParameterNotAvailable {
            name: "PARAM_SER_SIZE".to_string(),
            id: PARAM_SER_SIZE,
        });
    }

    let mut value: u16 = 0;
    unsafe {
        if pl_get_param(
            hcam,
            PARAM_SER_SIZE,
            ATTR_CURRENT as i16,
            &mut value as *mut _ as *mut std::ffi::c_void,
        ) == 0
        {
            return Err(DaqError::PvcamError(/* error code */));
        }
    }
    Ok(value)
}
```

**Affected Parameter Groups:**

1. **Sensor Dimensions:** PARAM_SER_SIZE, PARAM_PAR_SIZE
2. **Device Information:** PARAM_CHIP_NAME, PARAM_DD_VERSION, PARAM_CAM_FW_VERSION
3. **Acquisition Settings:** PARAM_SPDTAB_INDEX, PARAM_GAIN_INDEX, PARAM_BIT_DEPTH, PARAM_PIX_TIME
4. **Hardware Routes:** PARAM_READOUT_PORT

**Related Code Changes:**

- Location: `crates/daq-driver-pvcam/src/components/features.rs`
- New error variant added to `DaqError`: `ParameterNotAvailable { name: String, id: u32 }`
- Helper functions: `is_param_available()`, `require_param_available()`

### Integration with Parameter<T> System

The availability checks integrate seamlessly with the reactive parameter system:

- Initialization queries verify parameter availability
- Hardware write callbacks use availability checks before sending commands
- Error handling surfaces unavailable parameters cleanly to users

---

## Verification Results

### Hardware Test Outcomes (Prime BSI)

| Parameter | Status | Notes |
|-----------|--------|-------|
| PARAM_SER_SIZE | Pending | Hardware test on maitai required |
| PARAM_PAR_SIZE | Pending | Hardware test on maitai required |
| PARAM_CHIP_NAME | Pending | Hardware test on maitai required |
| PARAM_DD_VERSION | Pending | Hardware test on maitai required |
| PARAM_CAM_FW_VERSION | Pending | Hardware test on maitai required |
| PARAM_SPDTAB_INDEX | Pending | Hardware test on maitai required |
| PARAM_GAIN_INDEX | Pending | Hardware test on maitai required |
| PARAM_BIT_DEPTH | Pending | Hardware test on maitai required |
| PARAM_PIX_TIME | Pending | Hardware test on maitai required |
| PARAM_READOUT_PORT | Pending | Hardware test on maitai required |

**Test Command:**

```bash
ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
  export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
  cd ~/rust-daq && cargo test --features pvcam_hardware \
    test_core_parameters -- --nocapture'
```

### Legacy Hardware Test

Run on maitai (Prime BSI) to verify availability checks work correctly:

```bash
ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
  export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
  export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
  cd ~/rust-daq && cargo test --features pvcam_hardware \
    --test pvcam_hardware_smoke -- --nocapture'
```

### Expected Behavior

- Parameters supported by Prime BSI: Access succeeds
- Parameters not supported: Clean error message instead of SDK error

---

## References

- PVCAM SDK Examples: `/opt/pvcam/sdk/examples/code_samples/`
- SDK Documentation: https://docs.teledynevisionsolutions.com/pvcam-sdk/
- Related ADRs:
  - [adr-pvcam-driver-architecture.md](./adr-pvcam-driver-architecture.md)
  - [adr-pvcam-continuous-acquisition.md](./adr-pvcam-continuous-acquisition.md)

---

## Revision History

| Date | Author | Description |
|------|--------|-------------|
| 2025-01-10 | bd-ng5p | Initial gap analysis and helper function implementation |
| 2025-01-10 | bd-sk6z | Phase 1: Core sensor parameters (9 params) complete |
| 2025-01-10 | bd-l35g | Phase 2: Advanced feature parameters (5 params) complete |
| 2026-01-10 | bd-smn3 | Phase 3: PARAM_EXPOSURE_MODE, PARAM_EXPOSE_OUT_MODE (2 params) complete |
| 2026-01-10 | bd-aowg | Phase 4: Missing parameter functions (6 params) complete - 100% coverage |
