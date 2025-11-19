# Phase 1D Meta-Instrument Traits - Design RFC

**Status:** DRAFT (awaiting hardware validation feedback)
**Date:** 2025-11-16
**Phase:** Phase 1D (Diverse Instrument Migration)
**Milestone:** V4 Architecture Validation

---

## 1. Overview

Phase 1D extends the V4 architecture to validate its generalization across diverse instrument types. While Phase 1A-1C focused on single-instrument validation (Newport 1830C power meter), Phase 1D introduces three new instruments with fundamentally different characteristics:

- **PVCAM Camera (Photometrics PrimeBSI)** - High-bandwidth image data
- **Newport ESP300 Motion Controller** - Multi-axis coordinated movement
- **Generic SCPI Endpoint** - Query/response protocol with diverse device types

This RFC defines three meta-instrument traits that abstract over these instruments, enabling polymorphic control via `InstrumentManager` and pluggable adapters. The trait designs draw on:

1. **PowerMeter trait** - Reference implementation (synchronous measurement, single response)
2. **DynExp pattern** - Hardware-agnostic async interfaces with Arrow serialization
3. **V2 implementations** - Protocol details from existing instruments

## 2. Design Principles

### 2.1 Hardware-Agnostic Interfaces

Traits define **WHAT** an instrument does, not **HOW** it does it. Protocol details (VISA, serial, USB) are isolated in adapters.

- Trait methods are independent of underlying hardware (e.g., `start_stream()` works for both PVCAM-USB and future FLIR-GigE cameras)
- Adapters implement protocol-specific logic while maintaining trait compatibility
- This enables drop-in replacement of adapters without changing instrument actors

### 2.2 Async-First Operations

All potentially-blocking I/O is async to support actor model concurrency.

```rust
async fn start_stream(&self, config: CameraStreamConfig) -> Result<()>;
// NOT: fn start_stream(...) -> Result<()>
```

Rationale: Actor model requires async message handling; blocking calls would stall supervision.

### 2.3 Result-Based Error Handling

All fallible operations return `Result<T>` with structured error context via `anyhow::anyhow!()`.

```rust
async fn query(&self, cmd: &str) -> Result<String>;  // ✅
async fn query(&self, cmd: &str) -> String;          // ❌ Silently fails
```

### 2.4 Arrow-Compatible Serialization

All measurement data must be serializable to Apache Arrow for:
- Zero-copy data transfer between actors
- Storage in HDF5 via pyarrow
- Analysis with Polars
- GUI visualization

Each trait includes a `to_arrow()` method that converts measurements to RecordBatch with standardized schema.

### 2.5 Send + Sync Bounds

Traits are `Send + Sync` to enable concurrent actor usage and work-stealing schedulers.

```rust
#[async_trait::async_trait]
pub trait CameraSensor: Send + Sync { ... }
```

### 2.6 Minimal API Surface

Traits contain only essential methods. Domain-specific extensions (e.g., advanced gain modes) are handled via:
- Configuration structs passed to initialization
- Adapter-specific capabilities queried at runtime
- Future enhancement trait objects for advanced features

---

## 3. CameraSensor Trait

### 3.1 Use Cases

**Primary:** PVCAM camera (Photometrics PrimeBSI, high-speed 16-bit monochrome)

**Future:** FLIR cameras (GigE Vision), Basler (GenICam), Princeton Instruments (USB)

**Common Requirements:**
- Frame acquisition (single + streaming)
- Region of interest (ROI) configuration
- Exposure control
- Gain/binning settings
- Timestamp and frame numbering for synchronization

### 3.2 Trait Definition (Draft)

