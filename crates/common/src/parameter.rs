//! Parameter<T> - Declarative parameter management (ScopeFoundry pattern)
//!
//! Inspired by ScopeFoundry's LoggedQuantity, this module provides a unified
//! abstraction for instrument parameters that automatically synchronizes:
//! - GUI widgets (via watch channels)
//! - Hardware devices (via callbacks)
//! - Storage (via change listeners)
//!
//! # Architecture
//!
//! Parameter<T> **composes** Observable<T> to avoid code duplication:
//! - Observable<T> handles: watch channels, subscriptions, validation, metadata
//! - Parameter<T> adds: hardware write/read callbacks, change listeners
//!
//! # Introspectable Constraints (Phase 2: bd-cdh5.2)
//!
//! For GUI-aware constraints that render as Sliders or ComboBoxes, use the
//! type-specific `with_range_introspectable()` or `with_choices_introspectable()`
//! methods. These delegate to [`Observable`] and populate metadata fields that
//! are sent to the GUI via gRPC.
//!
//! ```rust,ignore
//! // Float parameter with slider bounds
//! let exposure = Parameter::new("exposure_ms", 100.0)
//!     .with_unit("ms")
//!     .with_range_introspectable(1.0, 10000.0);  // GUI renders as Slider
//!
//! // String parameter with enum choices
//! let fan_speed = Parameter::new("fan_speed", "auto".to_string())
//!     .with_choices_introspectable(vec!["off".into(), "low".into(), "auto".into()]);
//! ```
//!
//! See [`Observable`] for detailed documentation on introspectable constraints.
//!
//! # Basic Example
//!
//! ```rust,ignore
//! use common::parameter::Parameter;
//! use futures::future::BoxFuture;
//!
//! // Create parameter with introspectable constraints
//! let mut exposure = Parameter::new("exposure_ms", 100.0)
//!     .with_range_introspectable(1.0, 10000.0)  // GUI renders as Slider
//!     .with_unit("ms");
//!
//! // Connect to async hardware
//! exposure.connect_to_hardware_write(|val| {
//!     Box::pin(async move {
//!         camera.set_exposure(val).await
//!     })
//! });
//!
//! // Set value (validates, writes to hardware, notifies subscribers)
//! exposure.set(250.0).await?;
//!
//! // Subscribe for GUI updates
//! let mut rx = exposure.subscribe();
//! tokio::spawn(async move {
//!     while rx.changed().await.is_ok() {
//!         let value = *rx.borrow();
//!         println!("Exposure changed to: {}", value);
//!     }
//! });
//! ```
//!
//! # Data Flow
//!
//! ```text
//! User/Script calls param.set(value)
//!         │
//!         ▼
//! ┌───────────────────────────────────────────────────┐
//! │ 1. Validate against constraints (BEFORE hardware) │
//! │    - Range: min <= value <= max                   │
//! │    - Choices: value in enum_values                │
//! │    - NaN/Infinity: rejected for f64               │
//! └───────────────────────────────────────────────────┘
//!         │ (fails here if invalid)
//!         ▼
//! ┌───────────────────────────────────────────────────┐
//! │ 2. Write to hardware (if hardware_writer set)     │
//! └───────────────────────────────────────────────────┘
//!         │ (fails here if hardware error)
//!         ▼
//! ┌───────────────────────────────────────────────────┐
//! │ 3. Update internal value (via Observable)         │
//! │    - Notifies all watch channel subscribers       │
//! └───────────────────────────────────────────────────┘
//!         │
//!         ▼
//! ┌───────────────────────────────────────────────────┐
//! │ 4. Call change listeners (for storage, logging)   │
//! └───────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};

use crate::core::ParameterBase as CoreParameterBase;
use crate::error::DaqError;
use crate::observable::{Observable, ParameterAny, ParameterBase as ObservableParameterBase};

// =============================================================================
// Type Aliases for Complex Callback/Future Types
// =============================================================================

/// Hardware write callback type.
///
/// A function that takes a value and returns a future that writes to hardware.
/// Used by [`Parameter::connect_to_hardware_write`].
pub type HardwareWriter<T> =
    Arc<dyn Fn(T) -> BoxFuture<'static, Result<(), DaqError>> + Send + Sync>;

