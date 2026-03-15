//! Atomic Hardware Capabilities
//!
//! This module defines fine-grained capability traits that hardware devices can implement.
//! Instead of monolithic traits like `Camera` or `Instrument`, devices implement
//! specific capabilities they actually support:
//!
//! - A camera might implement: `Triggerable + ExposureControl + FrameProducer`
//! - A stage might implement: `Movable + Triggerable`
//! - A power meter might implement: `Readable`
//!
//! This approach enables:
//! - Better composition (devices can mix capabilities)
//! - Clearer contracts (traits are small and focused)
//! - Easier testing (mock individual capabilities)
//! - Hardware-agnostic code (functions work with trait bounds)
//!
//! # Design Philosophy
//!
//! Each capability trait:
//! - Is async (uses #[async_trait])
//! - Is thread-safe (requires Send + Sync)
//! - Uses anyhow::Result for errors
//! - Focuses on ONE thing
//!
//! # Example
//!
//! ```rust,ignore
//! // A triggered camera implements multiple capabilities
//! struct SimulatedCamera {
//!     exposure_ms: f64,
//!     armed: bool,
//!     frame_count: u32,
//! }
//!
//! #[async_trait]
//! impl ExposureControl for SimulatedCamera {
//!     async fn set_exposure(&self, seconds: f64) -> Result<()> {
//!         self.exposure_ms = seconds * 1000.0;
//!         Ok(())
//!     }
//!
//!     async fn get_exposure(&self) -> Result<f64> {
//!         Ok(self.exposure_ms / 1000.0)
//!     }
//! }
//!
//! #[async_trait]
//! impl Triggerable for SimulatedCamera {
//!     async fn arm(&self) -> Result<()> {
//!         self.armed = true;
//!         Ok(())
//!     }
//!
//!     async fn trigger(&self) -> Result<()> {
//!         if !self.armed {
//!             anyhow::bail!("Camera not armed");
//!         }
//!         // Capture frame...
//!         Ok(())
//!     }
//! }
//!
//! #[async_trait]
//! impl FrameProducer for SimulatedCamera {
//!     async fn start_stream(&self) -> Result<()> { Ok(()) }
//!     async fn stop_stream(&self) -> Result<()> { Ok(()) }
//!     fn resolution(&self) -> (u32, u32) { (1024, 1024) }
//! }
//!
//! // Use in generic code
//! async fn triggered_acquisition<T>(device: &T) -> Result<()>
//! where
//!     T: Triggerable + ExposureControl + FrameProducer
//! {
//!     device.set_exposure(0.1).await?;
//!     device.arm().await?;
//!     device.trigger().await?;
//!     Ok(())
//! }
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use crate::observable::{ParameterMetadata, ParameterSet};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use crate::data::Frame;

// =============================================================================
// Device Category
// =============================================================================

/// Device category for classification and UI grouping
///
/// Used by the hardware registry and UI panels to categorize devices.
/// Drivers should explicitly set their category; the gRPC layer falls back
/// to string-based inference only if category is not set.
///
/// # Example
///
/// ```rust,ignore
/// let metadata = DeviceMetadata {
///     category: Some(DeviceCategory::Camera),
///     frame_width: Some(2048),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeviceCategory {
    /// Cameras and imaging devices (FrameProducer)
    Camera,
    /// Motion stages and actuators (Movable)
    Stage,
    /// Detectors and sensors (Readable)
    Detector,
    /// Lasers and light sources
    Laser,
    /// Power meters and energy sensors
    PowerMeter,
    /// Devices that don't fit other categories
    #[default]
    Other,
}

impl DeviceCategory {
    /// Human-readable label
    pub fn label(&self) -> &'static str {
        match self {
            Self::Camera => "Cameras",
            Self::Stage => "Stages",
            Self::Detector => "Detectors",
            Self::Laser => "Lasers",
            Self::PowerMeter => "Power Meters",
            Self::Other => "Other",
        }
    }
}

// =============================================================================
// Capability Traits
// =============================================================================

/// Capability: Motion Control
///
/// Devices that can move to positions (stages, actuators, goniometers).
///
/// # Contract
/// - Positions are in device-native units (typically mm or degrees)
/// - `move_abs` and `move_rel` initiate motion but may return before completion
/// - `wait_settled` blocks until motion completes
/// - `position` returns current position (may be approximate during motion)
///
/// # Thread Safety
/// - All methods are async and require `&self` (immutable reference)
/// - Interior mutability (Mutex/RwLock) should be used for state
#[async_trait]
pub trait Movable: Send + Sync {
    /// Move to absolute position
    ///
    /// # Arguments
    /// * `position` - Target position in device-native units
    ///
    /// # Returns
    /// - Ok(()) if motion initiated successfully
    /// - Err if position is out of range or hardware error
    async fn move_abs(&self, position: f64) -> Result<()>;

    /// Move relative to current position
    ///
    /// # Arguments
    /// * `distance` - Distance to move (positive or negative)
    ///
    /// # Returns
    /// - Ok(()) if motion initiated successfully
    /// - Err if resulting position would be out of range
    async fn move_rel(&self, distance: f64) -> Result<()>;

    /// Get current position
    ///
    /// # Returns
    /// Current position in device-native units.
    /// May be approximate if device is currently moving.
    async fn position(&self) -> Result<f64>;

    /// Wait for motion to settle
    ///
    /// Blocks until device reports motion is complete.
    /// Should have internal timeout to prevent infinite blocking.
    ///
    /// # Returns
    /// - Ok(()) when settled
    /// - Err on timeout or hardware error
    async fn wait_settled(&self) -> Result<()>;

    /// Stop motion immediately
    ///
    /// Issues an emergency stop command to halt motion in progress.
    /// Not all devices support this - check capability before calling.
    ///
    /// # Returns
    /// - Ok(()) if stop command issued successfully
    /// - Err if device doesn't support stop or hardware error
    ///
    /// # Default Implementation
    /// Returns an error indicating stop is not supported.
    async fn stop(&self) -> Result<()> {
        anyhow::bail!("Stop not supported by this device")
    }
}

/// Capability: External Triggering
///
/// Devices that can be armed and triggered (cameras, detectors, pulse generators).
///
/// # Contract
/// - `arm()` prepares device for trigger (may configure hardware buffers)
/// - `trigger()` initiates acquisition/output
/// - Some devices require arm before every trigger, others stay armed
/// - Calling `trigger()` on unarmed device should return Err
#[async_trait]
pub trait Triggerable: Send + Sync {
    /// Arm device for trigger
    ///
    /// Prepares hardware to respond to trigger signal.
    /// May configure buffers, clear counters, or enter standby mode.
    ///
    /// # Returns
    /// - Ok(()) if armed successfully
    /// - Err if device is busy or in error state
    async fn arm(&self) -> Result<()>;

    /// Send software trigger
    ///
    /// Initiates acquisition/output. Device must be armed first.
    ///
    /// # Returns
    /// - Ok(()) if trigger accepted
    /// - Err if not armed or hardware error
    async fn trigger(&self) -> Result<()>;