```rust
use anyhow::Result;
use arrow::record_batch::RecordBatch;
use std::time::Duration;
use async_trait::async_trait;

/// Pixel format enumeration (extensible)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Mono8,       // 8-bit monochrome
    Mono12,      // 12-bit monochrome (packed)
    Mono16,      // 16-bit monochrome
    Bayer8,      // Bayer RGB (8-bit)
    Bayer16,     // Bayer RGB (16-bit)
}

/// Region of Interest configuration
#[derive(Debug, Clone)]
pub struct RegionOfInterest {
    /// Top-left X coordinate (pixel)
    pub x: u32,
    /// Top-left Y coordinate (pixel)
    pub y: u32,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

impl RegionOfInterest {
    pub fn full_sensor(width: u32, height: u32) -> Self {
        Self { x: 0, y: 0, width, height }
    }
}

/// Binning configuration (hardware-dependent)
#[derive(Debug, Clone, Copy)]
pub struct BinningConfig {
    /// Binning factor in X direction (1 = no binning, 2 = 2x2, etc.)
    pub x_bin: u8,
    /// Binning factor in Y direction
    pub y_bin: u8,
}

/// Trigger mode for frame acquisition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    /// Internal trigger (free-running at configured frame rate)
    Internal,
    /// External trigger on digital input
    External,
}

/// Camera timing configuration
#[derive(Debug, Clone)]
pub struct CameraTiming {
    /// Exposure time in microseconds
    pub exposure_us: u32,
    /// Frame period (for streaming); inverse = frame rate (Hz)
    pub frame_period_ms: f64,
    /// Trigger mode
    pub trigger_mode: TriggerMode,
}

/// Single frame with pixel data and metadata
#[derive(Debug, Clone)]
pub struct Frame {
    /// Frame timestamp (nanoseconds since acquisition start)
    pub timestamp_ns: i64,
    /// Frame counter (for detecting dropped frames)
    pub frame_number: u64,
    /// Pixel format
    pub pixel_format: PixelFormat,
    /// Width in pixels (after binning)
    pub width: u32,
    /// Height in pixels (after binning)
    pub height: u32,
    /// ROI applied to this frame
    pub roi: RegionOfInterest,
    /// Pixel data (format determined by pixel_format)
    /// For Mono16: Vec<u16> with length = width * height
    /// For Mono8: Vec<u8> with length = width * height
    pub pixel_data: Vec<u8>,
}

/// Streaming configuration
#[derive(Debug, Clone)]
pub struct CameraStreamConfig {
    /// ROI for streaming frames
    pub roi: RegionOfInterest,
    /// Binning configuration
    pub binning: BinningConfig,
    /// Timing settings
    pub timing: CameraTiming,
    /// Gain setting (0-100, hardware-dependent)
    pub gain: u8,
}

/// Camera sensor meta-instrument trait
///
/// Hardware-agnostic interface for camera control and data acquisition.
/// Implementations handle protocol-specific details (PVCAM SDK, GigE Vision, etc.).
#[async_trait::async_trait]
pub trait CameraSensor: Send + Sync {
    /// Start continuous frame acquisition
    ///
    /// # Arguments
    /// * `config` - Stream configuration (ROI, timing, gain, etc.)
    ///
    /// # Behavior
    /// - Configures camera according to `config`
    /// - Starts internal frame acquisition thread/task
    /// - Frames are queued internally and retrieved via `get_frame()` or polling
    /// - Returns immediately; actual acquisition happens asynchronously
    ///
    /// # Errors
    /// - Hardware not connected
    /// - Invalid configuration (ROI out of bounds, exposure exceeds max, etc.)
    /// - Frame rate exceeds hardware capability
    async fn start_stream(&self, config: CameraStreamConfig) -> Result<()>;

    /// Stop streaming acquisition
    ///
    /// # Behavior
    /// - Stops frame acquisition task
    /// - Clears internal frame queue
    /// - Returns any buffered frames (optional)
    ///
    /// # Errors
    /// - Hardware communication error
    async fn stop_stream(&self) -> Result<()>;

    /// Check if streaming is active
    fn is_streaming(&self) -> bool;

    /// Acquire single frame (snapshot)
    ///
    /// # Arguments
    /// * `config` - Timing and ROI for this frame
    ///
    /// # Behavior
    /// - Takes one frame with specified settings
    /// - Blocks until frame is available (with timeout)
    /// - Returns frame with full metadata
    ///
    /// # Errors
    /// - Hardware not connected
    /// - Timeout waiting for frame
    /// - Invalid configuration
    async fn snap_frame(&self, config: &CameraTiming) -> Result<Frame>;

    /// Configure Region of Interest
    ///
    /// # Arguments
    /// * `roi` - ROI definition
    ///
    /// # Behavior
    /// - Updates ROI for future frames
    /// - If streaming, applies to next queued frame
    ///
    /// # Errors
    /// - ROI out of sensor bounds
    /// - Invalid ROI dimensions
    async fn configure_roi(&self, roi: RegionOfInterest) -> Result<()>;

    /// Configure timing parameters
    ///
    /// # Arguments
    /// * `timing` - Exposure, frame period, trigger mode
    ///
    /// # Errors
    /// - Exposure exceeds hardware maximum
    /// - Frame period too short (violates exposure + readout time)
    async fn set_timing(&self, timing: CameraTiming) -> Result<()>;

    /// Set sensor gain
    ///
    /// # Arguments
    /// * `gain` - Gain in 0-100 range (hardware interprets as appropriate scale)
    ///
    /// # Errors
    /// - Gain out of valid range
    async fn set_gain(&self, gain: u8) -> Result<()>;

    /// Configure binning
    ///
    /// # Arguments
    /// * `binning` - X and Y binning factors
    ///
    /// # Errors
    /// - Binning not supported by hardware
    /// - Binning factor exceeds hardware maximum
    async fn set_binning(&self, binning: BinningConfig) -> Result<()>;

    /// Get current camera capabilities and limits
    fn get_capabilities(&self) -> CameraCapabilities;

    /// Serialize frames to Arrow RecordBatch for storage/transfer
    ///
    /// # Arguments
    /// * `frames` - Slice of frames to serialize
    ///
    /// # Output Schema
    /// ```text
    /// timestamp (i64 ns)
    /// frame_number (u64)
    /// pixel_format (utf8)
    /// width (u32)
    /// height (u32)
    /// roi_x (u32)
    /// roi_y (u32)
    /// roi_width (u32)
    /// roi_height (u32)
    /// pixel_data (binary, raw bytes for each frame)
    /// ```
    ///
    /// # Note
    /// Pixel data stored as binary blob; deserialization requires format context.
    fn to_arrow_frames(&self, frames: &[Frame]) -> Result<RecordBatch>;
}