/// Hardware read callback type.
///
/// A function that returns a future that reads from hardware.
/// Used by [`Parameter::connect_to_hardware_read`].
pub type HardwareReader<T> = Arc<dyn Fn() -> BoxFuture<'static, Result<T, DaqError>> + Send + Sync>;

/// Change listener callback type.
///
/// A synchronous function called after parameter value changes.
/// Used for side effects like storage logging or dependent parameter updates.
pub type ChangeListener<T> = Arc<dyn Fn(&T) + Send + Sync>;

/// Collection of change listeners with concurrent access.
pub type ChangeListeners<T> = Arc<RwLock<Vec<ChangeListener<T>>>>;

// =============================================================================
// Parameter<T> - Hardware-connected Observable
// =============================================================================

/// Typed parameter with automatic hardware synchronization
///
/// Composes `Observable<T>` with hardware callbacks. When you call `set()`:
/// 1. Writes to hardware (via hardware_writer callback)
/// 2. Updates internal value and notifies subscribers (via Observable)
/// 3. Calls change listeners (for storage, logging, etc.)
///
/// # Architecture
///
/// ```text
/// Parameter<T>
///   ├─ inner: Observable<T>        (subscriptions, validation, metadata)
///   ├─ hardware_writer: Option<F>  (writes to device)
///   ├─ hardware_reader: Option<F>  (reads from device)
///   └─ change_listeners: Vec<F>    (side effects: storage, logging)
/// ```
///
/// # Type Requirements
///
/// T must implement:
/// - Clone: For distributing values to subscribers
/// - Send + Sync: For thread-safe access
/// - PartialEq: For change detection
/// - Debug: For logging and error messages
/// - 'static: Required for tokio::sync::watch
#[derive(Clone)]
pub struct Parameter<T>
where
    T: Clone + Send + Sync + PartialEq + Debug + 'static,
{
    /// Base reactive primitive (handles watch channels, validation, metadata)
    inner: Observable<T>,

    /// Hardware write function (optional)
    ///
    /// When set, calling `set()` will write to hardware before updating
    /// the internal value. Function should return error if write fails.
    hardware_writer: Option<HardwareWriter<T>>,

    /// Hardware read function (optional)
    ///
    /// When set, calling `read_from_hardware()` will fetch the current
    /// hardware value and update the internal value.
    hardware_reader: Option<HardwareReader<T>>,

    /// Change listeners (called after value changes)
    ///
    /// Useful for side effects like updating dependent parameters or
    /// logging changes to storage. These are called AFTER Observable
    /// has notified all subscribers.
    change_listeners: ChangeListeners<T>,
}

