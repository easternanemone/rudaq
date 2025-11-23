# PVCAM Hardware Test Results

**Date**: 2025-11-23
**Remote Machine**: maitai@100.117.5.12 (EndeavourOS)
**Camera**: Photometrics Prime BSI
**PVCAM SDK**: v2.6 at /opt/pvcam/sdk

## Executive Summary

✅ **Compilation Success**: All PVCAM FFI bindings compile and link successfully
✅ **Hardware Detection Success**: Prime BSI camera detected and operational (required PVCAM environment variables)
⚠️  **Partial Test Success**: 9/28 tests passing, 19 failing due to implementation issues (binning, acquisition)

## Test Execution Results

### Build Status: ✅ SUCCESS

```bash
# Build command
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_LIB_DIR=/opt/pvcam/library/x86_64
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
cargo test --test hardware_pvcam_validation \
  --features 'instrument_photometrics,pvcam_hardware,hardware_tests,pvcam-sys/pvcam-sdk' \
  -- --test-threads=1
```

**Compilation**: Clean build with warnings only (52 naming convention warnings in FFI bindings)
**Linking**: Successfully linked against libpvcam.so.2.6
**Test Compilation**: All 28 tests compiled successfully

### Test Results: ⚠️ PARTIAL SUCCESS (9 passed, 19 failed)

**WITHOUT Environment Variables**: All 28 tests failed with "No PVCAM cameras detected"
**WITH Environment Variables**: Camera detected, 9 tests passing

**Required Environment Variables**:
```bash
export PVCAM_VERSION=3.10.0.3
export PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode
export PVCAM_SDK_PATH=/opt/pvcam/sdk
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_LIB_DIR=/opt/pvcam/library/x86_64
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
```

#### Passing Tests (9/28) ✅

1. `test_prime_bsi_dimensions` - Prime BSI 2048x2048 sensor detected correctly
2. `test_hardware_initialization` - Hardware camera initialization successful
3. `test_create_prime_bsi` - Camera instance creation works
4. `test_exposure_control` - Exposure time control functional
5. `test_arm_disarm_trigger` - Trigger arming/disarming operational
6. `test_invalid_binning` - Binning validation logic correct
7. `test_roi_full_sensor` - Full sensor ROI configuration works
8. `test_roi_quarter_sensor` - Partial ROI configuration works
9. `test_hardware_triggered_acquisition` - Triggered acquisition functional

#### Failing Tests (19/28) ❌

**Wrong Camera Model (2 failures)**:
- `test_create_prime_95b` - Expected Prime 95B (1200x1200), but Prime BSI (2048x2048) is connected
- `test_prime_95b_dimensions` - Same issue

**Binning Implementation Issues (6 failures)**:
- `test_binning_1x1`, `test_binning_2x2`, `test_binning_4x4`
- `test_binning_validation`, `test_frame_size_with_binning`, `test_hardware_binning`
- **Error**: "Failed to set horizontal binning"
- **Root Cause**: `pl_set_param(PARAM_BINNING_SER/PAR)` calls failing

**Frame Acquisition Issues (8 failures)**:
- Timeout: `test_acquire_single_frame`, `test_multiple_frames`, `test_rapid_acquisition`, `test_hardware_exposure_accuracy`
- Setup: `test_frame_data_pattern`, `test_hardware_frame_acquisition`, `test_hardware_pixel_uniformity`, `test_hardware_dark_noise`, `test_hardware_roi`
- **Errors**: "Failed to setup acquisition sequence" or "Acquisition timeout"

**ROI Validation Issues (3 failures)**:
- `test_invalid_roi_exceeds_sensor`, `test_roi_bounds_validation`
- **Issue**: Expected validation failures not happening correctly

**Execution Time**: 34.22s (tests running and communicating with camera)

## Root Cause Analysis

### 1. Camera Detection - RESOLVED ✅

**Initial Issue**: Camera not detected without proper environment variables
**Solution**: Source PVCAM environment scripts:
```bash
source /opt/pvcam/etc/profile.d/pvcam.sh
source /opt/pvcam/etc/profile.d/pvcam-sdk.sh
```

