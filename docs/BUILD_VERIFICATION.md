# Hardware Build Verification Guide

## Critical Information

**The `maitai` feature flag enables ALL real hardware on the maitai machine.**

Building without this flag produces a daemon with MOCK hardware only, even though it may compile successfully. This document provides comprehensive verification steps.

## What the `maitai` Feature Includes

The `maitai` feature is defined in `crates/daq-bin/Cargo.toml` and includes:

```toml
maitai = [
    "pvcam_hardware",              # Real PVCAM SDK (NOT mock)
    "rust_daq/thorlabs",           # ELL14 rotators
    "rust_daq/newport",            # ESP300 motion controller
    "rust_daq/spectra_physics",    # MaiTai laser
    "rust_daq/newport_power_meter",# 1830-C power meter
    "rust_daq/serial"              # Serial port base
]
```

## Correct Build Process

**ALWAYS use the build script:**

```bash
bash scripts/build-maitai.sh
```

This script:
1. Sources PVCAM environment variables from `config/hosts/maitai.env`
2. Performs a **full** `cargo clean` (required for feature flag changes)
3. Builds with `--features maitai`
4. Shows verification checklist

**DO NOT manually run `cargo build` unless you:**
- Have sourced the environment: `source config/hosts/maitai.env`
- Have done a full clean: `cargo clean`
- Include the maitai feature: `cargo build --release -p daq-bin --features maitai`

## Post-Build Verification

### Step 1: Check Build Output

The build script should show:
```
🔧 Building daemon with ALL REAL HARDWARE (maitai feature)...
   PVCAM_SDK_DIR=/opt/pvcam/sdk
   PVCAM_VERSION=7.1.1.118

   Enabled drivers:
     ✓ PVCAM camera (real SDK)
     ✓ Thorlabs ELL14 rotators
     ✓ Newport ESP300 motion controller
     ✓ Newport 1830-C power meter
     ✓ Spectra-Physics MaiTai laser
     ✓ Serial port communication
```

### Step 2: Start Daemon and Check Log

```bash
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml 2>&1 | tee daemon.log
```

### Step 3: Verify PVCAM SDK Initialization

Search the daemon log for PVCAM initialization:

```bash
grep -i "pvcam" daemon.log
```

**MUST show:**
```
pvcam_sdk feature enabled: true
PVCAM SDK initialized successfully (ref count: 1)
Successfully opened camera 'pvcamUSB_0' with handle 0
```

**If you see:**
```
pvcam_sdk feature enabled: false
using mock mode
```

**The build is INCORRECT - stop the daemon and rebuild with the script!**

### Step 4: Verify All 7 Devices Registered

```bash
grep "Registered.*device(s)" daemon.log -A 10
```

**Expected output:**
```
Registered 7 device(s)
  - prime_bsi: Photometrics Prime BSI Camera ([Triggerable, FrameProducer, ExposureControl, Parameterized])
  - maitai: MaiTai Ti:Sapphire Laser ([Readable, Parameterized, ShutterControl, EmissionControl, WavelengthTunable])
  - power_meter: Newport 1830-C Power Meter ([Readable, Parameterized, WavelengthTunable])
  - rotator_2: ELL14 Rotator (Address 2) ([Movable, Parameterized])
  - rotator_3: ELL14 Rotator (Address 3) ([Movable, Parameterized])
  - rotator_8: ELL14 Rotator (Address 8) ([Movable, Parameterized])
  - esp300_axis1: ESP300 Axis 1 ([Movable, Parameterized])
```

### Step 5: GUI Verification

1. Build GUI (does NOT require hardware features):
   ```bash
   cargo build --release -p daq-egui --bin rust-daq-gui
   ```

2. Launch GUI:
   ```bash
   ./target/release/rust-daq-gui --daemon-url http://localhost:50051
   ```

3. Check Instruments Panel:
   - Should show ALL 7 devices
   - Each device should have a control panel
   - Camera ImageViewer should stream REAL images (not synthetic gradients)

### Step 6: Camera Stream Verification

The most definitive test for real hardware:

1. In GUI, open ImageViewer for `prime_bsi`
2. Start streaming
3. **Mock mode shows:** Perfect gradients or uniform patterns
4. **Real hardware shows:** Actual camera noise, sensor hot pixels, real scene

## Troubleshooting

### Problem: Only Mock Devices Show Up

**Cause:** Built without `maitai` feature

**Solution:**
```bash
cargo clean
bash scripts/build-maitai.sh
```

### Problem: Daemon Shows "using mock mode"

**Cause:** PVCAM SDK feature not enabled in build

**Solution:** Same as above - rebuild with script

### Problem: Some Devices Missing

**Causes:**
1. Hardware not powered on
2. Serial port permissions
3. Wrong port configuration in `config/maitai_hardware.toml`

**Check:**
```bash
# List USB serial devices
ls -la /dev/ttyUSB* /dev/ttyS*

# Check PVCAM cameras
ls -la /dev/pvcamUSB*
```

### Problem: Build Script Fails

**Check environment:**
```bash
source config/hosts/maitai.env
echo $PVCAM_SDK_DIR
echo $PVCAM_VERSION
```

Should show:
```
/opt/pvcam/sdk
7.1.1.118
```

## Common Mistakes

### ❌ WRONG: Manual build without clean

```bash
cargo build --release -p daq-bin --features maitai
```

Problem: Cargo caches feature flags in dependencies. A previous build without `maitai` will keep using mock mode even if you add the feature flag.

### ❌ WRONG: Building GUI with hardware features

```bash
cargo build -p daq-egui --features maitai
```

Problem: GUI doesn't need hardware features (it connects remotely). This wastes compile time.

### ❌ WRONG: Building without environment sourced

```bash
cargo build --features maitai
```

Problem: PVCAM SDK requires environment variables. Build will fail or use mock mode.

### ✅ CORRECT: Use the build script

```bash
bash scripts/build-maitai.sh
```

## Quick Reference

```bash
# Correct workflow
bash scripts/build-maitai.sh                    # Build daemon with ALL hardware
cargo build --release -p daq-egui              # Build GUI (separate)

# Start daemon
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml

# Verify
grep "Registered.*device(s)" <daemon_output> -A 10
grep "pvcam_sdk feature enabled" <daemon_output>

# Expected: 7 devices, pvcam_sdk=true
```

## See Also

- `CLAUDE.md` - Main project documentation
- `scripts/build-maitai.sh` - Build script
- `crates/daq-bin/Cargo.toml` - Feature flag definitions
- `config/maitai_hardware.toml` - Hardware configuration
