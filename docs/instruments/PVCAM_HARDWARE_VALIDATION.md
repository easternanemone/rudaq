# PVCAM Hardware Validation Report

## Summary

- **Date:** 2025-10-31
- **Tester:** Remote agent via tailnet
- **SDK Installed:** Photometrics PVCAM 3.10.0.3 (`/opt/pvcam`)
- **Camera:** PRIME-BSI (`pvcamUSB_0`, SN A19G204008)
- **Rust driver state:** `RealPvcamSdk` still uses mock paths; hardware acquisition via Rust is not yet wired.

## Steps Performed

1. Installed SDK with `pvcam-sdk_install_helper-Arch.sh`.
2. Verified installation:
   ```bash
   source /opt/pvcam/etc/profile.d/pvcam.sh
   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:/opt/pvcam/library/i686:$LD_LIBRARY_PATH
   /opt/pvcam/bin/VersionInformation/x86_64/VersionInformationCli
   ```
3. Captured 10 frames:
   ```bash
   /opt/pvcam/bin/PVCamTest/x86_64/PVCamTestCli \
     --acq-frames=10 --exposure=20ms \
     --save-as=tiff --save-dir=/home/maitai/pvcam_test_output \
     --save-first=10
   ```

## Findings

- PVCAM CLI reports SDK 3.10.0, camera details, throughput (~47.5 FPS).
- TIFF files saved under `/home/maitai/pvcam_test_output`.
- `libtiff` warning occurs but does not affect capture.
- No `/dev/video*`; interaction must use PVCAM SDK.

## Next Steps

- Implement real FFI in `src/instruments_v2/pvcam_sdk.rs` under `--features pvcam_hardware`.
- Allow configuration (`sdk_mode = "real"`) to switch driver paths.
- Add Rust smoke test and expand operator documentation once FFI is live.

## Important Notes

- Enabling hardware mode now requires `sdk_mode = "real"` in the `[[instruments_v3]]` entry and compiling with `--features pvcam_hardware`. Without the feature flag the driver returns `FeatureDisabled` errors.
- The V3 factory path (`PVCAMCameraV3::from_config`) populates exposure, ROI, binning, gain, and trigger defaults from configuration so the real driver must validate those inputs via `pl_get_param` before applying them.
- Create a follow-up smoke test (see bd-81 / hw-10) once FFI is wired so regressions in real mode are caught alongside CLI validation.

## Automated Smoke Test

An opt-in smoke test is available to verify real hardware connectivity. The test is disabled by default and only runs when the `PVCAM_SMOKE_TEST` environment variable is set.

```bash
# On the hardware host
source /opt/pvcam/etc/profile.d/pvcam.sh
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
export PVCAM_SMOKE_TEST=1
export PVCAM_CAMERA_NAME=PrimeBSI    # optional; defaults to PrimeBSI

cargo test --test pvcam_hardware_smoke \
  --features 'instrument_photometrics,pvcam_hardware' \
  -- --nocapture
```

The smoke test performs the following actions:

1. Initializes the PVCAM SDK and creates driver for specified camera.
2. Queries camera info (chip name, sensor dimensions, serial).
3. Sets a short exposure (10ms).
4. Starts continuous acquisition and waits for a frame (5s timeout).
5. Validates frame data (dimensions, buffer size).
6. Stops acquisition and reports statistics.

If `PVCAM_SMOKE_TEST` is unset the test prints a skip message and exits immediately. This allows the test to live in the repository without impacting CI environments that lack hardware access.

**CI Integration:** The smoke test is configured to run in `.github/workflows/ci.yml` under the `hardware-tests` job (main branch pushes only).