    /// Check if device is currently armed
    ///
    /// # Returns
    /// - Ok(true) if device is armed and ready for trigger
    /// - Ok(false) if device is not armed
    /// - Err if state cannot be determined or not supported
    ///
    /// # Default Implementation
    /// Returns an error indicating state query is not supported.
    async fn is_armed(&self) -> Result<bool> {
        anyhow::bail!("Armed state query not supported by this device")
    }
}

/// Capability: Exposure Time Control
///
/// Devices with configurable integration time (cameras, spectrometers, photodetectors).
///
/// # Contract
/// - Exposure is in seconds (not milliseconds)
/// - Setting exposure does not start acquisition
/// - Exposure applies to next acquisition
#[async_trait]
pub trait ExposureControl: Send + Sync {
    /// Set exposure/integration time
    ///
    /// # Arguments
    /// * `seconds` - Exposure time in seconds
    ///
    /// # Returns
    /// - Ok(()) if exposure set successfully
    /// - Err if value is out of hardware range
    async fn set_exposure(&self, seconds: f64) -> Result<()>;

    /// Get current exposure setting
    ///
    /// # Returns
    /// Current exposure time in seconds
    async fn get_exposure(&self) -> Result<f64>;
}

// ============================================================================
// Frame Observer Pattern (bd-0dax.4)
// ============================================================================

/// Handle returned when registering a frame observer, used for unregistration.
///
/// This is an opaque handle - the internal ID is implementation-specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObserverHandle(pub u64);

impl ObserverHandle {
    /// Create a new observer handle with the given ID.
    #[must_use]
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the internal ID (for debugging/logging).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.0
    }
}

/// Trait for synchronous frame observers (bd-0dax.4).
///
/// Frame observers receive a reference to each frame during acquisition,
/// allowing for non-blocking inspection before the frame is delivered to
/// primary consumers.
///
/// # Contract
///
/// - `on_frame()` MUST NOT block
/// - `on_frame()` MUST complete quickly (< 1ms recommended)
/// - To persist data, implementations MUST copy to their own buffer
/// - Implementations MUST handle backpressure internally (drop if slow)
///
/// # Safety
///
/// The frame reference is only valid for the duration of the `on_frame()` call.
/// Implementations must not store the reference or attempt to extend its lifetime.
///
/// # Deadlock Warning
///
/// **NEVER call `unregister_observer()` from within `on_frame()`!**
///
/// The frame loop holds a read lock while iterating over observers. Calling
/// `unregister_observer()` from within a callback will attempt to acquire a write
/// lock, causing a deadlock. If you need to unregister based on frame content:
///
/// 1. Set a flag in your observer during `on_frame()`
/// 2. Check the flag from another task/thread and call `unregister_observer()` there
///
/// # Example
///
/// ```rust,ignore
/// use common::capabilities::{FrameObserver, ObserverHandle};
/// use common::data::FrameView;
///
/// struct DecimatedObserver {
///     interval: u64,
///     count: AtomicU64,
///     tx: mpsc::Sender<Vec<u8>>,
/// }
///
/// impl FrameObserver for DecimatedObserver {
///     fn on_frame(&self, frame: &FrameView<'_>) {
///         let count = self.count.fetch_add(1, Ordering::Relaxed);
///         if count % self.interval == 0 {
///             // Copy pixel data for persistence (required - can't hold reference)
///             let pixels = frame.pixels().to_vec();
///             let _ = self.tx.try_send(pixels); // Non-blocking
///         }
///     }
///
///     fn name(&self) -> &str {
///         "decimated_observer"
///     }
/// }
/// ```
pub trait FrameObserver: Send + Sync {
    /// Called synchronously for each frame during acquisition.
    ///
    /// This method is called from the frame loop before the frame is
    /// delivered to primary consumers. It MUST NOT block and MUST complete
    /// quickly (ideally < 100µs, definitely < 1ms).
    ///
    /// # Arguments
    ///
    /// - `frame`: Zero-copy view into frame data (valid only for this call)
    ///
    /// # Performance Warning
    ///
    /// Implementations MUST return immediately. Do NOT perform I/O, heavy
    /// computation, or lock acquisition here. If persistence or complex
    /// processing is needed, push data to a channel for processing in a separate
    /// task.
    ///
    /// Blocking or slow observers will stall the entire hardware driver loop,
    /// potentially causing buffer overflows in the SDK and dropping frames for
    /// all consumers.
    fn on_frame(&self, frame: &crate::data::FrameView<'_>);

    /// Optional: Return a descriptive name for this observer (for debugging/logging).
    fn name(&self) -> &'static str {
        "unnamed_observer"
    }
}

/// Type alias for pooled frame data from the object pool.
///
/// This represents a frame buffer loaned from a pre-allocated pool,
/// enabling zero-allocation frame handling for high-FPS scenarios.
pub type LoanedFrame = pool::Loaned<pool::FrameData>;

/// Capability: Frame/Image Production
///
/// Devices that produce 2D image frames (cameras, beam profilers).
///
/// # Contract
/// - `start_stream()` begins continuous acquisition
/// - `stop_stream()` halts acquisition
/// - Frames are delivered via `register_primary_output()` (primary consumer) or `register_observer()` (secondary consumers)
/// - `resolution()` is immutable (cannot be changed via this trait)
///
/// # Frame Delivery
///
/// ## Recommended: `register_primary_output()` (zero-allocation, single primary consumer)
/// Call `register_primary_output()` BEFORE `start_stream()` to register a channel
/// that will receive `LoanedFrame` objects with ownership. The primary consumer
/// owns frames and controls when they return to the pre-allocated pool.
///
/// ## Secondary: `register_observer()` (zero-copy, multiple tap consumers)
/// Register frame observers that receive borrowed `FrameView<'_>` references for
/// non-blocking secondary access. Observers must NOT block and should copy data
/// if persistence is needed. Multiple observers can be registered concurrently.
///
/// ## Legacy: `subscribe_frames()` (deprecated - do not use)
/// Returns a broadcast receiver for `Arc<Frame>`. Deprecated in favor of
/// `register_primary_output()` which provides better performance through pooling.
#[async_trait]
pub trait FrameProducer: Send + Sync {
    /// Start continuous frame acquisition
    ///
    /// # Returns
    /// - Ok(()) if streaming started
    /// - Err if already streaming or hardware error
    async fn start_stream(&self) -> Result<()>;

    /// Start finite frame acquisition with a maximum frame count
    ///
    /// # Arguments
    /// - `frame_limit`: Maximum number of frames to acquire.
    ///   - `Some(n)` where n > 0: acquire exactly n frames then stop
    ///   - `Some(0)` or `None`: continuous acquisition (same as `start_stream()`)
    ///
    /// # Returns
    /// - Ok(()) if streaming started
    /// - Err if already streaming or hardware error
    ///
    /// # Default Implementation
    /// Calls `start_stream()` for continuous acquisition. Drivers that support
    /// finite acquisition should override this method.
    async fn start_stream_finite(&self, frame_limit: Option<u32>) -> Result<()> {
        match frame_limit {
            Some(n) if n > 0 => {
                tracing::warn!(
                    "Device does not support finite acquisition; starting continuous stream \
                     (requested {} frames)",
                    n
                );
                self.start_stream().await
            }
            _ => self.start_stream().await,
        }
    }

