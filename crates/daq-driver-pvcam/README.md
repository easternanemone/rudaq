# daq-driver-pvcam

PVCAM camera driver crate used by rust-daq. Supports mock mode by default and hardware mode when the PVCAM SDK is available.

## Features
- `mock` (default): builds without the PVCAM SDK, uses mock acquisition.
- `pvcam_hardware`: enables PVCAM SDK bindings (requires env vars).
- `arrow_tap`: exposes an Arrow tap channel to receive frames as `UInt16Array`.

## Environment (hardware)
Set before running hardware tests/examples:
```
PVCAM_SDK_DIR=/opt/pvcam/sdk
PVCAM_LIB_DIR=/opt/pvcam/library/x86_64
PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode
LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
```

## Examples
- Mock only: `cargo test -p daq-driver-pvcam --no-default-features`
- Hardware smoke: `cargo test -p daq-driver-pvcam --features pvcam_hardware`
- Arrow tap (hardware + Arrow):
  ```bash
  PVCAM_SDK_DIR=/opt/pvcam/sdk \
  PVCAM_LIB_DIR=/opt/pvcam/library/x86_64 \
  PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode \
  LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH \
  cargo run -p daq-driver-pvcam --example arrow_tap --features "pvcam_hardware,arrow_tap" -- PrimeBSI
  ```
