# Maitai Hardware Setup & Recovery Guide

## System Overview

- **Device Name**: `maitai`
- **IP Address**: `100.117.5.12`
- **OS**: EndeavourOS (Arch Linux rolling)
- **User**: `maitai`
- **Camera**: Teledyne Photometrics Prime BSI Express (USB 3.0)
- **Camera Identifier**: `pvcamUSB_0`

## PVCAM Installation & Configuration

### 1. Installation Source

The PVCAM driver installed appears to be a version from the AUR (`pvcam-dkms` or similar), but it requires manual configuration to function correctly with the Prime BSI Express.

### 2. Critical Configuration Files

#### `/opt/pvcam/pvcam.ini`

This file is **required** and may not be created by the installer.
**Path**: `/opt/pvcam/pvcam.ini`
**Content**:

```ini
[Versions]
PVCAM_VERSION=7.1.1.118

[Path]
PVCAM_ROOT=/opt/pvcam
PVCAM_USB_DRIVER_PATH=/opt/pvcam/drivers/user-mode/pvcam_usb.x86_64.umd
```

### 3. Environment Variables (Permanent Fix)

The system requires specific environment variables to locate the User Mode Driver (UMD) and the PVCAM library.
**File**: `/etc/profile.d/pvcam.sh` (Created to ensure global persistence)
**Content**:

```bash
export PVCAM_ROOT=/opt/pvcam
export PVCAM_DIR=/opt/pvcam
export PVCAM_VERSION=7.1.1.118
export LD_LIBRARY_PATH=/opt/pvcam/drivers/user-mode:/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
```

### 4. Permissions

- The user interacting with the camera must be in the `users` group (or whichever group owns the USB device in `/dev/bus/usb/`).
- Verified `maitai` user is in `users`, `video`, etc.
- USB Device ID: `1f12:0003` (Cypress FX3 - Bootloader) -> Becomes proper device after init.

## Recovery Procedure

If the camera is not detecting ("No cameras found" or "Installation Corrupted" error 151):

1. **Check Connection**: Ensure USB 3.0 cable is seated.
2. **Verify/Fix Environment**:

    ```bash
    source /etc/profile.d/pvcam.sh
    ```

3. **Check `pvcam.ini`**: Ensure it exists and points to the correct UMD path.
4. **Test Detection**:

    ```bash
    cd /opt/pvcam/sdk/examples/code_samples/bin/linux-x86_64/release
    ./ExtendedEnumerations
    ```

5. **Reboot**: If `ExtendedEnumerations` hangs or fails weirdly, reboot `maitai` to reset the USB bus and kernel module state.

## Known Issues & Workarounds

### 85-Frame Stall (Prime BSI Express)

**Symptom**: Continuous acquisition stops receiving frames after approximately 85 frames. The camera status reports `READOUT_NOT_ACTIVE` (0) and no new frames arrive.

**Cause**: This appears to be a firmware or driver quirk specific to the Prime BSI Express on this platform. It occurs regardless of buffer size or drain speed.

**Workaround**: The `rust-daq` PVCAM driver implements an aggressive auto-restart mechanism:
1.  **Detection**: If 2 consecutive polls (with ~100ms total timeout) return no frames and the status is `READOUT_NOT_ACTIVE`, a stall is declared.
2.  **Recovery**: The driver automatically:
    - Stops the acquisition (`CCS_CLEAR`).
    - Waits for a brief settling period.
    - Re-runs `pl_exp_setup_cont` and `pl_exp_start_cont`.
    - Re-registers the EOF callback.
3.  **Transparency**: The driver maintains a monotonic virtual frame counter, so downstream consumers see a continuous stream of frame numbers despite the internal hardware restarts. There may be a small timing gap (glitch) during the restart, but data flow resumes automatically.