/// Camera capabilities and hardware limits
#[derive(Debug, Clone)]
pub struct CameraCapabilities {
    pub sensor_width: u32,
    pub sensor_height: u32,
    pub pixel_formats: Vec<PixelFormat>,
    pub max_binning_x: u8,
    pub max_binning_y: u8,
    pub min_exposure_us: u32,
    pub max_exposure_us: u32,
    pub max_frame_rate_hz: f64,
}
```

### 3.3 Supporting Types

See trait definition above for:
- `RegionOfInterest` - ROI specification
- `CameraTiming` - Exposure and frame rate
- `CameraStreamConfig` - Full streaming configuration
- `Frame` - Individual frame with pixel data
- `BinningConfig` - Sensor binning
- `PixelFormat` - Pixel data format
- `TriggerMode` - Acquisition trigger type
- `CameraCapabilities` - Hardware capabilities

### 3.4 Arrow Schema

Frames serialized to Arrow with following schema:

```
field "timestamp" (i64, Timestamp(Nanosecond)): Frame acquisition timestamp
field "frame_number" (u64): Sequential frame counter
field "pixel_format" (utf8): Format name ("Mono16", "Mono8", etc.)
field "width" (u32): Effective width after binning
field "height" (u32): Effective height after binning
field "roi_x" (u32): ROI origin X
field "roi_y" (u32): ROI origin Y
field "roi_width" (u32): ROI width
field "roi_height" (u32): ROI height
field "pixel_data" (binary): Raw pixel bytes (order: row-major, left-to-right)
```

**Storage Strategy for Images:**
- Small frames (<1MB): Embed binary data directly in RecordBatch
- Large frames (>1MB): Store binary as HDF5 external dataset, reference in Arrow metadata

### 3.5 Integration with DataPublisher

**Problem:** Frames can be 100+ MB/s on high-speed cameras; cannot broadcast full pixel data.

**Solution:** Two-tier publishing

1. **Frame Metadata Channel** (lightweight, always published)
   - Arrow RecordBatch with timestamp, frame_number, ROI, but NO pixel data
   - Subscribers (GUI, analysis) can react to metadata

2. **Pixel Data Channel** (optional, heavy, on-demand)
   - Full Frame with pixel data via separate async handler
   - Only subscribers explicitly requesting pixels receive them
   - Rate-limited via backpressure

**Implementation:**
```rust
// InstrumentManager routes based on subscriber type
pub enum CameraDataRequest {
    Metadata,           // Only timestamp + frame_number
    MetadataAndPixels,  // Full Frame
}

// CameraSensor actor accepts both:
pub async fn subscribe_metadata(&self) -> Receiver<ArrowRecordBatch> { ... }
pub async fn subscribe_pixels(&self) -> Receiver<Frame> { ... }
```

### 3.6 Open Questions

1. **Frame Queueing Strategy**
   - Should CameraSensor buffer frames internally or push directly to subscriber?
   - What's maximum queue depth before oldest frames are dropped?
   - How to signal dropped frames to subscribers?

2. **Pixel Data Memory Management**
   - Allocate new Vec<u8> for each frame (safe but allocator-intensive)?
   - Use memory pool for frame buffers (faster but requires pooling logic)?
   - Support zero-copy access to driver buffers (risky, requires pinning)?

3. **Binning vs. Software Crop**
   - Binning reduces data rate (hardware-assisted)
   - Software crop on full-res frames allows post-hoc analysis
   - Should trait expose both or require hardware binning?

4. **Bayer Filter Demosaicing**
   - Should CameraSensor expose demosaiced RGB frames?
   - Or just raw Bayer data and defer demosaicing to analysis?

5. **Synchronization with Motion**
   - How to timestamp frames relative to ESP300 motion?
   - Shared clock across USB devices or frame-number correlation?

---

## 4. MotionController Trait

### 4.1 Use Cases

**Primary:** Newport ESP300 3-axis controller (RS-232 serial, stepper motors)

**Future:**
- Elliptec linear stages (CAN bus)
- Galvanometric scanners (analog voltage)
- Rotary stages with homing

**Common Requirements:**
- Multi-axis absolute and relative movement
- Position and velocity feedback
- Homing and limit switch handling
- State machine (idle, moving, homing, error)

### 4.2 Trait Definition (Draft)

```rust
use anyhow::Result;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;