These scripts set critical variables:
- `PVCAM_VERSION=3.10.0.3` - Library version identifier
- `PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode` - User-mode driver path
- `PVCAM_SDK_PATH=/opt/pvcam/sdk` - SDK installation path

**Result**: Camera now fully detected and operational

### 2. PVCAM SDK Installation ✅

**Library Status**: Correctly installed and registered
```
libpvcam.so.2.6         -> /opt/pvcam/library/x86_64/libpvcam.so.2.6
libpvcamDDI.so.3.1      -> /opt/pvcam/library/x86_64/libpvcamDDI.so.3.1
```

**Dependencies**: All satisfied (libdl, libpthread, librt, libstdc++, libgcc_s, libc)

### 3. PVCAM Test Tool Diagnostic

**PVCamTestCli** (official diagnostic tool):
```
[I] Found libpvcam.so.2
[I] Path '/opt/pvcam/library/x86_64/'
[E] PVCAM version UNKNOWN, library unloaded
[E] Failure loading mandatory PVCAM library!!!
```

Even Photometrics' own test tool cannot initialize the PVCAM library, indicating a deeper system-level issue.

### 4. System Configuration ✅

**USB Memory Limit**: Increased from 16MB → 512MB
```bash
cat /sys/module/usbcore/parameters/usbfs_memory_mb
# Result: 512 (sufficient for Prime BSI frame buffering)
```

**Kernel Modules**: No camera-specific modules loaded
```bash
lsmod | grep -i 'photo\|pvcam\|camera\|video'
# Result: Only generic 'video' module (for Intel i915 graphics)
```

## Compilation Fixes Applied ✅

The following issues were successfully resolved to achieve clean compilation:

### 1. Header Include Order (pvcam-sys/wrapper.h)

**Problem**: `pvcam.h` included before `master.h`, causing "unknown type name" errors
```c
// BEFORE (incorrect):
#include <pvcam.h>
#include <master.h>

// AFTER (correct):
#include <master.h>  // Must be first - defines uns32, uns16, int16, etc.
#include <pvcam.h>
```

### 2. Missing PVCAM Constants (pvcam-sys/src/lib.rs)

**Problem**: Bindgen didn't export enum values as constants
**Solution**: Manually defined constants with correct i16 type

```rust
pub const ATTR_CURRENT: i16 = 0;
pub const TIMED_MODE: i16 = 0;
pub const READOUT_NOT_ACTIVE: i16 = 0;
pub const EXPOSURE_IN_PROGRESS: i16 = 1;
pub const READOUT_IN_PROGRESS: i16 = 2;
pub const READOUT_COMPLETE: i16 = 3;
pub const READOUT_FAILED: i16 = 4;
```

### 3. Invalid Glob Import (pvcam-sys/src/lib.rs)

**Problem**: `pub use self::*;` after `include!()` macro
**Solution**: Removed - bindings already included

### 4. API Call Fixes (src/hardware/pvcam.rs)

**pl_pvcam_init() signature**:
```rust
// BEFORE:
let mut init_result: rs_bool = 0;
if pl_pvcam_init(&mut init_result) == 0 { ... }

// AFTER:
if pl_pvcam_init() == 0 { ... }  // Takes no arguments
```

**pl_set_param() pointer mutability**:
```rust
// BEFORE:
let x_bin_param = x_bin as uns16;
pl_set_param(h, PARAM_BINNING_SER, &x_bin_param as *const _ as *const _)

// AFTER:
let mut x_bin_param = x_bin as uns16;
pl_set_param(h, PARAM_BINNING_SER, &mut x_bin_param as *mut _ as *mut _)
```

### 5. Library Path Configuration (pvcam-sys/build.rs)

**Problem**: Linker couldn't find libpvcam (library in non-standard location)
**Solution**: Added `PVCAM_LIB_DIR` environment variable support

```rust
let sdk_lib_path = if let Ok(lib_dir) = env::var("PVCAM_LIB_DIR") {
    PathBuf::from(lib_dir)
} else {
    PathBuf::from(&sdk_dir).join("lib")
};
```

