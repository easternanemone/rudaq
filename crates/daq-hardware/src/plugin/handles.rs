//! Capability Handle types for plugin-based instruments.
//!
//! This module provides lightweight handle types that wrap `Arc<GenericDriver>` and
//! implement the standard capability traits from `crate::hardware::capabilities`.
//!
//! # Design Pattern
//!
//! Each handle type:
//! - Wraps `Arc<GenericDriver>` for shared ownership
//! - Stores the name of the specific capability (e.g., axis name, sensor name)
//! - Implements one capability trait (Movable, Readable, etc.)
//! - Delegates to GenericDriver methods with the stored name
//!
//! This pattern allows a single GenericDriver to provide multiple capability-trait
//! objects (e.g., multiple axes from one multi-axis controller).
//!
//! # Example
//!
//! ```rust,ignore
//! use std::sync::Arc;
//!
//! // Create driver from plugin config
//! let driver = Arc::new(factory.spawn("my-stage", "/dev/ttyUSB0").await?);
//!
//! // Create handles for specific capabilities
//! let x_axis = PluginAxisHandle::new(driver.clone(), "x", false);
//! let y_axis = PluginAxisHandle::new(driver.clone(), "y", false);
//!
//! // Use via Movable trait
//! x_axis.move_abs(10.0).await?;
//! y_axis.move_abs(20.0).await?;
//! ```

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use super::driver::GenericDriver;
use crate::capabilities::{
    Actionable, ExposureControl, FrameProducer, Loggable, Movable, Readable, Settable, Switchable,
    Triggerable,
};

/// Default timeout for motion settling operations.
const DEFAULT_SETTLE_TIMEOUT: Duration = Duration::from_secs(30);

// =============================================================================
// PluginAxisHandle - Implements Movable
// =============================================================================

/// A handle to a specific axis on a plugin-based motion controller.
///
/// Implements the `Movable` trait by delegating to the underlying `GenericDriver`.
///
/// # Example
///
/// ```rust,ignore
/// let handle = PluginAxisHandle::new(driver.clone(), "x", false);
/// handle.move_abs(10.0).await?;
/// let pos = handle.position().await?;
/// ```
pub struct PluginAxisHandle {
    driver: Arc<GenericDriver>,
    axis_name: String,
    is_mocking: bool,
}

impl PluginAxisHandle {
    /// Creates a new axis handle.
    ///
    /// # Arguments
    /// * `driver` - Shared reference to the GenericDriver
    /// * `axis_name` - Name of the axis (must match YAML config)
    /// * `is_mocking` - If true, operations don't communicate with hardware
    pub fn new(driver: Arc<GenericDriver>, axis_name: impl Into<String>, is_mocking: bool) -> Self {
        Self {
            driver,
            axis_name: axis_name.into(),
            is_mocking,
        }
    }

    /// Returns the axis name this handle controls.
    pub fn axis_name(&self) -> &str {
        &self.axis_name
    }

    /// Returns whether this handle is in mock mode.
    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl Movable for PluginAxisHandle {
    async fn move_abs(&self, position: f64) -> Result<()> {
        self.driver
            .move_axis_abs(&self.axis_name, position, self.is_mocking)
            .await
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        self.driver
            .move_axis_rel(&self.axis_name, distance, self.is_mocking)
            .await
    }

    async fn position(&self) -> Result<f64> {
        self.driver
            .get_axis_position(&self.axis_name, self.is_mocking)
            .await
    }

    async fn wait_settled(&self) -> Result<()> {
        self.driver
            .wait_axis_settled(&self.axis_name, self.is_mocking, DEFAULT_SETTLE_TIMEOUT)
            .await
    }
}

// =============================================================================
// PluginSensorHandle - Implements Readable
// =============================================================================

/// A handle to a specific readable sensor on a plugin-based instrument.
///
/// Implements the `Readable` trait by delegating to the underlying `GenericDriver`.
///
/// # Example
///
/// ```rust,ignore
/// let handle = PluginSensorHandle::new(driver.clone(), "power", false);
/// let value = handle.read().await?;
/// ```
pub struct PluginSensorHandle {
    driver: Arc<GenericDriver>,
    sensor_name: String,
    is_mocking: bool,
}

impl PluginSensorHandle {
    /// Creates a new sensor handle.
    ///
    /// # Arguments
    /// * `driver` - Shared reference to the GenericDriver
    /// * `sensor_name` - Name of the readable capability (must match YAML config)
    /// * `is_mocking` - If true, returns mock data instead of communicating with hardware
    pub fn new(
        driver: Arc<GenericDriver>,
        sensor_name: impl Into<String>,
        is_mocking: bool,
    ) -> Self {
        Self {
            driver,
            sensor_name: sensor_name.into(),
            is_mocking,
        }
    }

