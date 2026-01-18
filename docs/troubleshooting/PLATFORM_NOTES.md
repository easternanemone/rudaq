# Platform Notes

Platform-specific setup, requirements, and troubleshooting for rust-daq.

## Linux

### GUI on Wayland

The `rust-daq-gui` binary requires Wayland and X11 support to be compiled in. This is enabled by default in the `daq-egui` crate.

**If you see this error:**
```
compile_error! ("The platform you're compiling for is not supported by winit");
```

**Cause:** The `eframe` dependency was built without windowing backend features.

**Solution:** Ensure `wayland` and `x11` features are enabled in eframe. This is the default configuration as of v0.5.x.

### Required System Libraries

For the GUI on Linux, you need:

```bash
# Debian/Ubuntu
sudo apt install libxkbcommon-dev libwayland-dev libxcb-shape0-dev libxcb-xfixes0-dev

# Fedora
sudo dnf install libxkbcommon-devel wayland-devel libxcb-devel

# Arch
sudo pacman -S libxkbcommon wayland libxcb
```

### XWayland Fallback

If running under Wayland but experiencing issues, you can force X11 mode:

```bash
WAYLAND_DISPLAY= cargo run --bin rust-daq-gui
```

### dark-light System Theme Detection (Temporarily Disabled)

**Status:** Disabled as of 2025-01-17 due to dependency conflict.

**Issue:** The `dark-light` crate (v2.0) depends on `ashpd` 0.10.x which conflicts with other dependencies requiring newer `zbus` versions.

**Error (if re-enabled):**
```
error[E0277]: the trait bound `zbus::Connection: From<zbus::Connection>` is not satisfied
```

**Current Workaround:** The `dark-light` feature is disabled in `daq-egui`. System theme detection falls back to dark mode. Users can still manually toggle themes in the GUI.

**To Re-enable (when fixed upstream):**
1. Check if `dark-light` 2.1+ or compatible `ashpd` update is released
2. Add `"dep:dark-light"` back to the `standalone` feature in `crates/daq-egui/Cargo.toml`
3. Remove the `#[cfg(feature = "dark-light")]` guards in `theme.rs`

## macOS

### Building

No special setup required. The GUI uses native Cocoa windowing.

### Code Signing (Distribution)

For distributing the GUI binary, you may need to sign and notarize:

```bash
codesign --sign "Developer ID Application: ..." target/release/rust-daq-gui
```

## Windows

### Building

Requires Visual Studio Build Tools with C++ workload for native dependencies.

### Serial Ports

Serial port access requires appropriate drivers for USB-serial adapters (FTDI, CH340, etc.).

## Cross-Platform

### HDF5 Storage

The `storage_hdf5` feature requires the HDF5 library:

- **Linux:** `sudo apt install libhdf5-dev`
- **macOS:** `brew install hdf5`
- **Windows:** Download from [HDF Group](https://www.hdfgroup.org/downloads/hdf5/)

### PVCAM (Photometrics Cameras)

See [PVCAM_SETUP.md](./PVCAM_SETUP.md) for detailed PVCAM SDK installation instructions.
