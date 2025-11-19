# V4 Trait Usage Guide

**Date:** 2025-11-17
**Status:** Phase 1D Reference Documentation
**Version:** 1.0

---

## Overview

This guide provides comprehensive examples for implementing V4 meta-instrument traits with Kameo actors. All Phase 1D traits follow consistent patterns for error handling, state management, and Arrow data serialization.

## Table of Contents

1. [Quick Start](#quick-start)
2. [Trait Implementations](#trait-implementations)
   - [CameraSensor](#camerasensor-trait)
   - [MotionController](#motioncontroller-trait)
   - [ScpiEndpoint](#scpiendpoint-trait)
3. [Hardware Adapters](#hardware-adapters)
   - [VisaAdapterV4](#visaadapterv4)
   - [SerialAdapterV4](#serialadapterv4)
4. [Error Handling Patterns](#error-handling-patterns)
5. [Arrow Conversion Examples](#arrow-conversion-examples)
6. [Testing Strategies](#testing-strategies)

---

## Quick Start

### Creating a New Instrument Actor

All V4 instrument actors follow this pattern:

```rust
use kameo::prelude::*;
use anyhow::Result;
use crate::traits::{TunableLaser, LaserMeasurement};
use crate::hardware::SerialAdapterV4;

/// Example laser actor implementing TunableLaser trait
pub struct MyLaser {
    adapter: Option<SerialAdapterV4>,
    wavelength_nm: f64,
    shutter_open: bool,
}

impl kameo::Actor for MyLaser {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(
        args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        // Hardware initialization here
        if let Some(ref adapter) = args.adapter {
            adapter.connect().await?;
        }
        Ok(args)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        // Cleanup here
        Ok(())
    }
}
```

---

## Trait Implementations

### CameraSensor Trait

**File:** `src/traits/camera_sensor.rs`

#### Basic Implementation

```rust
use crate::traits::{
    CameraSensor, Frame, CameraStreamConfig, CameraTiming,
    RegionOfInterest, BinningConfig, CameraCapabilities,
    PixelFormat, TriggerMode,
};
use anyhow::{Result, Context};
use async_trait::async_trait;

pub struct PVCAMActor {
    streaming: bool,
    capabilities: CameraCapabilities,
    current_roi: RegionOfInterest,
    current_timing: CameraTiming,
    current_gain: u8,
    current_binning: BinningConfig,
}

#[async_trait]
impl CameraSensor for PVCAMActor {
    async fn start_stream(&self, config: CameraStreamConfig) -> Result<()> {
        // 1. Validate configuration
        if config.roi.width > self.capabilities.sensor_width {
            anyhow::bail!("ROI width exceeds sensor dimensions");
        }

        // 2. Configure camera hardware
        // ... PVCAM SDK calls here

        // 3. Start acquisition thread
        // ... tokio::spawn for frame capture

        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        // Stop acquisition, clear queue
        Ok(())
    }

    fn is_streaming(&self) -> bool {
        self.streaming
    }

    async fn snap_frame(&self, config: &CameraTiming) -> Result<Frame> {
        // Single frame acquisition with timeout
        let frame = Frame {
            timestamp_ns: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            frame_number: 0,
            pixel_format: PixelFormat::Mono16,
            width: 1024,
            height: 1024,
            roi: self.current_roi.clone(),
            pixel_data: vec![0u8; 1024 * 1024 * 2], // Mono16 = 2 bytes/pixel
        };

        Ok(frame)
    }

    async fn configure_roi(&self, roi: RegionOfInterest) -> Result<()> {
        // Validate ROI bounds
        if roi.x + roi.width > self.capabilities.sensor_width {
            anyhow::bail!("ROI exceeds sensor width");
        }

        // Apply to hardware
        Ok(())
    }

    async fn set_timing(&self, timing: CameraTiming) -> Result<()> {
        if timing.exposure_us < self.capabilities.min_exposure_us {
            anyhow::bail!(
                "Exposure {}µs below minimum {}µs",
                timing.exposure_us,
                self.capabilities.min_exposure_us
            );
        }

        Ok(())
    }

    async fn set_gain(&self, gain: u8) -> Result<()> {
        if gain > 100 {
            anyhow::bail!("Gain must be 0-100, got {}", gain);
        }

        Ok(())
    }

    async fn set_binning(&self, binning: BinningConfig) -> Result<()> {
        if binning.x_bin > self.capabilities.max_binning_x {
            anyhow::bail!(
                "X binning {} exceeds maximum {}",
                binning.x_bin,
                self.capabilities.max_binning_x
            );
        }

        Ok(())
    }

    fn get_capabilities(&self) -> CameraCapabilities {
        self.capabilities.clone()
    }
}
```

#### Frame Streaming Pattern

```rust
async fn start_stream(&self, config: CameraStreamConfig) -> Result<()> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Frame>(10); // Bounded capacity

    // Spawn acquisition task
    let adapter = self.adapter.clone();
    tokio::spawn(async move {
        loop {
            // 1. Capture frame from hardware
            let frame = capture_frame_from_hardware().await;

            // 2. Send to channel (drops oldest if full)
            if tx.send(frame).await.is_err() {
                tracing::warn!("Frame dropped - queue full");
            }

            // 3. Check for stop signal
            if should_stop() {
                break;
            }
        }
    });

    Ok(())
}
```

---

### MotionController Trait

**File:** `src/traits/motion_controller.rs`

#### Basic Implementation

```rust
use crate::traits::{
    MotionController, AxisPosition, AxisState, MotionConfig,
    MotionEvent,
};
use anyhow::{Result, Context};
use async_trait::async_trait;

pub struct ESP300Actor {
    adapter: SerialAdapterV4,
    num_axes: u8,
    axis_configs: Vec<MotionConfig>,
}

#[async_trait]
impl MotionController for ESP300Actor {
    async fn move_absolute(&self, axis: u8, position: f64) -> Result<()> {
        // 1. Validate axis
        if axis >= self.num_axes {
            anyhow::bail!("Axis {} out of range (max: {})", axis, self.num_axes - 1);
        }

        // 2. Check soft limits
        let config = &self.axis_configs[axis as usize];
        if position < config.min_position || position > config.max_position {
            anyhow::bail!(
                "Position {:.3} outside soft limits [{:.3}, {:.3}]",
                position,
                config.min_position,
                config.max_position
            );
        }

        // 3. Send command (ESP300: "1PA10.5" = axis 1, position absolute, 10.5mm)
        let cmd = format!("{}PA{:.3}", axis + 1, position);
        self.adapter
            .send_command(&cmd)
            .await
            .with_context(|| format!("Move absolute failed for axis {}", axis))?;

        Ok(())
    }

    async fn move_relative(&self, axis: u8, delta: f64) -> Result<()> {
        // Read current position
        let current = self.read_position(axis).await?;

        // Move to current + delta (soft limit check in move_absolute)
        self.move_absolute(axis, current + delta).await
    }

    async fn stop(&self, axis: Option<u8>) -> Result<()> {
        match axis {
            Some(ax) => {
                // Stop single axis (ESP300: "1ST" = axis 1 stop)
                let cmd = format!("{}ST", ax + 1);
                self.adapter.send_command(&cmd).await?;
            }
            None => {
                // Stop all axes
                for ax in 0..self.num_axes {
                    let cmd = format!("{}ST", ax + 1);
                    self.adapter.send_command(&cmd).await?;
                }
            }
        }

        Ok(())
    }

    async fn home(&self, axis: u8) -> Result<()> {
        // ESP300: "1OR" = axis 1 home search
        let cmd = format!("{}OR", axis + 1);

        // Start homing
        self.adapter.send_command(&cmd).await?;

        // Poll for completion (max 30s)
        let timeout = tokio::time::Duration::from_secs(30);
        let start = tokio::time::Instant::now();

        loop {
            let state = self.read_axis_state(axis).await?;

            match state.state {
                AxisState::Idle => {
                    // Homing complete
                    return Ok(());
                }
                AxisState::Homing => {
                    // Still homing, continue polling
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
                AxisState::Faulted => {
                    anyhow::bail!("Homing failed - axis in error state");
                }
                _ => {}
            }

            if start.elapsed() > timeout {
                anyhow::bail!("Homing timeout after 30 seconds");
            }
        }
    }

    async fn read_position(&self, axis: u8) -> Result<f64> {
        // ESP300: "1TP?" = axis 1 tell position
        let cmd = format!("{}TP?", axis + 1);
        let response = self.adapter.send_command(&cmd).await?;

        response
            .trim()
            .parse::<f64>()
            .with_context(|| format!("Failed to parse position: {}", response))
    }

    async fn read_axis_state(&self, axis: u8) -> Result<AxisPosition> {
        let position = self.read_position(axis).await?;

        // ESP300: "1TS?" = axis 1 tell status
        let cmd = format!("{}TS?", axis + 1);
        let status_response = self.adapter.send_command(&cmd).await?;

        // Parse status byte to determine state
        let state = parse_esp300_status(&status_response)?;

        Ok(AxisPosition {
            position,
            velocity: 0.0, // ESP300 doesn't report instantaneous velocity
            state,
            timestamp_ns: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        })
    }

    async fn configure_motion(&self, axis: u8, config: MotionConfig) -> Result<()> {
        // ESP300: "1VA10" = axis 1 velocity 10 mm/s
        let cmd = format!("{}VA{:.3}", axis + 1, config.velocity);
        self.adapter.send_command(&cmd).await?;

        // ESP300: "1AC5" = axis 1 acceleration 5 mm/s²
        let cmd = format!("{}AC{:.3}", axis + 1, config.acceleration);
        self.adapter.send_command(&cmd).await?;

        // Store config locally (soft limits not sent to hardware)
        self.axis_configs[axis as usize] = config;

        Ok(())
    }

    async fn start_position_stream(&self) -> Result<tokio::sync::mpsc::Receiver<MotionEvent>> {
        let (tx, rx) = tokio::sync::mpsc::channel::<MotionEvent>(100);

        let adapter = self.adapter.clone();
        let num_axes = self.num_axes;

        tokio::spawn(async move {
            loop {
                for axis in 0..num_axes {
                    // Read position for each axis
                    if let Ok(position) = read_axis_state_impl(&adapter, axis).await {
                        let event = MotionEvent { axis, position };
                        let _ = tx.send(event).await;
                    }
                }

                // Poll at 10 Hz
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        });

        Ok(rx)
    }

    async fn stop_position_stream(&self) -> Result<()> {
        // Drop the sender to close the stream
        Ok(())
    }

    fn num_axes(&self) -> u8 {
        self.num_axes
    }

    async fn get_motion_config(&self, axis: u8) -> Result<MotionConfig> {
        if axis >= self.num_axes {
            anyhow::bail!("Axis {} out of range", axis);
        }

        Ok(self.axis_configs[axis as usize].clone())
    }
}
```

---

### ScpiEndpoint Trait

**File:** `src/traits/scpi_endpoint.rs`

#### Basic Implementation (Standard SCPI)

```rust
use crate::traits::{ScpiEndpoint, ScpiEvent};
use crate::hardware::VisaAdapterV4;
use anyhow::{Result, Context};
use async_trait::async_trait;
use std::time::Duration;

pub struct OscilloscopeActor {
    adapter: VisaAdapterV4,
    timeout: Duration,
}

#[async_trait]
impl ScpiEndpoint for OscilloscopeActor {
    async fn query(&self, cmd: &str) -> Result<String> {
        self.adapter
            .query(cmd)
            .await
            .with_context(|| format!("SCPI query failed: {}", cmd))
    }

    async fn query_with_timeout(&self, cmd: &str, timeout: Duration) -> Result<String> {
        self.adapter
            .query_with_timeout(cmd, timeout)
            .await
            .with_context(|| format!("SCPI query with timeout failed: {}", cmd))
    }

    async fn write(&self, cmd: &str) -> Result<()> {
        self.adapter
            .write(cmd)
            .await
            .with_context(|| format!("SCPI write failed: {}", cmd))
    }

    async fn transact(&self, cmd: &str, timeout: Duration) -> Result<()> {
        // Send command
        self.write(cmd).await?;

        // Poll *STB? (Service Request / Status Byte) until ready
        let start = tokio::time::Instant::now();

        loop {
            let status = self.query("*STB?").await?;
            let status_byte: u8 = status.trim().parse()?;

            // Bit 5 = Event Status Bit (ESB) - operation complete
            if status_byte & 0x20 != 0 {
                return Ok(());
            }

            // Bit 4 = Message Available (MAV) - error occurred
            if status_byte & 0x10 != 0 {
                anyhow::bail!("SCPI error during transaction");
            }

            if start.elapsed() > timeout {
                anyhow::bail!("SCPI transaction timeout");
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    async fn read_error(&self) -> Result<u8> {
        let response = self.query("*ESR?").await?;
        response
            .trim()
            .parse()
            .with_context(|| format!("Failed to parse error register: {}", response))
    }

    async fn clear_errors(&self) -> Result<()> {
        self.write("*CLS").await
    }

    async fn reset(&self) -> Result<()> {
        self.write("*RST").await
    }

    async fn identify(&self) -> Result<String> {
        self.query("*IDN?").await
    }

    fn get_timeout(&self) -> Duration {
        self.timeout
    }

    fn set_timeout(&self, timeout: Duration) {
        self.timeout = timeout;
    }
}
```

#### SET-Only Instrument Pattern

```rust
/// Laser controller that only accepts SET commands (no GET queries)
pub struct MaiTaiActor {
    adapter: SerialAdapterV4,
    wavelength_nm: f64,  // Cached state
    shutter_open: bool,  // Cached state
}

#[async_trait]
impl TunableLaser for MaiTaiActor {
    async fn set_wavelength(&self, wavelength: Wavelength) -> Result<()> {
        // Send SET command
        let cmd = format!("WAVELENGTH:{:.1}", wavelength.nm);
        self.adapter.send_command(&cmd).await?;

        // Update cached state
        self.wavelength_nm = wavelength.nm;

        Ok(())
    }

    async fn get_wavelength(&self) -> Result<Wavelength> {
        // Try hardware query first
        match self.adapter.send_command("WAVELENGTH?").await {
            Ok(response) => {
                // Parse and update cache
                let nm = response.trim().parse()?;
                self.wavelength_nm = nm;
                Ok(Wavelength { nm })
            }
            Err(_) => {
                // Fallback to cached value for SET-only instruments
                tracing::warn!("Hardware query failed, using cached wavelength");
                Ok(Wavelength {
                    nm: self.wavelength_nm,
                })
            }
        }
    }
}
```

---

## Hardware Adapters

### VisaAdapterV4

**File:** `src/hardware/visa_adapter_v4.rs`

#### Builder Pattern Usage

```rust
use v4_daq::hardware::VisaAdapterV4Builder;
use std::time::Duration;

async fn create_visa_instrument() -> Result<VisaAdapterV4> {
    let adapter = VisaAdapterV4Builder::new("TCPIP0::192.168.1.100::INSTR".to_string())
        .with_timeout(Duration::from_secs(2))
        .with_read_terminator("\n".to_string())
        .with_write_terminator("\n".to_string())
        .build()
        .await?;

    // Verify connection
    let idn = adapter.query("*IDN?").await?;
    println!("Connected to: {}", idn);

    Ok(adapter)
}
```

#### VISA Resource Strings

```rust
// TCP/IP LAN instrument
"TCPIP0::192.168.1.100::INSTR"

// TCP/IP with port
"TCPIP0::192.168.1.100::5025::SOCKET"

// USB instrument
"USB0::0x1AB1::0x0588::DS1234567890::INSTR"

// GPIB instrument (address 10)
"GPIB0::10::INSTR"
```

---

### SerialAdapterV4

**File:** `src/hardware/serial_adapter_v4.rs`

#### Builder Pattern Usage

```rust
use v4_daq::hardware::SerialAdapterV4Builder;
use std::time::Duration;

async fn create_serial_instrument() -> Result<SerialAdapterV4> {
    let adapter = SerialAdapterV4Builder::new("/dev/ttyUSB0".to_string(), 9600)
        .with_timeout(Duration::from_millis(500))
        .with_line_terminator("\r".to_string())
        .with_response_delimiter('\r')
        .build();

    adapter.connect().await?;

    Ok(adapter)
}
```

#### Echo Stripping (MaiTai Pattern)

```rust
async fn query_with_echo() -> Result<f64> {
    let adapter = SerialAdapterV4::new("/dev/ttyUSB0".to_string(), 9600);

    // MaiTai returns "WAVELENGTH:800" for "WAVELENGTH?" query
    let value_str = adapter
        .query_with_echo_strip("WAVELENGTH?", ':')
        .await?;

    // value_str = "800"
    let wavelength: f64 = value_str.parse()?;

    Ok(wavelength)
}
```

---

## Error Handling Patterns

### Result-Based Error Propagation

All trait methods return `Result<T>` with context:

```rust
async fn set_wavelength(&self, wavelength: Wavelength) -> Result<()> {
    if wavelength.nm < 680.0 || wavelength.nm > 1080.0 {
        anyhow::bail!("Wavelength {:.1}nm out of range [680.0, 1080.0]", wavelength.nm);
    }

    let cmd = format!("WAVELENGTH:{:.1}", wavelength.nm);
    self.adapter
        .send_command(&cmd)
        .await
        .with_context(|| format!("Failed to set wavelength to {:.1}nm", wavelength.nm))?;

    Ok(())
}
```

### Actor Error Handling

```rust
impl kameo::Actor for MyInstrument {
    type Error = BoxSendError;

    async fn on_start(
        args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        // Hardware initialization with error context
        if let Some(ref adapter) = args.adapter {
            adapter
                .connect()
                .await
                .with_context(|| "Failed to connect during actor start")?;
        }

        Ok(args)
    }
}
```

---

## Arrow Conversion Examples

All traits provide default `to_arrow_*()` implementations using `Lazy<Arc<Schema>>`.

### Camera Frames to Arrow

```rust
use arrow::record_batch::RecordBatch;

let frames: Vec<Frame> = capture_frames().await?;

// Default Arrow conversion
let batch: RecordBatch = camera.to_arrow_frames(&frames)?;

// Schema: timestamp (i64 ns), frame_number (u64), pixel_format (utf8),
//         width (u32), height (u32), roi_x, roi_y, roi_width, roi_height,
//         pixel_data (binary)
```

### Motion Events to Arrow

```rust
let events: Vec<MotionEvent> = collect_position_data().await?;

let batch: RecordBatch = controller.to_arrow_positions(&events)?;

// Schema: timestamp (i64 ns), axis (u8), position (f64),
//         velocity (f64), state (utf8: "Idle", "Moving", etc.)
```

### SCPI Events to Arrow

```rust
let events: Vec<ScpiEvent> = log_scpi_commands().await?;

let batch: RecordBatch = endpoint.to_arrow_events(&events)?;

// Schema: timestamp (i64 ns), command (utf8), response (utf8, nullable),
//         success (bool), error (utf8, nullable)
```

---

## Testing Strategies

### Mock Adapters

```rust
pub struct MockSerialAdapter {
    responses: HashMap<String, String>,
}

impl MockSerialAdapter {
    pub fn new() -> Self {
        let mut responses = HashMap::new();
        responses.insert("*IDN?".to_string(), "MockInstrument,v1.0".to_string());
        responses.insert("WAVELENGTH?".to_string(), "800.0".to_string());

        Self { responses }
    }

    pub async fn send_command(&self, cmd: &str) -> Result<String> {
        self.responses
            .get(cmd)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("No mock response for: {}", cmd))
    }
}
```

### Actor Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_wavelength_set() {
        let actor = MyLaser {
            adapter: None,  // Mock mode
            wavelength_nm: 800.0,
            shutter_open: false,
        };

        let wavelength = Wavelength { nm: 850.0 };
        actor.set_wavelength(wavelength).await.unwrap();

        let current = actor.get_wavelength().await.unwrap();
        assert_eq!(current.nm, 850.0);
    }
}
```

---

## Best Practices

### 1. Always Use Builder Pattern for Adapters

```rust
// ✅ Good: Builder with explicit configuration
let adapter = SerialAdapterV4Builder::new(port, baud)
    .with_timeout(Duration::from_secs(2))
    .build();

// ❌ Bad: Raw constructor with defaults
let adapter = SerialAdapterV4::new(port, baud);
```

### 2. Validate Parameters Before Hardware Calls

```rust
// ✅ Good: Validate first
if position < config.min_position {
    anyhow::bail!("Position out of bounds");
}
self.adapter.send_command(&cmd).await?;

// ❌ Bad: Send command, then check response
self.adapter.send_command(&cmd).await?;
if !response.contains("OK") {
    anyhow::bail!("Command failed");
}
```

### 3. Use `with_context()` for Error Messages

```rust
// ✅ Good: Contextual errors
self.adapter
    .query(cmd)
    .await
    .with_context(|| format!("Failed to query {}", cmd))?;

// ❌ Bad: Generic errors
self.adapter.query(cmd).await?;
```

### 4. Clone Adapters for Async Tasks

```rust
// ✅ Good: Clone Arc-wrapped adapter
let adapter = self.adapter.clone();
tokio::spawn(async move {
    adapter.send_command("INIT").await
});

// ❌ Bad: Move original adapter
tokio::spawn(async move {
    self.adapter.send_command("INIT").await  // Compiler error!
});
```

---

## Common Patterns

### Timeout Configuration

```rust
// Short timeout for fast queries
let idn = adapter
    .query_with_timeout("*IDN?", Duration::from_millis(500))
    .await?;

// Long timeout for slow operations
let result = adapter
    .query_with_timeout("CALC:ALL", Duration::from_secs(10))
    .await?;
```

### Retry Logic

```rust
async fn query_with_retry(adapter: &VisaAdapterV4, cmd: &str, retries: u32) -> Result<String> {
    for attempt in 0..retries {
        match adapter.query(cmd).await {
            Ok(response) => return Ok(response),
            Err(e) if attempt < retries - 1 => {
                tracing::warn!("Query failed (attempt {}), retrying: {}", attempt + 1, e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => return Err(e),
        }
    }

    unreachable!()
}
```

### State Caching for SET-Only Instruments

```rust
pub struct SetOnlyInstrument {
    adapter: SerialAdapterV4,
    cached_state: Arc<Mutex<HashMap<String, String>>>,
}

impl SetOnlyInstrument {
    async fn set_parameter(&self, key: &str, value: &str) -> Result<()> {
        // Send SET command
        let cmd = format!("{}:{}", key, value);
        self.adapter.send_command(&cmd).await?;

        // Update cache
        let mut cache = self.cached_state.lock().await;
        cache.insert(key.to_string(), value.to_string());

        Ok(())
    }

    async fn get_parameter(&self, key: &str) -> Result<String> {
        // Try hardware query first
        let query_cmd = format!("{}?", key);
        match self.adapter.send_command(&query_cmd).await {
            Ok(response) => Ok(response),
            Err(_) => {
                // Fallback to cache for SET-only instruments
                let cache = self.cached_state.lock().await;
                cache
                    .get(key)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("No cached value for {}", key))
            }
        }
    }
}
```

---

## See Also

- [Phase 1D Trait Updates](PHASE_1D_TRAIT_UPDATES.md) - Design decisions and hardware learnings
- [V4 Architecture Overview](ARCHITECTURE.md) - System architecture and design philosophy
- [Kameo Actor Guide](https://docs.rs/kameo/latest/kameo/) - Actor system documentation

---

**Document Version:** 1.0
**Last Updated:** 2025-11-17
**Status:** Phase 1D Reference Documentation