/// Axis state enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisState {
    /// Idle, not moving
    Idle,
    /// Currently moving to target
    Moving,
    /// In homing sequence
    Homing,
    /// Limit switch engaged
    LimitSwitch,
    /// Error state
    Error,
}

/// Axis position and state snapshot
#[derive(Debug, Clone, Copy)]
pub struct AxisPosition {
    /// Current position in motor units (e.g., mm, degrees)
    pub position: f64,
    /// Current velocity in units/second (may be 0 if idle)
    pub velocity: f64,
    /// Axis state
    pub state: AxisState,
    /// Timestamp of this measurement (ns)
    pub timestamp_ns: i64,
}

/// Motion configuration for an axis
#[derive(Debug, Clone)]
pub struct MotionConfig {
    /// Velocity in units/second
    pub velocity: f64,
    /// Acceleration in units/second²
    pub acceleration: f64,
    /// Deceleration in units/second²
    pub deceleration: f64,
    /// Minimum position (soft limit)
    pub min_position: f64,
    /// Maximum position (soft limit)
    pub max_position: f64,
}

/// Motion event for streaming position data
#[derive(Debug, Clone, Copy)]
pub struct MotionEvent {
    pub axis: u8,
    pub position: AxisPosition,
}

/// Motion controller meta-instrument trait
///
/// Hardware-agnostic interface for multi-axis motion control.
/// Implementations handle protocol-specific details (ESP300 serial, CAN, etc.).
#[async_trait::async_trait]
pub trait MotionController: Send + Sync {
    /// Move axis to absolute position
    ///
    /// # Arguments
    /// * `axis` - Axis number (0-indexed; 0=X, 1=Y, 2=Z for 3-axis)
    /// * `position` - Target position in motor units
    ///
    /// # Behavior
    /// - Commands motion with configured velocity/acceleration
    /// - Returns immediately (motion continues asynchronously)
    /// - Use `read_position()` or position streaming to monitor progress
    ///
    /// # Errors
    /// - Axis out of range
    /// - Position exceeds soft limits
    /// - Hardware communication error
    /// - Axis in error state
    async fn move_absolute(&self, axis: u8, position: f64) -> Result<()>;

    /// Move axis by relative delta
    ///
    /// # Arguments
    /// * `axis` - Axis number
    /// * `delta` - Position change (positive = forward)
    ///
    /// # Behavior
    /// - Same as move_absolute but with position += delta
    ///
    /// # Errors
    /// - Computed target exceeds limits
    /// - Axis out of range
    async fn move_relative(&self, axis: u8, delta: f64) -> Result<()>;

    /// Stop all motion on axis
    ///
    /// # Arguments
    /// * `axis` - Axis number (or None for all axes)
    ///
    /// # Behavior
    /// - Immediate deceleration to stop
    /// - Holds position
    ///
    /// # Errors
    /// - Hardware communication error
    async fn stop(&self, axis: Option<u8>) -> Result<()>;

    /// Home axis (find reference position)
    ///
    /// # Arguments
    /// * `axis` - Axis number
    ///
    /// # Behavior
    /// - Executes homing sequence (typically: move to limit, back off, set zero)
    /// - Blocks until homing complete
    /// - Sets axis position to 0
    ///
    /// # Errors
    /// - Limit switch not found
    /// - Hardware timeout
    /// - Axis already homing
    async fn home(&self, axis: u8) -> Result<()>;

    /// Read current position of axis
    ///
    /// # Arguments
    /// * `axis` - Axis number
    ///
    /// # Returns
    /// - Current position in motor units
    ///
    /// # Errors
    /// - Axis out of range
    /// - Hardware communication error
    async fn read_position(&self, axis: u8) -> Result<f64>;

    /// Read full state of axis
    ///
    /// # Arguments
    /// * `axis` - Axis number
    ///
    /// # Returns
    /// - AxisPosition with position, velocity, state, timestamp
    ///
    /// # Errors
    /// - Axis out of range
    /// - Hardware communication error
    async fn read_axis_state(&self, axis: u8) -> Result<AxisPosition>;

    /// Configure motion parameters for axis
    ///
    /// # Arguments
    /// * `axis` - Axis number
    /// * `config` - Velocity, acceleration, soft limits
    ///
    /// # Errors
    /// - Axis out of range
    /// - Invalid parameters (velocity=0, acceleration=negative, etc.)
    async fn configure_motion(&self, axis: u8, config: MotionConfig) -> Result<()>;