    /// Stop frame acquisition
    ///
    /// # Returns
    /// - Ok(()) if streaming stopped
    /// - Err on hardware error
    async fn stop_stream(&self) -> Result<()>;

    /// Get frame resolution (width, height)
    ///
    /// Returns sensor resolution in pixels.
    /// This is immutable - use separate ROI trait for cropping.
    fn resolution(&self) -> (u32, u32);

    /// Take the frame receiver for consuming streamed frames
    ///
    /// **DEPRECATED**: Use `register_primary_output()` instead for zero-allocation pooled frames.
    ///
    /// This can only be called once - subsequent calls return None.
    /// Call this BEFORE `start_stream()` to receive frames.
    ///
    /// # Returns
    /// - Some(receiver) if receiver is available
    /// - None if receiver was already taken or not supported by this device
    #[deprecated(
        since = "0.2.0",
        note = "Use register_primary_output() for zero-allocation pooled frame delivery. Sunset: v1.0"
    )]
    async fn take_frame_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<crate::data::Frame>> {
        // Default: no frame receiver support
        None
    }

    /// Subscribe to the frame stream
    ///
    /// **DEPRECATED**: Use `register_primary_output()` for zero-allocation pooled frames,
    /// or `register_observer()` for secondary frame access. This method will be removed
    /// in a future release.
    ///
    /// Returns a broadcast receiver that will receive `Arc<Frame>` for each captured frame.
    /// Multiple subscribers can receive the same frames but with heap allocation overhead.
    /// Can be called multiple times to create additional subscribers.
    ///
    /// # Returns
    /// - Some(receiver) if subscription succeeded
    /// - None if streaming is not supported by this device
    ///
    /// # Migration Guide
    ///
    /// **For primary consumers (owns frames):**
    /// ```rust,ignore
    /// // Old (deprecated): broadcast with Arc allocation
    /// let rx = camera.subscribe_frames().await?;
    /// while let Ok(frame) = rx.recv().await {
    ///     println!("Frame: {}x{}", frame.width, frame.height);
    /// }
    ///
    /// // New (recommended): pooled frames with zero allocation
    /// let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    /// camera.register_primary_output(tx).await?;
    /// camera.start_stream().await?;
    /// while let Some(frame) = rx.recv().await {
    ///     // LoanedFrame - from pre-allocated pool, auto-returned on drop
    ///     println!("Frame: {}x{}", frame.width, frame.height);
    /// }
    /// ```
    ///
    /// **For secondary consumers (observers only):**
    /// ```rust,ignore
    /// // Old (deprecated): multiple broadcast receivers with allocation
    /// let rx = camera.subscribe_frames().await?;
    ///
    /// // New (recommended): register observer for non-blocking tap
    /// struct MyObserver;
    /// impl FrameObserver for MyObserver {
    ///     fn on_frame(&self, frame: &FrameView<'_>) {
    ///         // Process frame without copying
    ///         println!("Tap: {}x{}", frame.width, frame.height);
    ///     }
    /// }
    /// let handle = camera.register_observer(Box::new(MyObserver)).await?;
    /// ```
    #[deprecated(
        since = "0.3.0",
        note = "Use register_primary_output() for primary consumers or register_observer() for secondary access. Sunset: v1.0"
    )]
    async fn subscribe_frames(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<std::sync::Arc<crate::data::Frame>>> {
        // Default: no broadcast support
        None
    }

    /// Get the number of active frame subscribers
    ///
    /// # Returns
    /// - Number of active receivers subscribed to the broadcast channel
    ///
    /// # Default Implementation
    /// Returns 0 (subscriber tracking not supported)
    fn receiver_count(&self) -> usize {
        0
    }

    /// Check if device is currently streaming frames
    ///
    /// # Returns
    /// - Ok(true) if actively streaming
    /// - Ok(false) if not streaming
    /// - Err if state cannot be determined or not supported
    ///
    /// # Default Implementation
    /// Returns an error indicating state query is not supported.
    async fn is_streaming(&self) -> Result<bool> {
        anyhow::bail!("Streaming state query not supported by this device")
    }

    /// Get the number of frames captured since streaming started
    ///
    /// # Returns
    /// - Count of frames captured during the current or last stream
    ///
    /// # Default Implementation
    /// Returns 0 (no frame count tracking)
    fn frame_count(&self) -> u64 {
        0
    }

    // ========================================================================
    // Primary Output Registration (bd-0dax.5)
    // ========================================================================

    /// Register the primary frame consumer.
    ///
    /// Only ONE primary consumer is allowed - it owns frames and controls pool reclamation.
    /// Call BEFORE `start_stream()`. Subsequent calls replace the previous consumer.
    ///
    /// This is the preferred method for high-performance frame delivery, as it uses
    /// pre-allocated pooled buffers (`LoanedFrame`) instead of heap-allocated `Arc<Frame>`.
    ///
    /// # Arguments
    /// * `tx` - Channel sender that will receive `LoanedFrame` ownership
    ///
    /// # Returns
    /// * `Ok(())` if registration succeeded
    /// * `Err` if device doesn't support pooled frames
    ///
    /// # Default Implementation
    /// Returns an error indicating pooled output is not supported.
    ///
    /// # Example
    /// ```rust,ignore
    /// let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    /// camera.register_primary_output(tx).await?;
    /// camera.start_stream().await?;
    ///
    /// while let Some(frame) = rx.recv().await {
    ///     // Process LoanedFrame - automatically returns to pool when dropped
    ///     println!("Frame: {}x{}", frame.width, frame.height);
    /// }
    /// ```
    async fn register_primary_output(
        &self,
        tx: tokio::sync::mpsc::Sender<LoanedFrame>,
    ) -> Result<()> {
        let _ = tx; // Suppress unused warning
        anyhow::bail!("Pooled frame output not supported by this device")
    }

    // ========================================================================
    // Frame Observer Methods (bd-0dax.4)
    // ========================================================================

    /// Register a tap for secondary frame access (observer pattern).
    ///
    /// Taps receive borrowed references to frames, NOT ownership.
    /// Multiple taps are allowed. Can be registered before or during streaming.
    /// Taps MUST NOT block - use try_send or bounded channels.
    ///
    /// # Arguments
    /// * `observer` - The observer implementing FrameObserver trait
    ///
    /// # Returns
    /// * Ok(handle) - Use handle to unregister tap later
    /// * Err if device doesn't support taps
    ///
    /// # Default Implementation
    /// Returns an error indicating taps are not supported.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let handle = device.register_observer(Box::new(my_observer)).await?;
    /// // ... streaming ...
    /// device.unregister_observer(handle).await?;
    /// ```
    async fn register_observer(&self, observer: Box<dyn FrameObserver>) -> Result<ObserverHandle> {
        let _ = observer;
        anyhow::bail!("Frame observers not supported by this device")
    }

    /// Unregister a previously registered frame observer.
    ///
    /// # Arguments
    /// * `handle` - Handle returned from register_observer
    ///
    /// # Returns
    /// * Ok(()) if unregistration succeeded
    /// * Err if handle is invalid or device doesn't support taps
    async fn unregister_observer(&self, handle: ObserverHandle) -> Result<()> {
        let _ = handle;
        anyhow::bail!("Frame observers not supported by this device")
    }

    /// Check if this device supports frame observers.
    ///
    /// # Returns
    ///
    /// - `true` if `register_observer()` will return a handle
    /// - `false` if observers are not supported
    ///
    /// # Default Implementation
    ///
    /// Returns `false`.
    fn supports_observers(&self) -> bool {
        false
    }

    /// Returns `true` if the last acquisition loop exited due to a hardware error.
    ///
    /// Used by the streaming layer to detect camera disconnects (USB/PCIe) and
    /// report failures to the device supervisor for automatic reconnection with
    /// exponential backoff.
    ///
    /// Drivers that can distinguish error-stops from user-stops should override
    /// this. The default returns `false` (no error tracking).
    fn has_acquisition_error(&self) -> bool {
        false
    }
}

