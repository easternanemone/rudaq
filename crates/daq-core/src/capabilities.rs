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

use crate::observable::ParameterSet;
use anyhow::Result;
use async_trait::async_trait;

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

    /// Icon for UI display
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Camera => "📷",
            Self::Stage => "🔄",
            Self::Detector => "📊",
            Self::Laser => "💡",
            Self::PowerMeter => "⚡",
            Self::Other => "🔧",
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
/// use daq_core::capabilities::{FrameObserver, ObserverHandle};
/// use daq_core::data::FrameView;
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
    /// delivered to primary consumers. It MUST NOT block.
    ///
    /// # Arguments
    ///
    /// - `frame`: Zero-copy view into frame data (valid only for this call)
    fn on_frame(&self, frame: &crate::data::FrameView<'_>);

    /// Optional: Return a descriptive name for this observer (for debugging/logging).
    fn name(&self) -> &str {
        "unnamed_observer"
    }
}

/// Type alias for pooled frame data from the object pool.
///
/// This represents a frame buffer loaned from a pre-allocated pool,
/// enabling zero-allocation frame handling for high-FPS scenarios.
pub type LoanedFrame = daq_pool::Loaned<daq_pool::FrameData>;

/// Capability: Frame/Image Production
///
/// Devices that produce 2D image frames (cameras, beam profilers).
///
/// # Contract
/// - `start_stream()` begins continuous acquisition
/// - `stop_stream()` halts acquisition
/// - Frames are delivered via `register_primary_output()` channel (preferred)
/// - `resolution()` is immutable (cannot be changed via this trait)
///
/// # Frame Delivery
///
/// ## Preferred: `register_primary_output()` (zero-allocation)
/// Call `register_primary_output()` BEFORE `start_stream()` to register a channel
/// that will receive `LoanedFrame` objects with ownership. The primary consumer
/// owns the frames and controls when they return to the pool.
///
/// ## Legacy: `subscribe_frames()` (deprecated)
/// Returns a broadcast receiver for `Arc<Frame>`. Multiple subscribers receive
/// the same frames but with heap allocation overhead.
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
    /// **DEPRECATED**: Use `subscribe_frames()` instead for multi-subscriber support.
    ///
    /// This can only be called once - subsequent calls return None.
    /// Call this BEFORE `start_stream()` to receive frames.
    ///
    /// # Returns
    /// - Some(receiver) if receiver is available
    /// - None if receiver was already taken or not supported by this device
    #[deprecated(
        since = "0.2.0",
        note = "Use subscribe_frames() for multi-subscriber support"
    )]
    async fn take_frame_receiver(&self) -> Option<tokio::sync::mpsc::Receiver<crate::data::Frame>> {
        // Default: no frame receiver support
        None
    }

    /// Subscribe to the frame stream
    ///
    /// **DEPRECATED**: Use `register_primary_output()` for zero-allocation pooled frames.
    /// This method will be removed in a future release.
    ///
    /// Returns a broadcast receiver that will receive `Arc<Frame>` for each captured frame.
    /// Multiple subscribers can receive the same frames without copying pixel data.
    /// Can be called multiple times to create additional subscribers.
    ///
    /// # Returns
    /// - Some(receiver) if subscription succeeded
    /// - None if streaming is not supported by this device
    ///
    /// # Example
    /// ```rust,ignore
    /// let rx = camera.subscribe_frames().await?;
    /// while let Ok(frame) = rx.recv().await {
    ///     // Process Arc<Frame> without copying pixel data
    ///     println!("Frame: {}x{}", frame.width, frame.height);
    /// }
    /// ```
    #[deprecated(
        since = "0.3.0",
        note = "Use register_primary_output() for zero-allocation pooled frames"
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

#[cfg(test)]
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