    /// Start streaming position data for monitoring
    ///
    /// # Returns
    /// - Async receiver for MotionEvent updates
    /// - Events include all axes, emitted at configured polling rate
    ///
    /// # Note
    /// - Only one stream active at a time
    /// - Calling again closes previous stream
    async fn start_position_stream(&self) -> Result<tokio::sync::mpsc::Receiver<MotionEvent>>;

    /// Stop position streaming
    ///
    /// # Behavior
    /// - Closes position stream
    /// - Does NOT stop motion
    async fn stop_position_stream(&self) -> Result<()>;

    /// Get number of axes on this controller
    fn num_axes(&self) -> u8;

    /// Get current configuration for axis
    async fn get_motion_config(&self, axis: u8) -> Result<MotionConfig>;

    /// Serialize motion events to Arrow RecordBatch
    ///
    /// # Arguments
    /// * `events` - Slice of motion events to serialize
    ///
    /// # Output Schema
    /// ```text
    /// timestamp (i64 ns, Timestamp(Nanosecond))
    /// axis (u8)
    /// position (f64)
    /// velocity (f64)
    /// state (utf8: "Idle", "Moving", "Homing", etc.)
    /// ```
    fn to_arrow_positions(&self, events: &[MotionEvent]) -> Result<RecordBatch>;
}
```

### 4.3 Supporting Types

See trait definition above for:
- `AxisState` - Axis state enumeration
- `AxisPosition` - Position and state snapshot
- `MotionConfig` - Velocity, acceleration, limits
- `MotionEvent` - Event for position streaming

### 4.4 Arrow Schema

Motion data serialized to Arrow with following schema:

```
field "timestamp" (i64, Timestamp(Nanosecond)): Event timestamp
field "axis" (u8): Axis number (0-indexed)
field "position" (f64): Current position
field "velocity" (f64): Current velocity
field "state" (utf8): State name ("Idle", "Moving", "Homing", "Error")
```

### 4.5 Integration with DataPublisher

**Problem:** ESP300 streams positions for 3 axes at 5 Hz = 15 updates/sec (manageable), but coordinated multi-axis moves require synchronized data.

**Solution:** Single position stream with all axes

```rust
// MotionEvent includes axis field, allowing single stream for multi-axis
pub struct MotionEvent {
    pub axis: u8,
    pub position: AxisPosition,
}

// Arrow schema naturally supports multi-axis in single RecordBatch:
// timestamp | axis | position | velocity | state
// 1000      | 0    | 10.5     | 2.1      | Moving
// 1000      | 1    | 20.3     | 1.8      | Moving
// 1000      | 2    | 5.0      | 0.0      | Idle
```

**Multi-Axis Coordination:**
- CameraActor triggers on frame acquisition
- Queries InstrumentManager for current ESP300 position
- Logs (timestamp_camera, position_x, position_y, position_z) for correlation

### 4.6 Open Questions

1. **Absolute vs. Relative Speed**
   - Currently both move_absolute() and move_relative() use same configured velocity
   - Should different speeds be allowed for different move types?
   - E.g., slow relative moves for fine adjustment, fast absolute moves for positioning?

2. **Multi-Axis Coordination**
   - Should trait include coordinated move operations?
   - E.g., `move_absolute_synchronized(&[(axis, pos), ...])` for simultaneous XYZ moves?
   - Or delegate to higher-level sequencer?

3. **Limit Switch Handling**
   - Should reaching a limit be an error or normal state?
   - How to distinguish between soft limits (software) and hard limits (hardware)?
   - Should trait expose limit switch status?

4. **Homing Behavior**
   - Some controllers support configurable homing sequences
   - Should home() be parameterized or use default sequence?
   - How long should home() timeout wait?

5. **Velocity Ramps**
   - Should acceleration/deceleration be per-move or global?
   - Support S-curves (smooth acceleration) or just linear ramps?

6. **Position Units**
   - Trait uses abstract "motor units" (mm, degrees, etc.)
   - Should metadata specify what units are being used?
   - Or should conversion happen at adapter layer?

---

## 5. ScpiEndpoint Trait

### 5.1 Use Cases

**Primary:** Generic SCPI instruments via VISA (oscilloscopes, signal generators, power supplies)

**Future:** Extended to multi-parameter instruments (e.g., lock-in amplifiers with frequency + amplitude + phase)

**Common Requirements:**
- Query/response command execution
- Timeout handling and error recovery
- Standard error queries (*ESR? *STB?)
- Command buffering and synchronization

### 5.2 Trait Definition (Draft)

```rust
use anyhow::Result;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use std::time::Duration;

/// SCPI command event (for streaming and logging)
#[derive(Debug, Clone)]
pub struct ScpiEvent {
    /// Timestamp of command execution (ns)
    pub timestamp_ns: i64,
    /// SCPI command sent
    pub command: String,
    /// Response from instrument (if any)
    pub response: Option<String>,
    /// Success or error
    pub success: bool,
    /// Error message (if failed)
    pub error: Option<String>,
}