/// Capability: Scalar Readout
///
/// Devices that produce single scalar values (power meters, temperature sensors,
/// voltmeters, pressure gauges).
///
/// # Contract
/// - `read()` performs measurement and returns value
/// - Units are device-specific (document in implementation)
/// - Reading should be fast (<100ms typical)
#[async_trait]
pub trait Readable: Send + Sync {
    /// Read current value
    ///
    /// Performs measurement and returns scalar value.
    /// Units depend on device type (watts, volts, celsius, etc.)
    ///
    /// # Returns
    /// - Ok(value) on successful read
    /// - Err on hardware error or timeout
    async fn read(&self) -> Result<f64>;
}

/// Capability: Wavelength Tuning
///
/// Devices with tunable wavelength output (lasers, monochromators, OPOs).
///
/// # Contract
/// - Wavelength is in nanometers (nm)
/// - `set_wavelength()` may block while tuning (device-specific)
/// - Implementation should validate wavelength is within device range
///
/// # Safety
/// CAUTION: Wavelength changes on high-power lasers may affect
/// beam alignment and optical safety equipment effectiveness.
#[async_trait]
pub trait WavelengthTunable: Send + Sync {
    /// Set output wavelength
    ///
    /// # Arguments
    /// * `wavelength_nm` - Target wavelength in nanometers
    ///
    /// # Returns
    /// - Ok(()) if wavelength set successfully
    /// - Err if value is out of hardware range or tuning failed
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()>;

    /// Get current wavelength setting
    ///
    /// # Returns
    /// Current wavelength in nanometers
    async fn get_wavelength(&self) -> Result<f64>;

    /// Get wavelength tuning range
    ///
    /// # Returns
    /// (min_nm, max_nm) tuple defining the valid wavelength range
    ///
    /// # Default Implementation
    /// Returns a typical NIR range. Override for specific devices.
    fn wavelength_range(&self) -> (f64, f64) {
        (700.0, 1000.0)
    }
}

/// Capability: Shutter Control
///
/// Devices with controllable beam shutter (lasers, light sources).
///
/// # Contract
/// - `open_shutter()` allows beam to pass
/// - `close_shutter()` blocks beam
/// - Shutter state should be queryable
///
/// # Safety
/// CAUTION: Always verify shutter state before assuming beam is blocked.
/// Use hardware interlocks for laser safety, never rely on software alone.
#[async_trait]
pub trait ShutterControl: Send + Sync {
    /// Open the shutter (allow beam to pass)
    ///
    /// # Returns
    /// - Ok(()) if shutter opened successfully
    /// - Err if shutter cannot be opened or hardware error
    ///
    /// # Safety
    /// Opening the shutter on a high-power laser creates an immediate
    /// eye/skin hazard. Verify safety interlocks before calling.
    async fn open_shutter(&self) -> Result<()>;

    /// Close the shutter (block beam)
    ///
    /// # Returns
    /// - Ok(()) if shutter closed successfully
    /// - Err if shutter cannot be closed or hardware error
    async fn close_shutter(&self) -> Result<()>;

    /// Query shutter state
    ///
    /// # Returns
    /// - Ok(true) if shutter is open (beam can pass)
    /// - Ok(false) if shutter is closed (beam blocked)
    /// - Err if state cannot be determined
    async fn is_shutter_open(&self) -> Result<bool>;
}

/// Capability: Emission Control
///
/// Devices with controllable emission (lasers, light sources).
///
/// # Contract
/// - `enable_emission()` activates the source
/// - `disable_emission()` deactivates the source
/// - Emission state should be queryable when possible
///
/// # Safety
/// CAUTION: Enabling emission on a high-power laser creates immediate
/// hazards. Always verify safety interlocks and shutter state first.
#[async_trait]
pub trait EmissionControl: Send + Sync {
    /// Enable emission (turn on the source)
    ///
    /// # Returns
    /// - Ok(()) if emission enabled successfully
    /// - Err if emission cannot be enabled or hardware error
    ///
    /// # Safety
    /// Enabling emission on high-power sources requires:
    /// - Proper PPE (safety glasses, etc.)
    /// - Verified beam path
    /// - Interlock systems active
    async fn enable_emission(&self) -> Result<()>;

    /// Disable emission (turn off the source)
    ///
    /// # Returns
    /// - Ok(()) if emission disabled successfully
    /// - Err if emission cannot be disabled or hardware error
    async fn disable_emission(&self) -> Result<()>;

    /// Query emission state
    ///
    /// # Returns
    /// - Ok(true) if emission is active
    /// - Ok(false) if emission is inactive
    /// - Err if state cannot be determined
    ///
    /// # Default Implementation
    /// Returns error indicating state query is not supported.
    async fn is_emission_enabled(&self) -> Result<bool> {
        anyhow::bail!("Emission state query not supported by this device")
    }
}

/// Capability: Device Staging (Bluesky-style lifecycle)
///
/// Devices that require preparation before acquisition sequences and cleanup after.
/// This follows the Bluesky/ophyd device lifecycle pattern.
///
/// # Contract
/// - `stage()` prepares device for acquisition (e.g., configure buffers, enable triggers)
/// - `unstage()` cleans up after acquisition (e.g., release resources, reset state)
/// - Staging/unstaging may be nested (count references internally if needed)
///
/// # Usage Pattern
/// ```rust,ignore
/// // Before scan
/// device.stage().await?;
///
/// // Perform acquisition
/// for position in scan_positions {
///     stage.move_abs(position).await?;
///     camera.trigger().await?;
/// }
///
/// // After scan
/// device.unstage().await?;
/// ```
#[async_trait]
pub trait Stageable: Send + Sync {
    /// Prepare device for acquisition sequence
    ///
    /// Called before a scan or acquisition sequence begins.
    /// May configure hardware buffers, enable triggers, or set parameters.
    ///
    /// # Returns
    /// - Ok(()) if staging successful
    /// - Err if device cannot be staged or is in error state
    async fn stage(&self) -> Result<()>;