    /// Returns the sensor name this handle reads from.
    pub fn sensor_name(&self) -> &str {
        &self.sensor_name
    }

    /// Returns whether this handle is in mock mode.
    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl Readable for PluginSensorHandle {
    async fn read(&self) -> Result<f64> {
        self.driver
            .read_named_f64(&self.sensor_name, self.is_mocking)
            .await
    }
}

// =============================================================================
// PluginSettableHandle - Implements Settable
// =============================================================================

/// A handle to settable parameters on a plugin-based instrument.
///
/// Implements the `Settable` trait by delegating to the underlying `GenericDriver`.
/// Unlike axis/sensor handles, this handle manages all settable parameters for the
/// instrument (parameter name is passed to each method).
///
/// # Example
///
/// ```rust,ignore
/// let mut handle = PluginSettableHandle::new(driver.clone(), false);
/// handle.set_value("wavelength", serde_json::json!(780.0)).await?;
/// let value = handle.get_value("wavelength").await?;
/// ```
pub struct PluginSettableHandle {
    driver: Arc<GenericDriver>,
    is_mocking: bool,
}

impl PluginSettableHandle {
    /// Creates a new settable handle.
    ///
    /// # Arguments
    /// * `driver` - Shared reference to the GenericDriver
    /// * `is_mocking` - If true, operations update internal state without hardware communication
    pub fn new(driver: Arc<GenericDriver>, is_mocking: bool) -> Self {
        Self { driver, is_mocking }
    }

    /// Returns whether this handle is in mock mode.
    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl Settable for PluginSettableHandle {
    async fn set_value(&self, name: &str, value: Value) -> Result<()> {
        self.driver
            .set_named_value(name, value, self.is_mocking)
            .await
    }

    async fn get_value(&self, name: &str) -> Result<Value> {
        self.driver.get_named_value(name, self.is_mocking).await
    }
}

// =============================================================================
// PluginSwitchableHandle - Implements Switchable
// =============================================================================

/// A handle to switchable features on a plugin-based instrument.
///
/// Implements the `Switchable` trait by delegating to the underlying `GenericDriver`.
/// Manages all switchable features (feature name is passed to each method).
///
/// # Example
///
/// ```rust,ignore
/// let mut handle = PluginSwitchableHandle::new(driver.clone(), false);
/// handle.turn_on("shutter").await?;
/// let is_open = handle.is_on("shutter").await?;
/// handle.turn_off("shutter").await?;
/// ```
pub struct PluginSwitchableHandle {
    driver: Arc<GenericDriver>,
    is_mocking: bool,
}

impl PluginSwitchableHandle {
    /// Creates a new switchable handle.
    ///
    /// # Arguments
    /// * `driver` - Shared reference to the GenericDriver
    /// * `is_mocking` - If true, operations update internal state without hardware communication
    pub fn new(driver: Arc<GenericDriver>, is_mocking: bool) -> Self {
        Self { driver, is_mocking }
    }

