# PVCAM Setup and Troubleshooting

This guide details the setup and troubleshooting steps for the Photometrics PVCAM driver on Linux, specifically for high-bandwidth cameras like the Prime BSI.

## Prerequisites

- **OS**: Arch Linux (verified), Ubuntu/Debian (supported by PVCAM SDK)
- **SDK**: PVCAM SDK 3.10.2.5 or later
- **Hardware**: Photometrics Camera (e.g., Prime BSI, Prime 95B) via USB 3.0 or PCIe

## Installation

1. **Install PVCAM SDK**: Follow the official instructions to install the SDK.
2. **Kernel Module**: Ensure the `pvcam` kernel module is loaded:

    ```bash
    sudo modprobe pvcam
    lsmod | grep pvcam
    ```

3. **Permissions**: Ensure your user is in the `video` or `users` group (depending on udev rules).

## Environment Setup

Before building or running PVCAM code, set these environment variables:

```bash
# CRITICAL: PVCAM_VERSION is required at runtime
export PVCAM_VERSION=7.1.1.118

# SDK and library locations
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export PVCAM_LIB_DIR=/opt/pvcam/library/x86_64

# LIBRARY_PATH for linker (required for cargo build)
export LIBRARY_PATH=$PVCAM_LIB_DIR:$LIBRARY_PATH

# LD_LIBRARY_PATH for runtime
export LD_LIBRARY_PATH=/opt/pvcam/drivers/user-mode:$PVCAM_LIB_DIR:$LD_LIBRARY_PATH
```

**Quick setup:** Source the PVCAM profile (sets most variables), then add linker path:
```bash
source /etc/profile.d/pvcam.sh
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
```

## Troubleshooting

### Error 151: PL_ERR_INSTALLATION_CORRUPTED

**Symptoms:**
- `PVCAM version UNKNOWN, library unloaded`
- `Failure loading mandatory PVCAM library`
- Error code 151 from PVCAM functions

**Cause:**
The `PVCAM_VERSION` environment variable is not set. Despite the misleading error message, this is NOT an installation corruption issue.

**Solution:**
```bash
export PVCAM_VERSION=7.1.1.118  # Check /opt/pvcam/pvcam.ini for your version
```

Or source the profile script:
```bash
source /etc/profile.d/pvcam.sh
```

### Linker error: "unable to find library -lpvcam"

**Symptoms:**
- Rust build fails with `rust-lld: error: unable to find library -lpvcam`

**Cause:**
The `LIBRARY_PATH` environment variable is not set for the linker.

**Solution:**
```bash
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
```

### "Failed to start continuous acquisition"

**Symptoms:**

- Camera is detected.
- Metadata (serial, firmware) can be read.
- `acquire_frame()` fails immediately with an error indicating acquisition start failure.

**Cause:**
This is often caused by insufficient USB memory buffer allocation in the Linux kernel. The default `usbfs_memory_mb` is typically 16MB or 200MB, which is too small for high-bandwidth cameras like the Prime BSI (which requires ~16MB *per frame* and allocates multiple buffers).

**Solution:**
Increase the `usbfs_memory_mb` limit to at least 1000MB.

**Temporary Fix:**

```bash
echo 1000 | sudo tee /sys/module/usbcore/parameters/usbfs_memory_mb
```

**Permanent Fix (Systemd):**
Create a systemd service to apply this setting on boot:

`/etc/systemd/system/pvcam-usb-buffer.service`:

```ini
[Unit]
Description=Set USBFS memory limit for PVCAM
After=network.target

[Service]
Type=oneshot
ExecStart=/bin/sh -c "echo 1000 > /sys/module/usbcore/parameters/usbfs_memory_mb"
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable pvcam-usb-buffer
sudo systemctl start pvcam-usb-buffer
```

### "Failed to open camera"

**Symptoms:**

- Application panics or returns error during `PvcamDriver::new()`.

**Checklist:**

1. **Kernel Module**: Run `lsmod | grep pvcam`. If missing, reinstall drivers or run `sudo depmod -a && sudo modprobe pvcam`.
2. **USB Connection**: Run `lsusb`. Look for Photometrics device (ID `1f12:xxxx`).
3. **Environment Variables**: Ensure `PVCAM_SDK_DIR` etc. are set correctly (see crate README).
4. **Lockfile**: Check for stale lockfiles if a previous process crashed.

### 16-bit Pixel Depth

**Note**: Most scientific cameras (Prime BSI included) return 16-bit data (`u16`).