    /// Clean up after acquisition sequence
    ///
    /// Called after a scan or acquisition sequence completes.
    /// Should release resources, disable triggers, and reset state.
    ///
    /// # Returns
    /// - Ok(()) if unstaging successful
    /// - Err if device cannot be unstaged or is in error state
    async fn unstage(&self) -> Result<()>;

    /// Query staging state
    ///
    /// # Returns
    /// - Ok(true) if device is currently staged
    /// - Ok(false) if device is not staged
    /// - Err if state cannot be determined or not supported
    ///
    /// # Default Implementation
    /// Returns an error indicating state query is not supported.
    async fn is_staged(&self) -> Result<bool> {
        anyhow::bail!("Staged state query not supported by this device")
    }
}

/// Capability: Settable (Configurable Parameters)
///
/// Devices that have parameters which can be set and optionally queried.
///
/// # Contract
/// - `set_value()` sets the parameter to a new value.
/// - `get_value()` queries the current value of the parameter.
/// - Values are represented as `serde_json::Value` to allow flexibility (f64, i64, bool, string, enum).
/// - Methods take `&self` (not `&mut self`) to allow use with `Arc<dyn Settable>`.
///   Implementations should use interior mutability (e.g., `Mutex`) for state changes.
#[async_trait]
pub trait Settable: Send + Sync {
    /// Set a named parameter to a new value.
    ///
    /// # Arguments
    /// * `name` - The identifier for the parameter to set.
    /// * `value` - The new value for the parameter.
    async fn set_value(&self, name: &str, value: serde_json::Value) -> Result<()>;

    /// Get the current value of a named parameter.
    ///
    /// # Arguments
    /// * `name` - The identifier for the parameter to query.
    async fn get_value(&self, name: &str) -> Result<serde_json::Value> {
        anyhow::bail!("Get value for '{}' not supported by this device", name)
    }
}

/// Capability: Switchable (On/Off States)
///
/// Devices that can be turned on or off.
///
/// # Contract
/// - `turn_on()` activates the device/feature.
/// - `turn_off()` deactivates the device/feature.
/// - `is_on()` queries the current on/off state.
#[async_trait]
pub trait Switchable: Send + Sync {
    /// Turn on a named switchable feature.
    ///
    /// # Arguments
    /// * `name` - The identifier for the feature to turn on.
    async fn turn_on(&mut self, name: &str) -> Result<()>;

    /// Turn off a named switchable feature.
    ///
    /// # Arguments
    /// * `name` - The identifier for the feature to turn off.
    async fn turn_off(&mut self, name: &str) -> Result<()>;

    /// Query the on/off state of a named switchable feature.
    ///
    /// # Arguments
    /// * `name` - The identifier for the feature to query.
    ///
    /// # Returns
    /// - `Ok(true)` if the feature is on.
    /// - `Ok(false)` if the feature is off.
    /// - `Err` if the state cannot be determined or is not supported.
    async fn is_on(&self, name: &str) -> Result<bool> {
        anyhow::bail!("State query for '{}' not supported by this device", name)
    }
}

/// Capability: Actionable (One-Time Commands)
///
/// Devices that can perform one-time actions.
///
/// # Contract
/// - `execute_action()` triggers a specific action.
/// - Actions are typically fire-and-forget or block until completion.
#[async_trait]
pub trait Actionable: Send + Sync {
    /// Execute a named one-time action.
    ///
    /// # Arguments
    /// * `name` - The identifier for the action to execute.
    async fn execute_action(&mut self, name: &str) -> Result<()>;
}

/// Capability: Loggable (Static Metadata)
///
/// Devices that provide static, typically read-only, identification or configuration data.
/// This data is usually read once at initialization and logged.
///
/// # Contract
/// - `get_log_value()` retrieves a specific piece of loggable data.
/// - Values are typically strings (e.g., serial number, firmware version).
#[async_trait]
pub trait Loggable: Send + Sync {
    /// Get a named piece of static loggable data.
    ///
    /// # Arguments
    /// * `name` - The identifier for the loggable data (e.g., "serial_number", "firmware_version").
    async fn get_log_value(&self, name: &str) -> Result<String>;
}

/// Capability: Parameter Registry Access
///
/// Devices that expose their parameters for introspection and control.
///
/// This trait enables generic code (gRPC, presets, HDF5 writers) to:
/// - List all parameters of a device
/// - Subscribe to parameter changes
/// - Snapshot device state for reproducibility
///
/// # Contract
/// - `parameters()` returns a reference to the device's parameter registry
/// - The ParameterSet should contain all mutable device parameters
/// - Parameters must use Parameter<T> for hardware-backed state
///
/// # Example
///
/// ```rust,ignore
/// impl Parameterized for MockCamera {
///     fn parameters(&self) -> &ParameterSet {
///         &self.params
///     }
/// }
///
/// // Generic code can now enumerate parameters
/// fn list_all_parameters<D: Parameterized>(device: &D) {
///     for name in device.parameters().names() {
///         println!("Parameter: {}", name);
///     }
/// }
/// ```
pub trait Parameterized: Send + Sync {
    /// Get device's parameter registry
    fn parameters(&self) -> &ParameterSet;

    /// Get metadata for a specific parameter (cached by registry on registration).
    fn get_parameter_metadata(&self, name: &str) -> Option<ParameterMetadata> {
        self.parameters()
            .get(name)
            .map(|param| ParameterMetadata::from(&param.metadata()))
    }
}

// =============================================================================
// Trait Composition Examples (Documentation)
// =============================================================================
//
// Example: Triggered Camera
//
// A camera that supports external triggering would implement:
// ```rust,ignore
// struct TriggeredCamera { /* ... */ }
//
// impl Triggerable for TriggeredCamera { /* ... */ }
// impl ExposureControl for TriggeredCamera { /* ... */ }
// impl FrameProducer for TriggeredCamera { /* ... */ }
//
// // Use in generic scan code
// async fn scan_with_camera<C>(camera: &C) -> Result<()>
// where
//     C: Triggerable + ExposureControl + FrameProducer
// {
//     camera.set_exposure(0.1).await?;
//     camera.arm().await?;
//     camera.trigger().await?;
//     Ok(())
// }
// ```
//
// =============================================================================
// Combined Traits (for trait objects)
// =============================================================================

/// Composite trait for cameras (convenience)
pub trait Camera: Triggerable + FrameProducer {}

/// Blanket implementation - any type implementing both traits gets Camera for free
impl<T: Triggerable + FrameProducer> Camera for T {}