    /// Returns whether this handle is in mock mode.
    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl Switchable for PluginSwitchableHandle {
    async fn turn_on(&mut self, name: &str) -> Result<()> {
        self.driver.turn_on_named(name, self.is_mocking).await
    }

    async fn turn_off(&mut self, name: &str) -> Result<()> {
        self.driver.turn_off_named(name, self.is_mocking).await
    }

    async fn is_on(&self, name: &str) -> Result<bool> {
        self.driver.is_named_on(name, self.is_mocking).await
    }
}

// =============================================================================
// PluginActionableHandle - Implements Actionable
// =============================================================================

/// A handle to actionable commands on a plugin-based instrument.
///
/// Implements the `Actionable` trait by delegating to the underlying `GenericDriver`.
/// Manages all actionable commands (action name is passed to each method).
///
/// # Example
///
/// ```rust,ignore
/// let mut handle = PluginActionableHandle::new(driver.clone(), false);
/// handle.execute_action("home").await?;
/// handle.execute_action("calibrate").await?;
/// ```
pub struct PluginActionableHandle {
    driver: Arc<GenericDriver>,
    is_mocking: bool,
}

impl PluginActionableHandle {
    /// Creates a new actionable handle.
    ///
    /// # Arguments
    /// * `driver` - Shared reference to the GenericDriver
    /// * `is_mocking` - If true, actions are no-ops (return immediately)
    pub fn new(driver: Arc<GenericDriver>, is_mocking: bool) -> Self {
        Self { driver, is_mocking }
    }

    /// Returns whether this handle is in mock mode.
    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl Actionable for PluginActionableHandle {
    async fn execute_action(&mut self, name: &str) -> Result<()> {
        self.driver
            .execute_named_action(name, self.is_mocking)
            .await
    }
}

// =============================================================================
// PluginLoggableHandle - Implements Loggable
// =============================================================================

/// A handle to loggable metadata on a plugin-based instrument.
///
/// Implements the `Loggable` trait by delegating to the underlying `GenericDriver`.
/// Retrieves static metadata values (value name is passed to each method).
///
/// # Example
///
/// ```rust,ignore
/// let handle = PluginLoggableHandle::new(driver.clone(), false);
/// let serial = handle.get_log_value("serial_number").await?;
/// let firmware = handle.get_log_value("firmware_version").await?;
/// ```
pub struct PluginLoggableHandle {
    driver: Arc<GenericDriver>,
    is_mocking: bool,
}

impl PluginLoggableHandle {
    /// Creates a new loggable handle.
    ///
    /// # Arguments
    /// * `driver` - Shared reference to the GenericDriver
    /// * `is_mocking` - If true, returns mock values instead of querying hardware
    pub fn new(driver: Arc<GenericDriver>, is_mocking: bool) -> Self {
        Self { driver, is_mocking }
    }

    /// Returns whether this handle is in mock mode.
    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl Loggable for PluginLoggableHandle {
    async fn get_log_value(&self, name: &str) -> Result<String> {
        self.driver.get_named_loggable(name, self.is_mocking).await
    }
}

// =============================================================================
// PluginTriggerableHandle - Implements Triggerable
// =============================================================================

/// A handle to triggerable capability on a plugin-based instrument.
///
/// Implements the `Triggerable` trait by delegating to the underlying `GenericDriver`.
pub struct PluginTriggerableHandle {
    driver: Arc<GenericDriver>,
    is_mocking: bool,
}

impl PluginTriggerableHandle {
    /// Creates a new triggerable handle.
    ///
    /// # Arguments
    /// * `driver` - Shared reference to the GenericDriver
    /// * `is_mocking` - If true, operations update internal state without hardware communication
    pub fn new(driver: Arc<GenericDriver>, is_mocking: bool) -> Self {
        Self { driver, is_mocking }
    }

    /// Returns whether this handle is in mock mode.
    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl Triggerable for PluginTriggerableHandle {
    async fn arm(&self) -> Result<()> {
        self.driver.arm_trigger(self.is_mocking).await
    }

    async fn trigger(&self) -> Result<()> {
        self.driver.send_trigger(self.is_mocking).await
    }

    async fn is_armed(&self) -> Result<bool> {
        self.driver.is_trigger_armed(self.is_mocking).await
    }
}

// =============================================================================
// PluginExposureControlHandle - Implements ExposureControl
// =============================================================================

/// A handle to exposure control capability on a plugin-based camera/detector.
pub struct PluginExposureControlHandle {
    driver: Arc<GenericDriver>,
    is_mocking: bool,
}

impl PluginExposureControlHandle {
    pub fn new(driver: Arc<GenericDriver>, is_mocking: bool) -> Self {
        Self { driver, is_mocking }
    }

    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl ExposureControl for PluginExposureControlHandle {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        self.driver.set_exposure(seconds, self.is_mocking).await
    }