- **Buffer Size**: When verifying frame data manually, remember `buffer_size_bytes = width * height * 2`.
- **Binning**: Some cameras support flexible binning (3x3, 5x5) even if not officially advertised in simple spec sheets. The driver typically queries the hardware for validity.

### Error 185: PL_ERR_CONFIGURATION_INVALID with CIRC_OVERWRITE

**Symptoms:**
- `pl_exp_setup_cont()` succeeds
- `pl_exp_start_cont()` fails with error 185
- Occurs when using `CIRC_OVERWRITE` buffer mode

**Cause:**
Prime BSI cameras often reject `CIRC_OVERWRITE` buffer mode on certain firmware versions.

**Solution:**
The `rust-daq` driver automatically detects this error and falls back to `CIRC_NO_OVERWRITE` mode. No user intervention is required.

### Continuous Acquisition Stalls After ~85 Frames

**Symptoms:**
- Continuous acquisition starts successfully
- Works for first ~85 frames
- Then stalls or stops receiving new frames (status `READOUT_NOT_ACTIVE`)

**Cause:**
Hardware/Firmware Errata. The Prime BSI camera stops producing frames after ~85 frames in continuous mode, regardless of buffer size or drain speed.

**Solution:**
The driver implements an automatic stall detection and recovery mechanism:
1.  Detects stall (2 consecutive timeouts + status 0).
2.  Stops acquisition (`CCS_CLEAR`).
3.  Restarts acquisition transparently.
4.  Maintains monotonic frame numbering to hide the reset from downstream consumers.

This allows sustained streaming indefinitely despite the hardware limitation.

### Camera Stuck in EXPOSURE_IN_PROGRESS (Status 1)

**Symptoms:**
- `pl_exp_check_cont_status()` continuously returns `Status: 1` (EXPOSURE_IN_PROGRESS)
- No frames are received despite streaming being enabled
- Logs show repeated timeouts:
  ```
  [PVCAM DEBUG] Timeouts: 1, Status: 1, Bytes: 0, BufferCnt: 0, streaming: true
  [PVCAM DEBUG] Breaking due to max consecutive timeouts
  ```

**Cause:**
The camera firmware is stuck in acquisition mode, typically from a **dirty shutdown** (e.g., using `pkill` or `kill -9` to terminate the daemon). When the daemon is killed abruptly, it bypasses the cleanup code that would normally call `pl_exp_abort()` and `pl_cam_close()`, leaving the camera hardware in an inconsistent state.

**Solution:**
Use the `force_reset` example to clear the stuck camera state:

```bash
cd /path/to/rust-daq
source config/hosts/maitai.env  # or scripts/env-check.sh

# Run the force reset tool
cargo run --example force_reset --features pvcam_sdk -p daq-driver-pvcam

# Expected output:
# Initializing PVCAM...
# Found 1 cameras.
# Opening camera 0: pvcamUSB_0
#   Camera opened. Handle: 0
#   Aborting acquisition (pl_exp_abort with CCS_HALT)...
#   Acquisition aborted.
#   Resetting PP features (pl_pp_reset)...
#   PP features reset.
#   Closing camera...
#   Camera closed.
# Uninitializing PVCAM...
# Done.
```

If the software reset doesn't work (camera still stuck), a **physical power cycle** of the camera may be required.

**Prevention:**
- Always stop the daemon gracefully with `Ctrl+C` when possible
- If using systemd, use `systemctl stop rust-daq-daemon`
- Avoid `pkill -9` or `kill -9` which bypass cleanup handlers

### Dirty Shutdown Recovery

**Symptoms:**
After killing the daemon with `pkill`, `kill -9`, or a system crash:
- Camera won't stream on next daemon start
- Camera stuck in EXPOSURE_IN_PROGRESS
- `pl_cam_open` may fail or hang

**Recovery Steps:**

1. **Try software reset first:**
   ```bash
   cargo run --example force_reset --features pvcam_sdk -p daq-driver-pvcam
   ```

2. **If software reset fails, power cycle the camera:**
   - Disconnect USB cable
   - Wait 5 seconds
   - Reconnect USB cable
   - Wait for device enumeration (check `lsusb`)

3. **Restart the daemon:**
   ```bash
   source config/hosts/maitai.env
   ./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml
   ```

## Verification

Run the hardware validation suite:

```bash
cargo test --test hardware_pvcam_validation --features "pvcam_sdk,hardware_tests" -- --test-threads=1
```

(Note: `--test-threads=1` is recommended to avoid resource contention on the USB bus during tests).