/// Example: Motion Stage
///
/// A motorized stage would implement:
/// ```rust,ignore
/// struct ESP300Stage { /* ... */ }
///
/// impl Movable for ESP300Stage { /* ... */ }
///
/// // Optionally also triggerable for synchronized scans
/// impl Triggerable for ESP300Stage { /* ... */ }
///
/// // Use in generic scan code
/// async fn line_scan<S>(stage: &S, start: f64, end: f64, steps: usize) -> Result<()>
/// where
///     S: Movable
/// {
///     for position in linspace(start, end, steps) {
///         stage.move_abs(position).await?;
///         stage.wait_settled().await?;
///         // Acquire data...
///     }
///     Ok(())
/// }
/// ```
/// Example: Power Meter
///
/// A simple power meter implements only Readable:
/// ```rust,ignore
/// struct NewportPowerMeter { /* ... */ }
///
/// impl Readable for NewportPowerMeter {
///     async fn read(&self) -> Result<f64> {
///         // SCPI query, return watts
///         Ok(0.042)
///     }
/// }
///
/// // Use in generic monitoring code
/// async fn monitor<R>(sensor: &R) -> Result<Vec<f64>>
/// where
///     R: Readable
/// {
///     let mut readings = Vec::new();
///     for _ in 0..100 {
///         readings.push(sensor.read().await?);
///         tokio::time::sleep(Duration::from_millis(10)).await;
///     }
///     Ok(readings)
/// }
/// ```
/// Capability: Generic Command Execution
///
/// Devices that can execute specialized commands with structured arguments.
///
/// # Contract
/// - `execute_command()` takes a command name and JSON arguments.
/// - Returns a JSON object with results.
#[async_trait]
pub trait Commandable: Send + Sync {
    /// Execute a specialized command
    ///
    /// # Arguments
    /// * `command` - Command identifier
    /// * `args` - Command arguments as a JSON object
    ///
    /// # Returns
    /// - Ok(JSON object) with results
    /// - Err if command unknown or execution failed
    async fn execute_command(
        &self,
        command: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value>;
}

// =============================================================================
// LIBS-Specific Capabilities
// =============================================================================

/// Gate mode for ICCD cameras with digital delay generators
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    /// Continuous wave mode - gate always open
    CWOn,
    /// Digital delay generator mode - gate controlled by DDG
    Ddg,
    /// Fire and forget mode - single gate pulse
    FireAndForget,
}

/// Temperature status for ICCD cameras
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemperatureStatus {
    /// CCD is at target temperature and stabilized
    Stabilized,
    /// CCD is actively cooling toward target
    Cooling,
    /// CCD is not at target temperature
    NotStabilized,
}

/// Capability: Gated Camera Control (ICCD)
///
/// Intensified CCD cameras with digital delay generator and MCP gain control.
/// Used for time-resolved spectroscopy (LIBS, fluorescence).
///
/// # Contract
/// - Extends `FrameProducer` with gating and intensifier controls
/// - Gate timing is in picoseconds for sub-nanosecond precision
/// - MCP gain range is device-specific (typically 0-1000)
/// - IntelliGate mode enables automatic gain adjustment
///
/// # Safety
/// CAUTION: High MCP gain with bright light can damage the intensifier.
/// Always verify gate timing and ambient light conditions before enabling MCP.
#[async_trait]
pub trait GatedCamera: FrameProducer {
    /// Set gate mode
    ///
    /// # Arguments
    /// * `mode` - Gate mode (CwOn, Ddg, FireAndForget)
    ///
    /// # Returns
    /// - Ok(()) if mode set successfully
    /// - Err if mode not supported or hardware error
    async fn set_gate_mode(&self, mode: GateMode) -> Result<()>;

    /// Set digital delay generator timing
    ///
    /// # Arguments
    /// * `delay_ps` - Gate delay in picoseconds (relative to trigger)
    /// * `width_ps` - Gate width in picoseconds
    ///
    /// # Contract
    /// - Delay and width must be within hardware limits (device-specific)
    /// - Only applies when gate mode is `Ddg`
    ///
    /// # Returns
    /// - Ok(()) if timing set successfully
    /// - Err if values out of range or hardware error
    async fn set_ddg_timing(&self, delay_ps: u64, width_ps: u64) -> Result<()>;

    /// Set MCP (micro-channel plate) gain
    ///
    /// # Arguments
    /// * `gain` - Gain value (device-specific range, typically 0-1000)
    ///
    /// # Safety
    /// High gain with bright light can damage the intensifier.
    /// Start with low gain and increase gradually.
    ///
    /// # Returns
    /// - Ok(()) if gain set successfully
    /// - Err if gain out of range or hardware error
    async fn set_mcp_gain(&self, gain: u16) -> Result<()>;

    /// Enable/disable IntelliGate automatic gain mode
    ///
    /// # Arguments
    /// * `enabled` - True to enable IntelliGate, false to disable
    ///
    /// # Contract
    /// - IntelliGate automatically adjusts MCP gain based on signal level
    /// - Manual gain setting is ignored when IntelliGate is enabled
    ///
    /// # Returns
    /// - Ok(()) if mode changed successfully
    /// - Err if not supported or hardware error
    async fn set_intelligate(&self, enabled: bool) -> Result<()>;

    /// Get CCD temperature status
    ///
    /// # Returns
    /// - Ok(status) indicating cooling state
    /// - Err if temperature cannot be read or not supported
    async fn get_temperature_status(&self) -> Result<TemperatureStatus>;
}

/// Capability: Spectrometer Control
///
/// Devices with tunable wavelength, grating selection, and wavelength calibration.
/// Used for spectroscopy applications (LIBS, Raman, fluorescence).
///
/// # Contract
/// - Grating numbers are device-specific (1-indexed, typically 1-3)
/// - Wavelength is in nanometers
/// - Slit widths are in micrometers
/// - Calibration returns pixel-to-wavelength mapping
///
/// # Safety
/// CAUTION: Moving gratings while shutters are open can expose
/// sensitive detectors to uncalibrated wavelengths.
#[async_trait]
pub trait SpectrometerControl: Send + Sync {
    /// Set active grating
    ///
    /// # Arguments
    /// * `grating_num` - Grating number (1-indexed, device-specific)
    ///
    /// # Returns
    /// - Ok(()) if grating set successfully
    /// - Err if grating number invalid or hardware error
    async fn set_grating(&self, grating_num: u8) -> Result<()>;

    /// Get active grating
    ///
    /// # Returns
    /// - Ok(grating_num) - Current grating number
    /// - Err if grating cannot be read
    async fn get_grating(&self) -> Result<u8>;

    /// Set center wavelength
    ///
    /// # Arguments
    /// * `nm` - Center wavelength in nanometers
    ///
    /// # Contract
    /// - Valid range depends on grating and spectrometer model
    /// - Moving to zero-order position may require special handling
    ///
    /// # Returns
    /// - Ok(()) if wavelength set successfully
    /// - Err if wavelength out of range or hardware error
    async fn set_wavelength(&self, nm: f64) -> Result<()>;

    /// Get current center wavelength
    ///
    /// # Returns
    /// - Ok(nm) - Current center wavelength
    /// - Err if wavelength cannot be read
    async fn get_wavelength(&self) -> Result<f64>;

