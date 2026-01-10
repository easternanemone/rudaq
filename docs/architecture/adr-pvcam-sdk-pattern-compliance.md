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
| PARAM_DD_VERSION | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_CHIP_NAME | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_CAM_FW_VERSION | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_SER_SIZE | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_PAR_SIZE | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_SPDTAB_INDEX | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_PIX_TIME | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_GAIN_INDEX | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_BIT_DEPTH | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_READOUT_PORT | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_CLEAR_CYCLES | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_PMODE | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_EXPOSURE_MODE | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_EXPOSE_OUT_MODE | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_CENTROIDS_ENABLED | Centroids.cpp | ⚠️ Pending | features.rs |
| PARAM_CENTROIDS_MODE | Centroids.cpp | ⚠️ Pending | features.rs |
| PARAM_CENTROIDS_THRESHOLD | Centroids.cpp | ⚠️ Pending | features.rs |
| PARAM_METADATA_ENABLED | MultipleRegions.cpp | ⚠️ Pending | features.rs |
| PARAM_ROI_COUNT | MultipleRegions.cpp | ⚠️ Pending | acquisition.rs |
| PARAM_PP_INDEX | PostProcessing.cpp | ⚠️ Pending | features.rs |
| PARAM_SMART_STREAM_MODE | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_SMART_STREAM_MODE_ENABLED | Common.cpp | ⚠️ Pending | features.rs |
| PARAM_FRAME_BUFFER_SIZE | acquisition.rs | ✅ Existing | acquisition.rs |
| PARAM_CIRC_BUFFER | acquisition.rs | ✅ Existing | acquisition.rs |

### Summary

- **Total parameters requiring checks:** 26
- **Already compliant:** 4 (15%)
- **Pending updates:** 22 (85%)

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
| Parameter access | components/features.rs | ⚠️ Availability checks needed |
| Driver API | lib.rs | ✅ Parameter<T> reactive system |

---

## Implementation Plan

### Phase 1: Core Parameters (P0)

Update most-used parameters with availability checks:

1. Temperature: `PARAM_TEMP`, `PARAM_TEMP_SETPOINT` ✅
2. Sensor info: `PARAM_SER_SIZE`, `PARAM_PAR_SIZE`, `PARAM_CHIP_NAME`
3. Speed/gain: `PARAM_SPDTAB_INDEX`, `PARAM_GAIN_INDEX`, `PARAM_BIT_DEPTH`

### Phase 2: Advanced Features (P1)

Update feature-specific parameters:

1. Centroids: `PARAM_CENTROIDS_*`
2. Smart streaming: `PARAM_SMART_STREAM_*`
3. Post-processing: `PARAM_PP_*`
4. Metadata: `PARAM_METADATA_ENABLED`

### Phase 3: Remaining Parameters (P2)

Complete remaining parameter functions with availability checks.

---

## Verification

### Hardware Test

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
