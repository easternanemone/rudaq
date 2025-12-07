# PVCAM Camera Operator Guide

## Overview

This guide covers operation of Photometrics cameras (Prime BSI, Prime 95B) through the rust-daq PVCAM driver.

## V5 Reactive Parameter System

The PVCAM driver uses the V5 `Parameter<T>` reactive system, making all camera state observable and controllable via gRPC.

### Observable Parameters

All camera settings are exposed as parameters accessible via `ListParameters`/`GetParameter`/`SetParameter` gRPC methods:

| Parameter | Type | Description |
|-----------|------|-------------|
| `exposure_ms` | `f64` | Exposure time (0.1-60000 ms) |
| `roi` | `Roi` | Region of interest |
| `binning` | `(u16, u16)` | Binning factors (x, y) |
| `armed` | `bool` | Trigger armed state |
| `streaming` | `bool` | Continuous streaming active |
| `temperature` | `f64` | Current sensor temperature (read-only) |
| `temperature_setpoint` | `f64` | Target cooling temperature (-25 to 25°C) |
| `fan_speed` | `String` | Fan speed (High/Medium/Low/Off) |
| `gain_index` | `u16` | Current gain mode index (0-3) |
| `speed_index` | `u16` | Current speed/readout mode index (0-1) |

### Real-time Updates

When hardware methods like `set_gain_index()` or `get_temperature()` are called, the corresponding parameter is automatically updated. Remote GUI clients subscribed to parameters receive real-time notifications of state changes.

```rust
// Hardware change propagates to parameter automatically
camera.set_gain_index(2).await?;  // Updates gain_index parameter
let temp = camera.get_temperature().await?;  // Updates temperature parameter
```

## Supported Cameras

| Camera | Sensor Size | Pixel Size | ADC | Typical Use |
|--------|-------------|------------|-----|-------------|
| Prime BSI | 2048 x 2048 | 6.5 um | 11-bit | High-sensitivity imaging |
| Prime 95B | 1200 x 1200 | 11 um | 16-bit | Low-light applications |

## Quick Start

### 1. Verify Camera Connection

```bash
# Check if PVCAM SDK recognizes the camera
source /opt/pvcam/etc/profile.d/pvcam.sh
/opt/pvcam/bin/VersionInformation/x86_64/VersionInformationCli
```

Expected output includes camera serial number and firmware version.

### 2. Capture Test Frames (CLI Tool)

```bash
/opt/pvcam/bin/PVCamTest/x86_64/PVCamTestCli \
  --acq-frames=10 \
  --exposure=20ms \
  --save-as=tiff \
  --save-dir=/tmp/pvcam_test \
  --save-first=10
```

### 3. Configuration

Add to your `config/default.toml`:

```toml
[instruments.prime_bsi]
type = "pvcam"
name = "Prime BSI Camera"
camera_name = "PrimeBSI"  # or "Prime95B"
sdk_mode = "real"         # or "mock" for testing
exposure_ms = 100.0       # Default exposure
gain_index = 1            # 0-3 depending on mode
speed_index = 0           # 0=5ns/pixel, 1=10ns/pixel
polling_rate_hz = 10.0    # Frame polling rate
```

## Camera Settings

### Gain Modes

| Index | Name | Description |
|-------|------|-------------|
| 0 | (Default) | Standard sensitivity |
| 1 | Full well | Maximum well capacity for bright samples |
| 2 | Balanced | Balance between sensitivity and well depth |
| 3 | Sensitivity | Maximum sensitivity for dim samples |

**Selecting Gain Mode:**
```rust
camera.set_gain_index(2).await?;  // Set to Balanced
let gain = camera.get_gain().await?;
println!("Current gain: {} - {}", gain.index, gain.name);
```

### Speed Modes

| Index | Name | Frame Rate | Use Case |
|-------|------|------------|----------|
| 0 | 5 ns/pixel | Higher | Fast imaging, time-lapse |
| 1 | 10 ns/pixel | Lower | Low-noise, scientific |

**Selecting Speed Mode:**
```rust
camera.set_speed_index(0).await?;  // Fastest readout
let speed = camera.get_speed().await?;
println!("Current speed: {}", speed.name);
```

### Exposure Time

```rust
// Set exposure to 50ms
camera.set_exposure_ms(50.0).await?;

// Query current exposure
let exposure = camera.get_exposure_ms().await?;
println!("Exposure: {} ms", exposure);
```

Valid range: Typically 0.1ms to 60000ms (camera-dependent)

### Binning

```rust
// Set 2x2 binning (increases sensitivity, reduces resolution)
camera.set_binning(2, 2).await?;

// Valid values: 1, 2, 4, 8
// Frame size = sensor_size / binning_factor
```

### Region of Interest (ROI)

```rust
use rust_daq::hardware::Roi;

// Set center quarter of sensor
let roi = Roi {
    x: 512,       // Start X
    y: 512,       // Start Y
    width: 1024,  // ROI width
    height: 1024, // ROI height
};
camera.set_roi(roi).await?;
```

## Temperature Control

The Prime BSI uses thermoelectric cooling for low-noise operation.

```rust
// Query current temperature
let temp = camera.get_temperature().await?;
println!("Sensor: {:.1}C", temp);

// Query setpoint
let setpoint = camera.get_temperature_setpoint().await?;
println!("Target: {:.1}C", setpoint);

// Fan speed control
use rust_daq::hardware::pvcam::FanSpeed;
camera.set_fan_speed(FanSpeed::High).await?;  // High, Medium, Low
```

**Typical Values:**
- Operating temperature: -20C (cooled)
- Setpoint: -20C
- Difference should be <1C when stabilized

## Advanced Features

### Smart Streaming (Multi-Exposure HDR)