    /// Set slit width
    ///
    /// # Arguments
    /// * `slit_id` - Slit identifier (1=entrance, 2=exit, device-specific)
    /// * `width_um` - Slit width in micrometers
    ///
    /// # Returns
    /// - Ok(()) if slit width set successfully
    /// - Err if slit_id invalid or width out of range
    async fn set_slit_width(&self, slit_id: u8, width_um: u16) -> Result<()>;

    /// Get wavelength calibration for detector
    ///
    /// # Arguments
    /// * `num_pixels` - Number of detector pixels (for array size)
    ///
    /// # Returns
    /// - Ok(Vec<f64>) - Wavelength in nm for each pixel
    /// - Err if calibration unavailable or num_pixels invalid
    ///
    /// # Contract
    /// - Returns a vector of length `num_pixels`
    /// - Calibration depends on current grating and center wavelength
    async fn get_calibration(&self, num_pixels: usize) -> Result<Vec<f64>>;

    /// Check if spectrometer is at zero order position
    ///
    /// # Returns
    /// - Ok(true) if at zero order (direct beam path)
    /// - Ok(false) if dispersed position
    /// - Err if position cannot be determined
    async fn is_at_zero_order(&self) -> Result<bool>;

    /// Set shutter state
    ///
    /// # Arguments
    /// * `open` - True to open shutter, false to close
    ///
    /// # Returns
    /// - Ok(()) if shutter state changed successfully
    /// - Err if shutter control failed or not supported
    async fn set_shutter(&self, open: bool) -> Result<()>;
}

/// Trigger source for pulse generators and motion controllers
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSource {
    /// Software trigger command
    Software,
    /// External hardware trigger
    External {
        /// Hardware channel identifier
        channel: String,
    },
    /// Position-based trigger (for TriggerOnPosition)
    Position,
}

/// Capability: Trigger on Position (Dover Motion Controllers)
///
/// Motion stages that can generate trigger pulses at precise position increments.
/// Used for synchronized scanning with cameras/spectrometers.
///
/// # Contract
/// - Extends `Movable` with position-triggered output
/// - Trigger pulses are generated at `increment` intervals between `start_pos` and `end_pos`
/// - Pulse width is in nanoseconds
/// - Bidirectional mode generates triggers in both directions
///
/// # Safety
/// CAUTION: Ensure connected devices can handle the trigger rate
/// at maximum velocity and minimum increment.
#[async_trait]
pub trait TriggerOnPosition: Movable {
    /// Enable trigger-on-position mode
    ///
    /// # Arguments
    /// * `start_pos` - Position where triggering begins (device units)
    /// * `end_pos` - Position where triggering ends (device units)
    /// * `increment` - Position increment between triggers (device units)
    /// * `bidirectional` - Generate triggers in both directions if true
    /// * `pulse_width_ns` - Trigger pulse width in nanoseconds
    ///
    /// # Contract
    /// - Triggers are generated at: start_pos, start_pos+increment, start_pos+2*increment, ...
    /// - In bidirectional mode, triggers also occur during reverse motion
    /// - Pulse width must be sufficient for connected device (typically 1-10µs)
    ///
    /// # Returns
    /// - Ok(()) if TOP enabled successfully
    /// - Err if parameters invalid or hardware error
    async fn enable_top(
        &self,
        start_pos: f64,
        end_pos: f64,
        increment: f64,
        bidirectional: bool,
        pulse_width_ns: u64,
    ) -> Result<()>;

    /// Disable trigger-on-position mode
    ///
    /// # Returns
    /// - Ok(()) if TOP disabled successfully
    /// - Err if hardware error
    async fn disable_top(&self) -> Result<()>;

    /// Check if trigger-on-position is enabled
    ///
    /// # Returns
    /// - Ok(true) if TOP is enabled
    /// - Ok(false) if TOP is disabled
    /// - Err if state cannot be determined
    async fn is_top_enabled(&self) -> Result<bool>;
}

/// Capability: Pulse Generator
///
/// Devices that can generate pulse trains with precise timing.
/// Used for triggering cameras, lasers, and other instruments.
///
/// # Contract
/// - Pulse train timing is in seconds (not milliseconds)
/// - `wait_done()` blocks until pulse train completes
/// - Trigger source determines when pulse train starts
#[async_trait]
pub trait PulseGenerator: Send + Sync {
    /// Configure pulse train parameters
    ///
    /// # Arguments
    /// * `high_time_s` - Duration of high state in seconds
    /// * `low_time_s` - Duration of low state in seconds
    /// * `num_pulses` - Number of pulses to generate (0 = continuous)
    ///
    /// # Contract
    /// - Pulse frequency = 1 / (high_time_s + low_time_s)
    /// - Timing precision depends on hardware (typically 1-10ns)
    ///
    /// # Returns
    /// - Ok(()) if configuration successful
    /// - Err if timing values invalid or hardware error
    async fn configure_pulse_train(
        &self,
        high_time_s: f64,
        low_time_s: f64,
        num_pulses: u32,
    ) -> Result<()>;

    /// Wait for pulse train to complete
    ///
    /// # Contract
    /// - Blocks until `num_pulses` have been generated
    /// - Returns immediately if num_pulses=0 (continuous mode)
    /// - Should have internal timeout to prevent infinite blocking
    ///
    /// # Returns
    /// - Ok(()) when pulse train completes
    /// - Err on timeout or hardware error
    async fn wait_done(&self) -> Result<()>;

    /// Set trigger source for pulse train
    ///
    /// # Arguments
    /// * `source` - Trigger source (Software, External, Position)
    ///
    /// # Contract
    /// - Software: pulse train starts on explicit trigger command
    /// - External: pulse train starts on hardware trigger signal
    /// - Position: pulse train synchronized with motion (requires TriggerOnPosition)
    ///
    /// # Returns
    /// - Ok(()) if trigger source set successfully
    /// - Err if source not supported or hardware error
    async fn set_trigger_source(&self, source: TriggerSource) -> Result<()>;
}

/// Interlock status for safety systems
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterlockStatus {
    /// Interlock is closed (safe to operate)
    Closed,
    /// Interlock is open (unsafe condition detected)
    Open,
    /// Interlock status cannot be determined
    Unknown,
}

/// Capability: Safety Interlock
///
/// Devices with safety interlock monitoring for laser safety systems.
///
/// # Contract
/// - Interlock open indicates unsafe condition (door open, beam path obstructed)
/// - Systems should refuse to enable emission when interlock is open
/// - GUI should display prominent warning when interlock is open
///
/// # Safety
/// CRITICAL: This is a monitoring capability only. Never rely on software
/// interlocks alone for laser safety. Always use hardware interlocks that
/// directly cut power to the laser in unsafe conditions.
#[async_trait]
pub trait SafetyInterlock: Send + Sync {
    /// Check if safety interlock is open
    ///
    /// # Returns
    /// - Ok(true) if interlock is OPEN (unsafe condition)
    /// - Ok(false) if interlock is CLOSED (safe to operate)
    /// - Err if interlock status cannot be determined
    ///
    /// # Safety
    /// This is a monitoring function only. Hardware interlocks
    /// should independently disable hazardous systems.
    async fn is_interlock_open(&self) -> Result<bool>;