### 6. Dependency Installation

**Problem**: Missing build dependencies on EndeavourOS
**Solution**: Installed via pacman
```bash
sudo pacman -S --noconfirm clang llvm llvm-libs
```

## Next Steps to Fix Remaining Issues

### Implementation Fixes Needed

**1. Binning Parameter Handling** (6 test failures)

Current `pl_set_param` calls for binning are failing. Need to:
- Check PVCAM parameter IDs (PARAM_BINNING_SER, PARAM_BINNING_PAR may be incorrect)
- Verify parameter data types and sizes
- Test with actual camera to determine correct PVCAM API usage
- Consult PVCAM SDK documentation for binning configuration

**2. Frame Acquisition Sequence** (8 test failures)

Multiple acquisition failures suggest issues with:
- `pl_exp_setup_seq` - sequence setup parameters may be incorrect
- Frame buffer allocation - ensure sufficient buffer size
- Exposure timing - verify exposure time units (ms vs µs)
- Readout status polling - check `pl_exp_check_status` usage

**3. ROI Validation Logic** (3 test failures)

Validation not catching invalid ROIs:
- Implement bounds checking before calling PVCAM API
- Verify sensor dimensions are correctly retrieved
- Add proper error handling for out-of-bounds ROIs

### Current Test Command (Working)

```bash
source /opt/pvcam/etc/profile.d/pvcam.sh
source /opt/pvcam/etc/profile.d/pvcam-sdk.sh
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_LIB_DIR=/opt/pvcam/library/x86_64
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH

cargo test --test hardware_pvcam_validation \
  --features 'instrument_photometrics,pvcam_hardware,hardware_tests,pvcam-sys/pvcam-sdk' \
  -- --test-threads=1
```

### Success Metrics

- ✅ Camera detection: **WORKING**
- ✅ Basic initialization: **WORKING**
- ✅ Exposure control: **WORKING**
- ✅ ROI configuration: **WORKING (partial)**
- ✅ Triggering: **WORKING**
- ❌ Binning control: **NEEDS FIX**
- ❌ Frame acquisition: **NEEDS FIX**
- ❌ ROI validation: **NEEDS FIX**

## File Modifications Summary

### Modified Files (Committed: f500045e)

1. `pvcam-sys/wrapper.h` - Fixed include order
2. `pvcam-sys/src/lib.rs` - Added constants, removed invalid glob import
3. `pvcam-sys/build.rs` - Added PVCAM_LIB_DIR support
4. `src/hardware/pvcam.rs` - Fixed API calls

### Test File

- `tests/hardware_pvcam_validation.rs` - 28 tests ready (5 unit, 15 mock, 8 hardware)

## Conclusion

**Build Infrastructure**: ✅ Fully functional - code compiles, links, and runs
**Hardware Detection**: ✅ **WORKING** - Camera detected and communicating (required environment variables)
**Driver Integration**: ⚠️ **PARTIAL** - 9/28 tests passing, binning and acquisition need fixes
**Path Forward**: Fix binning API calls and frame acquisition sequence setup

The PVCAM driver integration has achieved **hardware communication** with the Prime BSI camera. Basic operations (initialization, exposure control, ROI, triggering) are working. Remaining issues are implementation details (binning parameters, acquisition sequence setup) that require PVCAM SDK documentation review and testing with actual hardware.

---

**Commits**:
- `f500045e` - fix(pvcam): resolve compilation and linking issues for PVCAM SDK integration
- `15ae421c` - docs(pvcam): initial test results (before environment variable fix)

**Working Test Command**:
```bash
source /opt/pvcam/etc/profile.d/pvcam.sh
source /opt/pvcam/etc/profile.d/pvcam-sdk.sh
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_LIB_DIR=/opt/pvcam/library/x86_64
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH

cargo test --test hardware_pvcam_validation \
  --features 'instrument_photometrics,pvcam_hardware,hardware_tests,pvcam-sys/pvcam-sdk' \
  -- --test-threads=1
```

**Test Results**: 9 passed, 19 failed (32% pass rate - significant progress from 0%)