Captures a sequence of frames at different exposures:

```rust
// Enable Smart Streaming
camera.enable_smart_streaming().await?;

// Set exposure sequence (short, medium, long)
let exposures = vec![1.0, 10.0, 100.0]; // milliseconds
camera.set_smart_stream_exposures(&exposures).await?;

// Disable when done
camera.disable_smart_streaming().await?;
```

### PrimeLocate (Centroids/Particle Tracking)

Hardware-accelerated particle detection:

```rust
use rust_daq::hardware::pvcam::{CentroidsMode, CentroidsConfig};

// Enable centroids
camera.enable_centroids().await?;

// Configure detection
let config = CentroidsConfig {
    mode: CentroidsMode::Locate,  // or Track, Blob
    radius: 5,                     // Detection radius (pixels)
    max_count: 100,               // Maximum particles
    threshold: 1000,              // Detection threshold
};
camera.set_centroids_config(&config).await?;
```

### PrimeEnhance (Hardware Denoising)

Real-time noise reduction:

```rust
// Enable denoising
camera.enable_prime_enhance().await?;

// Configure (optional)
camera.set_prime_enhance_iterations(3).await?;

// Query parameters
let iterations = camera.get_prime_enhance_iterations().await?;
let gain = camera.get_prime_enhance_gain().await?;
let offset = camera.get_prime_enhance_offset().await?;
let lambda = camera.get_prime_enhance_lambda().await?;

println!("Denoising: iter={}, gain={}, offset={}, lambda={}",
         iterations, gain, offset, lambda);
```

### Post-Processing Features

List and configure post-processing filters:

```rust
// List available features
let features = camera.list_pp_features().await?;
for feat in &features {
    println!("[{}] {}", feat.index, feat.name);
}

// Get parameters for a feature
let params = camera.get_pp_params(feature_index).await?;

// Reset all to defaults
camera.reset_pp_features().await?;
```

**Available Features on Prime BSI:**
- DESPECKLE BRIGHT LOW/HIGH
- DESPECKLE DARK LOW/HIGH
- DENOISING
- QUANTVIEW

## Frame Acquisition

### Single Frame

```rust
use rust_daq::hardware::capabilities::FrameProducer;

// Acquire single frame
let frame = camera.acquire_frame().await?;
println!("Frame: {}x{} pixels, {} values",
         frame.width, frame.height, frame.buffer.len());
```

### Triggered Acquisition

```rust
use rust_daq::hardware::capabilities::Triggerable;

// Arm for external trigger
camera.arm().await?;

// Wait for trigger (with timeout)
match tokio::time::timeout(
    Duration::from_secs(10),
    camera.wait_for_trigger()
).await {
    Ok(Ok(())) => println!("Trigger received!"),
    Ok(Err(e)) => println!("Trigger error: {}", e),
    Err(_) => println!("Timeout waiting for trigger"),
}

// Disarm
camera.disarm().await?;
```

### Continuous Streaming

```rust
// Start continuous acquisition
camera.start_continuous().await?;

// Get frame channel
let mut rx = camera.take_frame_receiver().await?;

// Process frames
while let Some(frame) = rx.recv().await {
    process_frame(&frame);
}

// Stop streaming
camera.stop_continuous().await?;
```

## Troubleshooting

### Camera Not Detected

1. Check USB connection (Prime BSI uses USB 3.0)
2. Verify PVCAM daemon is running:
   ```bash
   ps aux | grep pvcam
   ```
3. Check udev rules are installed:
   ```bash
   ls -la /etc/udev/rules.d/*pvcam*
   ```

### Build Errors

Ensure environment variables are set:
```bash
source /opt/pvcam/etc/profile.d/pvcam.sh
export PVCAM_SDK_DIR=/opt/pvcam/sdk
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
```

### "Parameter not available" Errors

Some parameters (like speed table name) are not available on all cameras. These errors are informational and don't affect functionality.

### Temperature Not Stable

Wait for camera to cool down after power-on. Typical stabilization time: 5-10 minutes. Check fan speed is set appropriately.

### Frame Acquisition Timeout

1. Verify exposure time isn't excessively long
2. Check for external trigger if in triggered mode
3. Verify camera isn't in an error state

## API Quick Reference

### Capability Traits Implemented

| Trait | Methods |
|-------|---------|
| `FrameProducer` | `acquire_frame()`, `start_continuous()`, `stop_continuous()` |
| `ExposureControl` | `set_exposure_ms()`, `get_exposure_ms()` |
| `Triggerable` | `arm()`, `disarm()`, `wait_for_trigger()` |

### Camera-Specific Methods

```rust
// Binning
camera.set_binning(x, y).await?;
camera.binning().await;

// ROI
camera.set_roi(roi).await?;
camera.roi().await;

// Gain and Speed
camera.set_gain_index(idx).await?;
camera.get_gain().await?;
camera.list_gain_modes().await?;
camera.set_speed_index(idx).await?;
camera.get_speed().await?;
camera.list_speed_modes().await?;

// Temperature
camera.get_temperature().await?;
camera.get_temperature_setpoint().await?;
camera.set_fan_speed(speed).await?;
camera.get_fan_speed().await?;

// Camera Info
camera.get_camera_info().await?;
camera.get_chip_name().await?;
camera.get_bit_depth().await?;
camera.get_readout_time_us().await?;
camera.get_pixel_size_nm().await?;
```

## References

- PVCAM SDK Documentation: `/opt/pvcam/sdk/doc/`
- Driver Source: `src/hardware/pvcam.rs`
- Test Suite: `tests/hardware_pvcam_validation.rs`
- Validation Report: `docs/instruments/PVCAM_VALIDATION_CHECKLIST.md`