    /// Get detailed interlock status
    ///
    /// # Returns
    /// - Ok(status) with current interlock state
    /// - Err if status cannot be determined
    async fn interlock_status(&self) -> Result<InterlockStatus>;
}

// =============================================================================
// Runtime Reconfiguration (Phase 5 — Hot-Swap)
// =============================================================================

/// Capability: Runtime Reconfiguration
///
/// Devices that support applying new configuration without a full restart.
/// The reconciler calls `reconfigure()` when a config change is detected
/// in the database and the device is not actively measuring.
///
/// # Contract
/// - Validate config before applying (reject invalid configs gracefully)
/// - Preserve state where possible (don't reset counters, cached data, etc.)
/// - Return `Err` if the change requires a full restart (caller will
///   fall back to unregister + register)
/// - Implementations should be idempotent: applying the same config twice is a no-op
#[async_trait]
pub trait Reconfigurable: Send + Sync {
    /// Apply new configuration values at runtime.
    ///
    /// # Returns
    /// - `Ok(())` if the config was applied successfully
    /// - `Err` if the change requires a full driver restart
    async fn reconfigure(&self, config: toml::Value) -> Result<()>;
}

// =============================================================================
// Post-Reconnection State Refresh (bd-47p2)
// =============================================================================

/// Capability: Post-Reconnection State Refresh
///
/// Devices that can query their current hardware state after a reconnection
/// (e.g., DeviceSupervisor restart). This ensures cached software state is
/// re-synchronized with actual hardware state, preventing silent divergence.
///
/// # Contract
/// - Query all readable parameters/state from the hardware
/// - Update internal cached values without triggering hardware write callbacks
/// - Return a summary of refreshed state as key-value pairs
/// - Errors in individual parameter reads should be logged but not abort
///   the entire refresh (best-effort)
/// - Safe to call multiple times (idempotent)
///
/// # Motivation
/// After a TCP/SCPI device reconnects, the device may have been power-cycled
/// or manually adjusted. The software's cached position, wavelength, shutter
/// state, etc. may no longer match reality. This trait provides a standardized
/// hook for the supervisor to trigger a full state re-read.
#[async_trait]
pub trait StateRefreshable: Send + Sync {
    /// Refresh all readable device state from hardware.
    ///
    /// Returns a map of parameter names to their current values as read
    /// from the hardware. This is purely informational (for logging/events);
    /// the implementation is responsible for updating its own internal state.
    ///
    /// # Errors
    /// Returns `Err` only if the refresh failed catastrophically (e.g.,
    /// transport is broken). Individual parameter read failures should be
    /// logged and skipped.
    async fn refresh_state(&self) -> Result<HashMap<String, serde_json::Value>>;
}

/// State machine for safe runtime reconfiguration.
///
/// Prevents config changes while a device is actively measuring.
/// The reconciler checks the lock state before calling `reconfigure()`.
///
/// ```text
/// Idle ──start_measurement()──► Measuring
///  ▲                              │
///  └──finish_measurement()────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MeasurementLock {
    /// Device is idle — safe to reconfigure.
    #[default]
    Idle,
    /// Device is actively measuring — reconfiguration must wait.
    Measuring,
}

impl MeasurementLock {
    /// Returns `true` if the device is idle and safe to reconfigure.
    pub fn is_idle(&self) -> bool {
        *self == Self::Idle
    }
}

// =============================================================================
// Composite Capabilities (bd-bog5)
// =============================================================================

/// A composite operation that coordinates multiple device capabilities.
///
/// Implementations combine capabilities from multiple devices into
/// higher-level operations (e.g., synchronized acquisition, coordinated motion).
#[async_trait]
pub trait CompositeCapability: Send + Sync {
    /// Human-readable name for this composite operation.
    fn name(&self) -> &str;

    /// List of (device_id, capability) pairs required for execution.
    fn required_capabilities(&self) -> Vec<(String, crate::driver::Capability)>;

    /// Execute the composite operation using capabilities from the provider.
    async fn execute(&self, provider: &dyn CapabilityProvider) -> Result<()>;
}

/// Provides typed access to device capabilities by device ID.
///
/// This abstraction decouples composite operations from the concrete
/// `DeviceRegistry`, enabling testing with mock providers.
pub trait CapabilityProvider: Send + Sync {
    /// Get a device's Movable capability (if supported).
    fn get_movable(&self, id: &str) -> Option<Arc<dyn Movable>>;
    /// Get a device's Readable capability (if supported).
    fn get_readable(&self, id: &str) -> Option<Arc<dyn Readable>>;
    /// Get a device's Triggerable capability (if supported).
    fn get_triggerable(&self, id: &str) -> Option<Arc<dyn Triggerable>>;
    /// Get a device's FrameProducer capability (if supported).
    fn get_frame_producer(&self, id: &str) -> Option<Arc<dyn FrameProducer>>;
    /// Get a device's ExposureControl capability (if supported).
    fn get_exposure_control(&self, id: &str) -> Option<Arc<dyn ExposureControl>>;
    /// Get a device's ShutterControl capability (if supported).
    fn get_shutter_control(&self, id: &str) -> Option<Arc<dyn ShutterControl>>;
    /// Get a device's WavelengthTunable capability (if supported).
    fn get_wavelength_tunable(&self, id: &str) -> Option<Arc<dyn WavelengthTunable>>;
    /// Get a device's EmissionControl capability (if supported).
    fn get_emission_control(&self, id: &str) -> Option<Arc<dyn EmissionControl>>;
    /// Get a device's Settable capability (if supported).
    fn get_settable(&self, id: &str) -> Option<Arc<dyn Settable>>;
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // Mock implementations for testing

    struct MockStage {
        position: std::sync::Mutex<f64>,
    }

    #[async_trait]
    impl Movable for MockStage {
        async fn move_abs(&self, position: f64) -> Result<()> {
            *self.position.lock().unwrap() = position;
            Ok(())
        }

        async fn move_rel(&self, distance: f64) -> Result<()> {
            *self.position.lock().unwrap() += distance;
            Ok(())
        }

        async fn position(&self) -> Result<f64> {
            Ok(*self.position.lock().unwrap())
        }

        async fn wait_settled(&self) -> Result<()> {
            // Simulate settling time
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_movable_trait() {
        let stage = MockStage {
            position: std::sync::Mutex::new(0.0),
        };

        // Test absolute move
        stage.move_abs(10.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 10.0);

        // Test relative move
        stage.move_rel(5.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 15.0);

        // Test settle
        stage.wait_settled().await.unwrap();
    }

    struct MockPowerMeter;

    #[async_trait]
    impl Readable for MockPowerMeter {
        async fn read(&self) -> Result<f64> {
            Ok(0.123)
        }
    }

    #[tokio::test]
    async fn test_readable_trait() {
        let meter = MockPowerMeter;
        let reading = meter.read().await.unwrap();
        assert_eq!(reading, 0.123);
    }
}
