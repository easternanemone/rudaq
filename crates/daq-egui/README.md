# daq-egui UI

The egui front-end ships two binaries:

- `daq-gui` (default): the main control panel.
- `daq-rerun` (feature `rerun_viewer`): an egui wrapper that embeds the Rerun viewer and listens for log streams on `0.0.0.0:9876`.

## PVCAM live view to Rerun

With `instrument_photometrics`, `pvcam_hardware`, `arrow`, and `driver_pvcam_arrow_tap` enabled, the left nav shows a toggle **"PVCAM Live to Rerun"**. When enabled, the app:

1. Connects to the Prime BSI via the PVCAM driver.
2. Streams u16 grayscale frames as `Tensor` records to the Rerun gRPC listener at `127.0.0.1:9876` under path `/pvcam/image`.
3. Stops streaming when the toggle is clicked again.

Environment needed for hardware:
```
PVCAM_SDK_DIR=/opt/pvcam/sdk
PVCAM_LIB_DIR=/opt/pvcam/library/x86_64
PVCAM_UMD_PATH=/opt/pvcam/drivers/user-mode
LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
```

Run the viewer and GUI together:
```
cargo run -p daq-egui --bin daq-rerun --features rerun_viewer &
cargo run -p daq-egui --bin daq-gui --features "instrument_photometrics,pvcam_hardware,arrow,driver_pvcam_arrow_tap"
```

If the viewer isn't running, the GUI will still attempt to connect; errors are logged to stderr.