    async fn get_exposure(&self) -> Result<f64> {
        self.driver.get_exposure(self.is_mocking).await
    }
}

// =============================================================================
// PluginFrameProducerHandle - Implements FrameProducer
// =============================================================================

/// A handle to frame acquisition capability on a plugin-based camera.
///
/// Implements the `FrameProducer` trait by delegating to the underlying `GenericDriver`.
///
/// # Example
///
/// ```rust,ignore
/// let handle = PluginFrameProducerHandle::new(driver.clone(), false);
/// let rx = handle.subscribe_frames().await.unwrap();
/// handle.start_stream().await?;
/// while let Ok(frame) = rx.recv().await {
///     println!("Frame: {}x{}", frame.width, frame.height);
/// }
/// handle.stop_stream().await?;
/// ```
pub struct PluginFrameProducerHandle {
    driver: Arc<GenericDriver>,
    is_mocking: bool,
}

impl PluginFrameProducerHandle {
    /// Creates a new frame producer handle.
    ///
    /// # Arguments
    /// * `driver` - Shared reference to the GenericDriver
    /// * `is_mocking` - If true, generates synthetic frames instead of hardware acquisition
    pub fn new(driver: Arc<GenericDriver>, is_mocking: bool) -> Self {
        Self { driver, is_mocking }
    }

    /// Returns whether this handle is in mock mode.
    pub fn is_mocking(&self) -> bool {
        self.is_mocking
    }
}

#[async_trait]
impl FrameProducer for PluginFrameProducerHandle {
    async fn start_stream(&self) -> Result<()> {
        self.driver.start_frame_stream(self.is_mocking).await
    }

    async fn start_stream_finite(&self, frame_limit: Option<u32>) -> Result<()> {
        self.driver
            .start_frame_stream_finite(frame_limit, self.is_mocking)
            .await
    }

    async fn stop_stream(&self) -> Result<()> {
        self.driver.stop_frame_stream(self.is_mocking).await
    }

    fn resolution(&self) -> (u32, u32) {
        self.driver.frame_resolution()
    }

    async fn subscribe_frames(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<std::sync::Arc<crate::Frame>>> {
        self.driver.subscribe_frames().await
    }

    async fn is_streaming(&self) -> Result<bool> {
        self.driver.is_frame_streaming(self.is_mocking).await
    }

    fn frame_count(&self) -> u64 {
        self.driver.frame_count()
    }

    async fn register_primary_output(
        &self,
        tx: tokio::sync::mpsc::Sender<crate::capabilities::LoanedFrame>,
    ) -> Result<()> {
        self.driver.register_primary_output(tx).await
    }

    async fn register_observer(
        &self,
        observer: Box<dyn crate::capabilities::FrameObserver>,
    ) -> Result<crate::capabilities::ObserverHandle> {
        self.driver.register_observer(observer).await
    }

    async fn unregister_observer(&self, handle: crate::capabilities::ObserverHandle) -> Result<()> {
        self.driver.unregister_observer(handle).await
    }

    fn supports_observers(&self) -> bool {
        self.driver.supports_observers()
    }
}

// =============================================================================
// Factory Methods on GenericDriver
// =============================================================================

impl GenericDriver {
    /// Creates an axis handle for the named axis.
    ///
    /// # Arguments
    /// * `axis_name` - Name of the axis (must exist in YAML movable.axes)
    /// * `is_mocking` - Whether to use mock mode
    ///
    /// # Returns
    /// A `PluginAxisHandle` that implements `Movable`.
    ///
    /// # Note
    /// The caller must wrap `self` in `Arc` before calling this method.
    /// This is typically done by the `PluginFactory` when spawning drivers.
    pub fn axis_handle(
        self: &Arc<Self>,
        axis_name: impl Into<String>,
        is_mocking: bool,
    ) -> PluginAxisHandle {
        PluginAxisHandle::new(self.clone(), axis_name, is_mocking)
    }