impl<T> Parameter<T>
where
    T: Clone + Send + Sync + PartialEq + Debug + 'static,
{
    /// Create new parameter with initial value
    pub fn new(name: impl Into<String>, initial: T) -> Self {
        let inner = Observable::new(name, initial);

        Self {
            inner,
            hardware_writer: None,
            hardware_reader: None,
            change_listeners: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Set parameter description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.inner = self.inner.with_description(description);
        self
    }

    /// Set parameter unit
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.inner = self.inner.with_units(unit);
        self
    }

    /// Set numeric range constraints
    pub fn with_range(mut self, min: T, max: T) -> Self
    where
        T: PartialOrd,
    {
        self.inner = self.inner.with_range(min, max);
        self
    }

    /// Set the dtype for this parameter (for GUI introspection).
    pub fn with_dtype(self, dtype: impl Into<String>) -> Self {
        self.inner.with_metadata(|m| m.dtype = dtype.into());
        self
    }

    /// Modify metadata through a closure (for advanced constraint population).
    pub fn with_metadata<F>(&self, f: F)
    where
        F: FnOnce(&mut crate::observable::ObservableMetadata),
    {
        self.inner.with_metadata(f);
    }

    /// Set discrete choice constraints
    pub fn with_choices(mut self, choices: Vec<T>) -> Self
    where
        T: PartialEq,
    {
        let choices_clone = choices.clone();
        self.inner = self.inner.with_validator(move |value| {
            if choices_clone.iter().any(|c| c == value) {
                Ok(())
            } else {
                Err(DaqError::ParameterInvalidChoice.into())
            }
        });
        self
    }

    /// Set custom validation function
    pub fn with_validator(
        mut self,
        validator: impl Fn(&T) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.inner = self.inner.with_validator(validator);
        self
    }

    /// Make parameter read-only
    pub fn read_only(mut self) -> Self {
        self.inner = self.inner.read_only();
        self
    }

    /// Connect hardware write function
    ///
    /// After calling this, `set()` will write to hardware before updating
    /// the internal value. If hardware write fails, value is not updated.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// exposure.connect_to_hardware_write(|val| {
    ///     camera.set_exposure(val)
    /// });
    /// ```
    pub fn connect_to_hardware_write(
        &mut self,
        writer: impl Fn(T) -> BoxFuture<'static, Result<(), DaqError>> + Send + Sync + 'static,
    ) {
        self.hardware_writer = Some(Arc::new(writer));
    }

    /// Connect hardware read function
    ///
    /// After calling this, `read_from_hardware()` will fetch the current
    /// hardware value and update the parameter.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// exposure.connect_to_hardware_read(|| {
    ///     camera.get_exposure()
    /// });
    /// ```
    pub fn connect_to_hardware_read(
        &mut self,
        reader: impl Fn() -> BoxFuture<'static, Result<T, DaqError>> + Send + Sync + 'static,
    ) {
        self.hardware_reader = Some(Arc::new(reader));
    }

    /// Connect both hardware read and write functions
    pub fn connect_to_hardware(
        &mut self,
        writer: impl Fn(T) -> BoxFuture<'static, Result<(), DaqError>> + Send + Sync + 'static,
        reader: impl Fn() -> BoxFuture<'static, Result<T, DaqError>> + Send + Sync + 'static,
    ) {
        self.connect_to_hardware_write(writer);
        self.connect_to_hardware_read(reader);
    }

    /// Add change listener (called after value changes)
    ///
    /// Useful for side effects like updating dependent parameters,
    /// logging to storage, or triggering recalculations.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// exposure.add_change_listener(|val| {
    ///     log::info!("Exposure changed to: {} ms", val);
    /// });
    /// ```
    pub async fn add_change_listener(&self, listener: impl Fn(&T) + Send + Sync + 'static) {
        let mut listeners = self.change_listeners.write().await;
        listeners.push(Arc::new(listener));
    }

    /// Get current value (delegates to Observable)
    pub fn get(&self) -> T {
        self.inner.get()
    }

    /// Set value (validates, writes to hardware if connected, notifies subscribers)
    ///
    /// This is the main method for changing parameter values. It:
    /// 1. Validates against constraints (via Observable) - BEFORE hardware write
    /// 2. Writes to hardware (if connected)
    /// 3. Updates internal value and notifies subscribers (via Observable)
    /// 4. Calls change listeners
    ///
    /// Returns error if validation fails or hardware write fails.
    ///
    /// # Safety
    /// Validation is performed BEFORE writing to hardware to prevent
    /// driving the device to an invalid state if validation would fail.
    pub async fn set(&self, value: T) -> Result<()> {
        // Step 1: Validate BEFORE hardware write to prevent invalid device states
        // This ensures we don't write to hardware if validation will fail
        self.inner.validate(&value)?;

        // Step 2: Write to hardware if connected (only after validation passes)
        if let Some(writer) = &self.hardware_writer {
            writer(value.clone()).await?;
        }

        // Step 3: Update Observable (skips validation since already done, notifies subscribers)
        // Using set_unchecked since we already validated above
        self.inner.set_unchecked(value.clone());

        // Step 4: Call change listeners (AFTER Observable update)
        let listeners = self.change_listeners.read().await;
        for listener in listeners.iter() {
            listener(&value);
        }

        Ok(())
    }

    /// Read current value from hardware and update parameter
    ///
    /// Only works if hardware reader is connected. Does NOT validate
    /// (assumes hardware value is valid).
    pub async fn read_from_hardware(&self) -> Result<()> {
        let reader = self
            .hardware_reader
            .as_ref()
            .ok_or_else(|| DaqError::ParameterNoHardwareReader)?;

        let value = reader().await?;

        // Update Observable without validation (hardware is source of truth)
        self.inner.set_unchecked(value.clone());

        // Call change listeners
        let listeners = self.change_listeners.read().await;
        for listener in listeners.iter() {
            listener(&value);
        }

        Ok(())
    }

    /// Subscribe to value changes (delegates to Observable)
    ///
    /// Returns a watch receiver that notifies whenever the value changes.
    /// Multiple subscribers can observe independently.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut rx = exposure.subscribe();
    /// tokio::spawn(async move {
    ///     while rx.changed().await.is_ok() {
    ///         let value = *rx.borrow();
    ///         update_gui_widget(value);
    ///     }
    /// });
    /// ```
    pub fn subscribe(&self) -> watch::Receiver<T> {
        self.inner.subscribe()
    }

    /// Get parameter metadata (delegates to Observable)
    pub fn name(&self) -> String {
        self.inner.name()
    }

    /// Get parameter description (delegates to Observable)
    pub fn description(&self) -> Option<String> {
        self.inner.metadata().description
    }

    /// Get parameter unit of measurement (delegates to Observable)
    pub fn unit(&self) -> Option<String> {
        self.inner.metadata().units
    }

    /// Check if parameter is read-only (delegates to Observable)
    pub fn is_read_only(&self) -> bool {
        self.inner.metadata().read_only
    }

    /// Get direct access to inner Observable (for advanced use)
    pub fn inner(&self) -> &Observable<T> {
        &self.inner
    }
}

