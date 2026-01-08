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