/// SCPI endpoint meta-instrument trait
///
/// Hardware-agnostic interface for SCPI command execution.
/// Implementations handle protocol-specific details (VISA, serial, Ethernet, etc.).
#[async_trait::async_trait]
pub trait ScpiEndpoint: Send + Sync {
    /// Send command and read response
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string (e.g., "*IDN?", "MEAS:VOLT:DC?")
    ///
    /// # Behavior
    /// - Sends command and waits for response
    /// - Returns full response string
    /// - No timeout; uses implementation default
    ///
    /// # Errors
    /// - Hardware communication error
    /// - Timeout (implementation-dependent)
    /// - Malformed response
    async fn query(&self, cmd: &str) -> Result<String>;

    /// Send command with explicit timeout
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string
    /// * `timeout` - Maximum time to wait for response
    ///
    /// # Errors
    /// - Hardware communication error
    /// - Timeout exceeded
    /// - Malformed response
    async fn query_with_timeout(&self, cmd: &str, timeout: Duration) -> Result<String>;

    /// Send command without expecting response
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string (e.g., "*RST", "*CLS")
    ///
    /// # Behavior
    /// - Sends command and returns immediately
    /// - No response is read
    ///
    /// # Errors
    /// - Hardware communication error
    async fn write(&self, cmd: &str) -> Result<()>;

    /// Send command and verify execution with *STB? check
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string
    /// * `timeout` - Maximum time to wait
    ///
    /// # Behavior
    /// - Sends command
    /// - Polls *STB? (service request / status byte) until ready or timeout
    /// - Returns when instrument indicates completion
    ///
    /// # Errors
    /// - Hardware communication error
    /// - Timeout
    /// - Error bit set in status
    async fn transact(&self, cmd: &str, timeout: Duration) -> Result<()>;

    /// Query instrument error (ESR? - Event Status Register)
    ///
    /// # Returns
    /// - Error code from instrument
    /// - 0 = no error, non-zero = error condition
    ///
    /// # Errors
    /// - Hardware communication error
    async fn read_error(&self) -> Result<u8>;

    /// Clear error queue and event status register (*CLS)
    ///
    /// # Errors
    /// - Hardware communication error
    async fn clear_errors(&self) -> Result<()>;

    /// Reset instrument to factory defaults (*RST)
    ///
    /// # Errors
    /// - Hardware communication error
    async fn reset(&self) -> Result<()>;

    /// Query instrument identity (*IDN?)
    ///
    /// # Returns
    /// - Identity string (e.g., "Agilent Technologies,34401A,1234567,5.01")
    ///
    /// # Errors
    /// - Hardware communication error
    async fn identify(&self) -> Result<String>;

    /// Get current timeout setting
    fn get_timeout(&self) -> Duration;

    /// Set default timeout for queries
    ///
    /// # Arguments
    /// * `timeout` - New timeout duration
    fn set_timeout(&self, timeout: Duration);

    /// Serialize SCPI events to Arrow RecordBatch
    ///
    /// # Arguments
    /// * `events` - Slice of events to serialize
    ///
    /// # Output Schema
    /// ```text
    /// timestamp (i64 ns, Timestamp(Nanosecond))
    /// command (utf8)
    /// response (utf8, nullable)
    /// success (bool)
    /// error (utf8, nullable)
    /// ```
    fn to_arrow_events(&self, events: &[ScpiEvent]) -> Result<RecordBatch>;
}
```

### 5.3 Supporting Types

See trait definition above for:
- `ScpiEvent` - Command event for logging and streaming

### 5.4 Arrow Schema

SCPI events serialized to Arrow with following schema:

```
field "timestamp" (i64, Timestamp(Nanosecond)): Command execution time
field "command" (utf8): SCPI command sent
field "response" (utf8, nullable): Response from instrument
field "success" (bool): Whether command succeeded
field "error" (utf8, nullable): Error message if failed
```

### 5.5 Integration with DataPublisher

**Problem:** SCPI commands are request-response; not naturally streaming. But commands may be issued frequently (e.g., polling measurements).

**Solution:** Optional command logging stream

```rust
// InstrumentManager routes SCPI events:
pub struct ScpiInstrumentActor {
    // ...
    event_subscribers: Vec<Receiver<ScpiEvent>>,
}

