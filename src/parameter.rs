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
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::parameter::Parameter;
//! use futures::future::BoxFuture;
//!
//! // Create parameter with constraints
//! let mut exposure = Parameter::new("exposure_ms")
//!     .with_initial(100.0)
//!     .with_range(1.0, 10000.0)
//!     .with_unit("ms")
//!     .build();
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

use anyhow::Result;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};

use crate::core::ParameterBase;
use crate::error::DaqError;
use crate::observable::Observable;

// =============================================================================
// Constraints - DEPRECATED: Use Observable validators instead
// =============================================================================

/// Parameter constraints for validation
///
/// **DEPRECATED**: This enum is kept for backwards compatibility but new code
/// should use `Observable::with_range()` or `Observable::with_validator()` directly.
///
/// # Migration
///
/// ```rust,ignore
/// // Old:
/// Parameter::new("x", 0.0).with_range(0.0, 100.0)
///
/// // New:
/// Observable::new("x", 0.0).with_range(0.0, 100.0)
/// ```
#[derive(Clone, Serialize, Deserialize, Default)]
#[deprecated(
    since = "0.5.0",
    note = "Use Observable::with_range() or with_validator() instead"
)]
pub enum Constraints<T> {
    /// No constraints - all values accepted.
    #[default]
    None,

    /// Numeric range constraint (inclusive bounds).
    ///
    /// Values must satisfy: `min <= value <= max`.
    /// Commonly used for exposure times, positions, power levels.
    Range {
        /// Minimum allowed value (inclusive).
        min: T,
        /// Maximum allowed value (inclusive).
        max: T,
    },

    /// Discrete choice constraint.
    ///
    /// Value must match one of the provided choices exactly.
    /// Useful for enumerated settings like trigger modes or filters.
    Choices(Vec<T>),

    /// Custom validation function (not serializable).
    ///
    /// Provides arbitrary validation logic. Cannot be serialized,
    /// so this variant is skipped during JSON encoding.
    #[serde(skip)]
    Custom(Arc<dyn Fn(&T) -> Result<()> + Send + Sync>),
}

impl<T: PartialOrd + Clone + Debug> Constraints<T> {
    /// Validate value against constraints
    pub fn validate(&self, value: &T) -> Result<()> {
        match self {
            Constraints::None => Ok(()),

            Constraints::Range { min, max } => {
                if value < min || value > max {
                    Err(DaqError::ParameterInvalidChoice.into())
                } else {
                    Ok(())
                }
            }

            Constraints::Choices(choices) => {
                if choices.iter().any(|c| c == value) {
                    Ok(())
                } else {
                    Err(DaqError::ParameterInvalidChoice.into())
                }
            }

            Constraints::Custom(validator) => validator(value),
        }
    }
}

impl<T: Debug> std::fmt::Debug for Constraints<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Constraints::None => write!(f, "None"),
            Constraints::Range { min, max } => f
                .debug_struct("Range")
                .field("min", min)
                .field("max", max)
                .finish(),
            Constraints::Choices(choices) => f.debug_tuple("Choices").field(choices).finish(),
            Constraints::Custom(_) => write!(f, "Custom(<function>)"),
        }
    }
}

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
    hardware_writer:
        Option<Arc<dyn Fn(T) -> BoxFuture<'static, Result<(), DaqError>> + Send + Sync>>,

    /// Hardware read function (optional)
    ///
    /// When set, calling `read_from_hardware()` will fetch the current
    /// hardware value and update the internal value.
    hardware_reader: Option<Arc<dyn Fn() -> BoxFuture<'static, Result<T, DaqError>> + Send + Sync>>,

    /// Change listeners (called after value changes)
    ///
    /// Useful for side effects like updating dependent parameters or
    /// logging changes to storage. These are called AFTER Observable
    /// has notified all subscribers.
    change_listeners: Arc<RwLock<Vec<Arc<dyn Fn(&T) + Send + Sync>>>>,
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
    /// 1. Validates against constraints (via Observable)
    /// 2. Writes to hardware (if connected)
    /// 3. Updates internal value and notifies subscribers (via Observable)
    /// 4. Calls change listeners
    ///
    /// Returns error if validation fails or hardware write fails.
    pub async fn set(&self, value: T) -> Result<()> {
        // Step 1: Write to hardware if connected (BEFORE Observable update)
        if let Some(writer) = &self.hardware_writer {
            writer(value.clone()).await?;
        }

        // Step 2: Update Observable (validates and notifies subscribers)
        self.inner.set(value.clone())?;

        // Step 3: Call change listeners (AFTER Observable update)
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
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Get parameter description (delegates to Observable)
    pub fn description(&self) -> Option<&str> {
        self.inner.metadata().description.as_deref()
    }

    /// Get parameter unit of measurement (delegates to Observable)
    pub fn unit(&self) -> Option<&str> {
        self.inner.metadata().units.as_deref()
    }

    /// Check if parameter is read-only (delegates to Observable)
    pub fn is_read_only(&self) -> bool {
        self.inner.metadata().read_only
    }

    /// Get parameter constraints (DEPRECATED: Observable uses validators)
    #[deprecated(since = "0.5.0", note = "Use Observable metadata instead")]
    pub fn constraints(&self) -> Constraints<T> {
        Constraints::None // Legacy compatibility
    }

    /// Get direct access to inner Observable (for advanced use)
    pub fn inner(&self) -> &Observable<T> {
        &self.inner
    }
}

// =============================================================================
// ParameterBase Implementation (for dynamic collections)
// =============================================================================

impl<T> ParameterBase for Parameter<T>
where
    T: Clone + Send + Sync + PartialEq + Debug + Serialize + for<'de> Deserialize<'de> + 'static,
{
    fn name(&self) -> &str {
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
        serde_json::Value::Null // Observable uses validators, not serializable constraints
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
// Tests
// =============================================================================

#[cfg(test)]
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
        assert_eq!(param.description(), Some("Camera exposure time"));
        assert_eq!(param.unit(), Some("ms"));
        assert_eq!(param.get(), 100.0);
    }
}