// =============================================================================
// ParameterBase Implementation (for dynamic collections)
// =============================================================================

impl<T> CoreParameterBase for Parameter<T>
where
    T: Clone + Send + Sync + PartialEq + Debug + Serialize + for<'de> Deserialize<'de> + 'static,
{
    fn name(&self) -> String {
        self.inner.name()
    }

    fn value_json(&self) -> serde_json::Value {
        serde_json::to_value(self.get()).unwrap_or(serde_json::Value::Null)
    }

    fn set_json(&mut self, value: serde_json::Value) -> Result<()> {
        let typed_value: T = serde_json::from_value(value)?;
        futures::executor::block_on(self.set(typed_value))
    }

    fn constraints_json(&self) -> serde_json::Value {
        let metadata = self.inner.metadata();
        let mut constraints = serde_json::Map::new();

        if let Some(min) = metadata.min_value {
            constraints.insert("min".to_string(), serde_json::json!(min));
        }
        if let Some(max) = metadata.max_value {
            constraints.insert("max".to_string(), serde_json::json!(max));
        }
        if !metadata.enum_values.is_empty() {
            constraints.insert(
                "enum_values".to_string(),
                serde_json::json!(metadata.enum_values),
            );
        }
        if !metadata.dtype.is_empty() {
            constraints.insert("dtype".to_string(), serde_json::json!(metadata.dtype));
        }

        if constraints.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::Object(constraints)
        }
    }
}

impl<T> ParameterAny for Parameter<T>
where
    T: Clone + Send + Sync + PartialEq + Debug + Serialize + for<'de> Deserialize<'de> + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn value_as_f64(&self) -> Option<f64> {
        self.as_any()
            .downcast_ref::<Parameter<f64>>()
            .map(|p| p.get())
    }

    fn value_as_bool(&self) -> Option<bool> {
        self.as_any()
            .downcast_ref::<Parameter<bool>>()
            .map(|p| p.get())
    }

    fn value_as_string(&self) -> Option<String> {
        self.as_any()
            .downcast_ref::<Parameter<String>>()
            .map(|p| p.get())
    }

    fn value_as_i64(&self) -> Option<i64> {
        self.as_any()
            .downcast_ref::<Parameter<i64>>()
            .map(|p| p.get())
    }
}

impl<T> ObservableParameterBase for Parameter<T>
where
    T: Clone + Send + Sync + PartialEq + Debug + Serialize + for<'de> Deserialize<'de> + 'static,
{
    fn name(&self) -> String {
        self.inner.name()
    }

    fn get_json(&self) -> Result<serde_json::Value> {
        self.inner.get_json()
    }

    fn set_json(&self, value: serde_json::Value) -> Result<()> {
        let typed_value: T = serde_json::from_value(value)?;
        futures::executor::block_on(self.set(typed_value))
    }

    fn metadata(&self) -> crate::observable::ObservableMetadata {
        self.inner.metadata()
    }

    fn has_subscribers(&self) -> bool {
        self.inner.has_subscribers()
    }

    fn subscriber_count(&self) -> usize {
        self.inner.subscriber_count()
    }
}

// =============================================================================
// Parameter Builder (Fluent API)
// =============================================================================