// Subscribers can:
// - Log all commands for debugging
// - Detect error patterns
// - Trigger alerts on command failures
```

**Measurement Streaming:**
Different for each instrument type (covered in Phase 2 vertical slices):
- Oscilloscope: Stream waveform data (handled by camera-like trait for high-bandwidth)
- Power supply: Stream voltage/current (simple scalar via PowerMeter-like trait)
- Signal generator: Stream frequency/amplitude commands (via ScpiEndpoint)

### 5.6 Open Questions

1. **Compound Queries**
   - Some instruments support multiple responses per command
   - E.g., "MEAS:VOLT:AC?; MEAS:CURR:AC?" returns two values
   - Should trait support parsing these or defer to caller?

2. **Command Queuing**
   - Multiple subsystems might want to query simultaneously
   - Should ScpiEndpoint queue commands internally or require caller synchronization?
   - What's max queue depth before backpressure?

3. **Status Polling**
   - How frequently should *STB? be checked in transact()?
   - Should polling rate be configurable?
   - What if polling interval exceeds user's timeout?

4. **Error Recovery**
   - Should failed queries automatically clear error register?
   - Or require explicit `clear_errors()`?
   - What about recovery from communication timeouts?

5. **Binary Data Responses**
   - Some SCPI instruments return binary waveform data
   - Should trait support binary responses or only text?
   - Current design assumes UTF-8 strings

6. **Synchronized Multi-Instrument Queries**
   - In Phase 2, may want to query oscilloscope + power meter simultaneously
   - Should there be a trait for grouping queries?
   - Or handled at InstrumentManager level?

---

## 6. InstrumentManager Integration

### 6.1 Trait Registration

`InstrumentManager` maintains registry of traits by type:

```rust
pub struct InstrumentManager {
    // Existing registries
    instruments: HashMap<String, Arc<dyn Any>>,  // Actor handles

    // New: trait-specific registries
    power_meters: HashMap<String, Arc<dyn PowerMeter>>,
    cameras: HashMap<String, Arc<dyn CameraSensor>>,
    motion_controllers: HashMap<String, Arc<dyn MotionController>>,
    scpi_endpoints: HashMap<String, Arc<dyn ScpiEndpoint>>,
}
```

### 6.2 Routing Commands

Commands routed by trait type:

```rust
pub enum InstrumentCommand {
    // Existing power meter
    PowerMeter(String, PowerMeterCommand),

    // New in Phase 1D
    Camera(String, CameraCommand),
    Motion(String, MotionCommand),
    Scpi(String, ScpiCommand),
}

pub enum CameraCommand {
    StartStream(CameraStreamConfig),
    StopStream,
    SnapFrame(CameraTiming),
    SetRoi(RegionOfInterest),
    // ...
}
```

### 6.3 Adapter Interface

Each trait implemented by specific adapter:

```rust
pub struct PVCAMAdapter {
    // ... PVCAM SDK integration ...
}

#[async_trait::async_trait]
impl CameraSensor for PVCAMAdapter {
    async fn start_stream(&self, config: CameraStreamConfig) -> Result<()> {
        // PVCAM-specific implementation
    }
    // ...
}
```

---

## 7. Adapter Requirements

### 7.1 VisaAdapterV4 (for ScpiEndpoint)

**New methods:**
```rust
pub async fn query(&self, cmd: &str) -> Result<String>;
pub async fn query_with_timeout(&self, cmd: &str, timeout: Duration) -> Result<String>;
pub async fn write(&self, cmd: &str) -> Result<()>;
```

**Implementation:**
- Wraps `visa-rs` (NI-VISA bindings)
- Thread-safe VISA session management
- Timeout configuration

### 7.2 CameraAdapterV4 (for CameraSensor)

**Requirements:**
- Camera SDK abstraction (PVCAM, GigE Vision, etc.)
- Frame queue management
- Pixel data buffering

**Key methods:**
```rust
pub async fn configure_roi(&self, roi: RegionOfInterest) -> Result<()>;
pub async fn start_acquisition(&self, config: CameraStreamConfig) -> Result<()>;
pub fn dequeue_frame(&self) -> Option<Frame>;
```

### 7.3 SerialAdapterV4 (extended for MotionController)

**New methods:**
```rust
pub async fn query_position(&self, axis: u8) -> Result<f64>;
pub async fn command_move(&self, axis: u8, position: f64) -> Result<()>;
```

**Enhancement:**
- Protocol parsing for ESP300 responses (e.g., "1,10.5,2.1" -> AxisPosition)
- Timeout management for serial communication

---

## 8. Migration from V2

### 8.1 PVCAM V2 → CameraSensor

**V2 Implementation:** `src/instruments_v2/pvcam.rs`

**Mapping:**
- V2 `Measurement::ImageData` → V4 `Frame`
- V2 configuration structs → V4 `CameraStreamConfig`
- V2 broadcast channel → V4 trait-based subscriber pattern

**Changes:**
- Remove V2 `Instrument` trait impl; add `CameraSensor` impl
- Convert V2 async task to CameraAdapterV4 (trait impl)
- Update Arrow schema to match `to_arrow_frames()` spec

### 8.2 ESP300 V2 → MotionController

**V2 Implementation:** `src/instruments_v2/esp300.rs`

**Mapping:**
- V2 position polling → V4 `read_position()`
- V2 command execution → V4 `move_absolute()`, etc.
- V2 state tracking → V4 `AxisState` enum

**Changes:**
- Implement `MotionController` trait
- Adapt `MotionConfig` for velocity/acceleration settings
- Convert position events to Arrow schema

### 8.3 SCPI V2 → ScpiEndpoint

**V2 Implementation:** `src/instruments_v2/scpi.rs`

**Mapping:**
- V2 command/response → V4 `query()` / `write()`
- V2 error handling → V4 `read_error()` / `clear_errors()`

**Changes:**
- Implement `ScpiEndpoint` trait
- Add `to_arrow_events()` for command logging

---

## 9. Future Considerations

### 9.1 Phase 2 Instruments

**Expected traits:**
- `SpectrumAnalyzer` - Frequency response data
- `Waveform` - Oscilloscope waveform acquisition
- `MultiChannelADC` - Multichannel data streaming

**Extensibility:**
- Traits defined independently but follow same principles
- Share supporting types where applicable (e.g., `RegionOfInterest` for detector instruments)

### 9.2 Trait Composition

For instruments with multiple functions (e.g., optical spectrum analyzer that can also measure power):

```rust
pub struct OpticalSpectrumAnalyzer;