    /// Creates a sensor handle for the named readable capability.
    ///
    /// # Arguments
    /// * `sensor_name` - Name of the readable capability (must exist in YAML capabilities.readable)
    /// * `is_mocking` - Whether to use mock mode
    ///
    /// # Returns
    /// A `PluginSensorHandle` that implements `Readable`.
    pub fn sensor_handle(
        self: &Arc<Self>,
        sensor_name: impl Into<String>,
        is_mocking: bool,
    ) -> PluginSensorHandle {
        PluginSensorHandle::new(self.clone(), sensor_name, is_mocking)
    }

    /// Creates a settable handle for all settable parameters.
    ///
    /// # Arguments
    /// * `is_mocking` - Whether to use mock mode
    ///
    /// # Returns
    /// A `PluginSettableHandle` that implements `Settable`.
    pub fn settable_handle(self: &Arc<Self>, is_mocking: bool) -> PluginSettableHandle {
        PluginSettableHandle::new(self.clone(), is_mocking)
    }

    /// Creates a switchable handle for all switchable features.
    ///
    /// # Arguments
    /// * `is_mocking` - Whether to use mock mode
    ///
    /// # Returns
    /// A `PluginSwitchableHandle` that implements `Switchable`.
    pub fn switchable_handle(self: &Arc<Self>, is_mocking: bool) -> PluginSwitchableHandle {
        PluginSwitchableHandle::new(self.clone(), is_mocking)
    }

    /// Creates an actionable handle for all actionable commands.
    ///
    /// # Arguments
    /// * `is_mocking` - Whether to use mock mode
    ///
    /// # Returns
    /// A `PluginActionableHandle` that implements `Actionable`.
    pub fn actionable_handle(self: &Arc<Self>, is_mocking: bool) -> PluginActionableHandle {
        PluginActionableHandle::new(self.clone(), is_mocking)
    }

    /// Creates a loggable handle for all loggable metadata.
    ///
    /// # Arguments
    /// * `is_mocking` - Whether to use mock mode
    ///
    /// # Returns
    /// A `PluginLoggableHandle` that implements `Loggable`.
    pub fn loggable_handle(self: &Arc<Self>, is_mocking: bool) -> PluginLoggableHandle {
        PluginLoggableHandle::new(self.clone(), is_mocking)
    }

    /// Creates an exposure control handle.
    ///
    /// # Arguments
    /// * `is_mocking` - Whether to use mock mode
    ///
    /// # Returns
    /// A `PluginExposureControlHandle` that implements `ExposureControl`.
    pub fn exposure_control_handle(
        self: &Arc<Self>,
        is_mocking: bool,
    ) -> PluginExposureControlHandle {
        PluginExposureControlHandle::new(self.clone(), is_mocking)
    }

    /// Creates a frame producer handle.
    ///
    /// # Arguments
    /// * `is_mocking` - Whether to use mock mode (synthetic frames)
    ///
    /// # Returns
    /// A `PluginFrameProducerHandle` that implements `FrameProducer`.
    pub fn frame_producer_handle(self: &Arc<Self>, is_mocking: bool) -> PluginFrameProducerHandle {
        PluginFrameProducerHandle::new(self.clone(), is_mocking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full testing requires a mock GenericDriver or integration tests.
    // These tests verify the handle types can be constructed and have correct methods.

    // Test that handle types implement the expected traits
    fn _assert_movable<T: Movable>() {}
    fn _assert_readable<T: Readable>() {}
    fn _assert_settable<T: Settable>() {}
    fn _assert_switchable<T: Switchable>() {}
    fn _assert_actionable<T: Actionable>() {}
    fn _assert_loggable<T: Loggable>() {}
    fn _assert_triggerable<T: Triggerable>() {}
    fn _assert_frame_producer<T: FrameProducer>() {}

    #[test]
    fn handles_implement_traits() {
        // Compile-time verification that handles implement their traits
        _assert_movable::<PluginAxisHandle>();
        _assert_readable::<PluginSensorHandle>();
        _assert_settable::<PluginSettableHandle>();
        _assert_switchable::<PluginSwitchableHandle>();
        _assert_actionable::<PluginActionableHandle>();
        _assert_loggable::<PluginLoggableHandle>();
        _assert_frame_producer::<PluginFrameProducerHandle>();
    }
}