/// Builder for creating parameters with fluent API
///
/// Provides a chainable interface for constructing parameters with
/// optional metadata and constraints. More ergonomic than calling
/// individual setter methods on `Parameter`.
///
/// # Example
///
/// ```rust,ignore
/// let param = ParameterBuilder::new("wavelength", 532.0)
///     .description("Laser wavelength")
///     .unit("nm")
///     .range(400.0, 1000.0)
///     .build();
/// ```
pub struct ParameterBuilder<T>
where
    T: Clone + Send + Sync + PartialEq + Debug + 'static,
{
    name: String,
    initial: T,
    description: Option<String>,
    unit: Option<String>,
    min: Option<T>,
    max: Option<T>,
    choices: Option<Vec<T>>,
    read_only: bool,
}

impl<T> ParameterBuilder<T>
where
    T: Clone + Send + Sync + PartialEq + Debug + 'static,
{
    /// Create a new parameter builder.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique parameter identifier (e.g., "exposure_ms")
    /// * `initial` - Initial parameter value
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let builder = ParameterBuilder::new("gain", 1.0);
    /// ```
    pub fn new(name: impl Into<String>, initial: T) -> Self {
        Self {
            name: name.into(),
            initial,
            description: None,
            unit: None,
            min: None,
            max: None,
            choices: None,
            read_only: false,
        }
    }

    /// Set parameter description.
    ///
    /// Human-readable description for GUI tooltips and documentation.
    /// Returns `self` for method chaining.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set parameter unit of measurement.
    ///
    /// Unit string displayed in GUI labels (e.g., "ms", "mW", "degrees").
    /// Returns `self` for method chaining.
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Set numeric range constraints.
    ///
    /// Values will be validated against `min <= value <= max`.
    /// Returns `self` for method chaining.
    ///
    /// # Arguments
    ///
    /// * `min` - Minimum allowed value (inclusive)
    /// * `max` - Maximum allowed value (inclusive)
    pub fn range(mut self, min: T, max: T) -> Self
    where
        T: PartialOrd,
    {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    /// Set discrete choice constraints.
    ///
    /// Values must match one of the provided choices exactly.
    /// Returns `self` for method chaining.
    ///
    /// # Arguments
    ///
    /// * `choices` - List of valid parameter values
    pub fn choices(mut self, choices: Vec<T>) -> Self {
        self.choices = Some(choices);
        self
    }

    /// Make parameter read-only.
    ///
    /// Read-only parameters reject `set()` calls with an error.
    /// Useful for computed values or hardware-reported parameters.
    /// Returns `self` for method chaining.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }
}

impl<T> ParameterBuilder<T>
where
    T: Clone + Send + Sync + PartialEq + PartialOrd + Debug + 'static,
{
    /// Build the parameter.
    ///
    /// Constructs the final `Parameter<T>` instance from the builder
    /// configuration. Consumes the builder.
    ///
    /// # Returns
    ///
    /// Configured parameter ready for use
    pub fn build(self) -> Parameter<T> {
        let mut param = Parameter::new(self.name, self.initial);

        if let Some(desc) = self.description {
            param = param.with_description(desc);
        }

        if let Some(unit) = self.unit {
            param = param.with_unit(unit);
        }

        if let (Some(min), Some(max)) = (self.min, self.max) {
            param = param.with_range(min, max);
        }

        if let Some(choices) = self.choices {
            param = param.with_choices(choices);
        }

        if self.read_only {
            param = param.read_only();
        }

        param
    }
}

// =============================================================================
// Type-specific Parameter Extensions (Introspectable Constraints)
// =============================================================================
//
// These methods delegate to Observable<T> for actual implementation.
// See observable.rs for detailed documentation on constraint behavior.
//
// Phase 2 (bd-cdh5.2): Added to support rich GUI widgets that read constraint
// metadata via gRPC ListParameters.

impl Parameter<f64> {
    /// Set numeric range constraints with introspectable metadata for GUI.
    ///
    /// Delegates to [`Observable<f64>::with_range_introspectable()`] which:
    /// - Sets `metadata.min_value` and `metadata.max_value` for GUI introspection
    /// - Sets `metadata.dtype = "float"`
    /// - Adds a validator that rejects values outside `[min, max]`
    /// - Rejects NaN and Infinity values
    ///
    /// The GUI renders a Slider widget when both bounds are present.
    ///
    /// # Panics
    ///
    /// Panics if `min` or `max` is non-finite, or if `min > max`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let exposure = Parameter::new("exposure_ms", 100.0)
    ///     .with_unit("ms")
    ///     .with_range_introspectable(1.0, 10000.0);  // GUI renders as Slider
    /// ```
    ///
    /// # See Also
    ///
    /// - [`Observable<f64>::with_range_introspectable()`] - Full documentation
    /// - [`Parameter::with_range()`] - Validation only, no GUI introspection
    pub fn with_range_introspectable(mut self, min: f64, max: f64) -> Self {
        self.inner = self.inner.with_range_introspectable(min, max);
        self
    }
}

impl Parameter<i64> {
    /// Set numeric range constraints with introspectable metadata for GUI.
    ///
    /// Delegates to [`Observable<i64>::with_range_introspectable()`] which:
    /// - Sets `metadata.min_value` and `metadata.max_value` for GUI introspection
    /// - Sets `metadata.dtype = "int"`
    /// - Adds a validator that rejects values outside `[min, max]`
    ///
    /// The GUI renders a Slider widget when both bounds are present.
    ///
    /// # Integer Precision Note
    ///
    /// Large i64 values (outside ±2^53) may lose precision in the GUI metadata
    /// since they are stored as f64. Runtime validation uses exact i64 values.
    ///
    /// # Panics
    ///
    /// Panics if `min > max`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let gain = Parameter::new("gain", 1i64)
    ///     .with_range_introspectable(0, 100);  // GUI renders as Slider
    /// ```
    ///
    /// # See Also
    ///
    /// - [`Observable<i64>::with_range_introspectable()`] - Full documentation
    /// - [`Parameter::with_range()`] - Validation only, no GUI introspection
    pub fn with_range_introspectable(mut self, min: i64, max: i64) -> Self {
        self.inner = self.inner.with_range_introspectable(min, max);
        self
    }
}

impl Parameter<String> {
    /// Set choice constraints with introspectable metadata for GUI.
    ///
    /// Delegates to [`Observable<String>::with_choices_introspectable()`] which:
    /// - Sets `metadata.enum_values` with the allowed choices
    /// - Sets `metadata.dtype = "enum"` (per proto contract, not "string")
    /// - Adds a validator that rejects values not in the choices list
    ///
    /// The GUI renders a ComboBox/Dropdown widget when `enum_values` is non-empty.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let fan_speed = Parameter::new("fan_speed", "auto".to_string())
    ///     .with_choices_introspectable(vec![
    ///         "off".into(),
    ///         "low".into(),
    ///         "medium".into(),
    ///         "high".into(),
    ///         "auto".into(),
    ///     ]);  // GUI renders as ComboBox
    /// ```
    ///
    /// # See Also
    ///
    /// - [`Observable<String>::with_choices_introspectable()`] - Full documentation
    /// - [`Parameter::with_choices()`] - Validation only, no GUI introspection
    pub fn with_choices_introspectable(mut self, choices: Vec<String>) -> Self {
        self.inner = self.inner.with_choices_introspectable(choices);
        self
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parameter_basic() {
        let param = Parameter::new("test", 42.0);
        assert_eq!(param.get(), 42.0);

        param.set(100.0).await.unwrap();
        assert_eq!(param.get(), 100.0);
    }

    #[tokio::test]
    async fn test_parameter_range_validation() {
        let param = Parameter::new("test", 50.0).with_range(0.0, 100.0);

        assert!(param.set(50.0).await.is_ok());
        assert!(param.set(150.0).await.is_err()); // Out of range
        assert!(param.set(-10.0).await.is_err()); // Out of range
    }

    #[tokio::test]
    async fn test_parameter_choices() {
        let param = Parameter::new("mode", "auto".to_string())
            .with_choices(vec!["auto".to_string(), "manual".to_string()]);

        assert!(param.set("manual".to_string()).await.is_ok());
        assert!(param.set("invalid".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_parameter_read_only() {
        let param = Parameter::new("readonly", 42.0).read_only();

        assert!(param.set(100.0).await.is_err());
        assert_eq!(param.get(), 42.0); // Unchanged
    }

    #[tokio::test]
    async fn test_parameter_hardware_write() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let hardware_value = Arc::new(AtomicU64::new(0));
        let hw_val_clone = hardware_value.clone();

        let mut param = Parameter::new("exposure", 100.0);
        param.connect_to_hardware_write(move |val| {
            let hw = hw_val_clone.clone();
            Box::pin(async move {
                hw.store(val as u64, Ordering::SeqCst);
                Ok(())
            })
        });

        param.set(250.0).await.unwrap();
        assert_eq!(hardware_value.load(Ordering::SeqCst), 250);
    }

    #[tokio::test]
    async fn test_parameter_subscription() {
        let param = Parameter::new("test", 0.0);
        let mut rx = param.subscribe();

        // Initial value
        assert_eq!(*rx.borrow(), 0.0);

        // Change value
        param.set(42.0).await.unwrap();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), 42.0);
    }

    #[tokio::test]
    async fn test_parameter_change_listener() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let listener_called = Arc::new(AtomicU64::new(0));
        let lc_clone = listener_called.clone();

        let param = Parameter::new("test", 0.0);
        param
            .add_change_listener(move |_val| {
                lc_clone.fetch_add(1, Ordering::SeqCst);
            })
            .await;

        param.set(10.0).await.unwrap();
        param.set(20.0).await.unwrap();

        assert_eq!(listener_called.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_parameter_builder() {
        let param = ParameterBuilder::new("exposure", 100.0)
            .description("Camera exposure time")
            .unit("ms")
            .range(1.0, 10000.0)
            .build();

        assert_eq!(param.name(), "exposure");
        assert_eq!(
            param.description(),
            Some("Camera exposure time".to_string())
        );
        assert_eq!(param.unit(), Some("ms".to_string()));
        assert_eq!(param.get(), 100.0);
    }

    /// Critical safety test: Validation MUST happen BEFORE hardware write.
    /// This prevents driving hardware to an invalid state if validation fails.
    /// Regression test for bd-jnfu.2.
    #[tokio::test]
    async fn test_parameter_validates_before_hardware_write() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

        let hardware_write_called = Arc::new(AtomicBool::new(false));
        let hardware_value = Arc::new(AtomicU64::new(0));
        let hw_called_clone = hardware_write_called.clone();
        let hw_val_clone = hardware_value.clone();

        // Create parameter with range validation (0.0 to 100.0)
        let mut param = Parameter::new("exposure", 50.0).with_range(0.0, 100.0);

        // Connect hardware writer that tracks if it was called
        param.connect_to_hardware_write(move |val| {
            let hw_called = hw_called_clone.clone();
            let hw_val = hw_val_clone.clone();
            Box::pin(async move {
                hw_called.store(true, Ordering::SeqCst);
                hw_val.store(val as u64, Ordering::SeqCst);
                Ok(())
            })
        });

        // Try to set an INVALID value (150.0 is outside range 0-100)
        let result = param.set(150.0).await;

        // Validation should fail
        assert!(result.is_err(), "Setting out-of-range value should fail");

        // CRITICAL: Hardware write should NOT have been called
        assert!(
            !hardware_write_called.load(Ordering::SeqCst),
            "Hardware write should NOT be called when validation fails"
        );

        // Value should remain unchanged
        assert_eq!(
            param.get(),
            50.0,
            "Parameter value should not change on failed set"
        );

        // Now try a VALID value
        hardware_write_called.store(false, Ordering::SeqCst);
        let result = param.set(75.0).await;

        // Should succeed
        assert!(result.is_ok(), "Setting valid value should succeed");

        // Hardware should have been written
        assert!(
            hardware_write_called.load(Ordering::SeqCst),
            "Hardware write should be called for valid values"
        );
        assert_eq!(hardware_value.load(Ordering::SeqCst), 75);
        assert_eq!(param.get(), 75.0);
    }

    /// Test that read-only parameters don't trigger hardware writes
    #[tokio::test]
    async fn test_parameter_readonly_no_hardware_write() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let hardware_write_called = Arc::new(AtomicBool::new(false));
        let hw_called_clone = hardware_write_called.clone();

        let mut param = Parameter::new("readonly_param", 42.0).read_only();

        param.connect_to_hardware_write(move |_val| {
            let hw_called = hw_called_clone.clone();
            Box::pin(async move {
                hw_called.store(true, Ordering::SeqCst);
                Ok(())
            })
        });

        // Try to set value on read-only parameter
        let result = param.set(100.0).await;

        // Should fail
        assert!(result.is_err());

        // Hardware should NOT have been written
        assert!(
            !hardware_write_called.load(Ordering::SeqCst),
            "Hardware write should NOT be called for read-only parameter"
        );
    }
}