impl CameraSensor for OpticalSpectrumAnalyzer { ... }    // Raw detector images
impl SpectrumAnalyzer for OpticalSpectrumAnalyzer { ... }  // Processed spectra
impl ScpiEndpoint for OpticalSpectrumAnalyzer { ... }      // SCPI control
```

### 9.3 Advanced Capabilities Trait

For hardware-specific features not in base trait:

```rust
pub trait CameraSensorAdvanced: CameraSensor {
    async fn set_detector_temperature(&self, temp_c: f64) -> Result<()>;
    async fn get_detector_temperature(&self) -> Result<f64>;
    async fn enable_tdfi(&self, enabled: bool) -> Result<()>;  // Time-delay integration
}
```

---

## 10. Implementation Timeline

### Phase 1D Hardware Validation (Concurrent)

- **Week 1-2:** Design phase (this RFC)
- **Week 2-3:** Hardware validation begins; iterate on trait design based on findings
- **Week 3-4:** Implement adapters + trait methods
- **Week 4-5:** Integration testing

### Parallelization with Hardware Work

- Hardware team: Validation, protocol debugging
- Code team: Trait impl, adapter coding (can start without hardware)
- Integration point: Trait signatures finalized after week 1, implementation follows

---

## 11. Next Steps

### Immediate (Before Hardware Validation)

- [ ] **Review this RFC** - Codex and team feedback
- [ ] **Finalize trait signatures** - Lock in method names, error handling, async patterns
- [ ] **Define Arrow schemas** - Confirm serialization strategy
- [ ] **InstrumentManager routing** - Design command dispatch for new traits

### During Hardware Validation (Weeks 2-3)

- [ ] **Gather hardware details** - Protocol quirks, timing constraints, error conditions
- [ ] **Iterate on trait design** - Adjust based on hardware realities
- [ ] **Prototype adapters** - Begin implementation with real hardware feedback
- [ ] **Update RFC** - Document resolved open questions

### Post-Validation (Weeks 3-5)

- [ ] **Complete adapter implementation** - Full trait impl for PVCAM, ESP300, SCPI
- [ ] **Integration testing** - Verify InstrumentManager routing, Arrow serialization
- [ ] **Performance validation** - Confirm frame streaming, position updates don't bottleneck
- [ ] **Merge to main** - Phase 1D implementation complete

---

## 12. Appendix: Reference Materials

### V2 Instrument Implementations
- PVCAM: `/Users/briansquires/code/rust-daq/src/instruments_v2/pvcam.rs`
- ESP300: `/Users/briansquires/code/rust-daq/src/instruments_v2/esp300.rs`
- SCPI: `/Users/briansquires/code/rust-daq/src/instruments_v2/scpi.rs`

### PowerMeter Reference Trait
- Location: `/Users/briansquires/code/rust-daq/src/traits/power_meter.rs`
- Shows async trait pattern, Arrow serialization, minimal API

### V4 Architecture Documentation
- Overview: `/Users/briansquires/code/rust-daq/ARCHITECTURE.md`
- Tracing: `/Users/briansquires/code/rust-daq/docs/v4/TRACING_SYSTEM.md`
- Configuration: `/Users/briansquires/code/rust-daq/docs/v4/CONFIG_SYSTEM.md`

### Related Specifications
- DynExp Architecture: Design pattern for hardware-agnostic instrument traits
- Apache Arrow: Columnar data format for efficient serialization
- VISA Standard: Instrument control protocol (for ScpiEndpoint implementations)
- SCPI Standard: Standard Commands for Programmable Instruments

---

**Document Version:** 0.1 (Draft)
**Last Updated:** 2025-11-16
**Status:** Awaiting team review and hardware validation feedback
