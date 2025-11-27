//! Parameter<T> - Declarative parameter management (ScopeFoundry pattern)
//!
//! Inspired by ScopeFoundry's LoggedQuantity, this module provides a unified
//! abstraction for instrument parameters that automatically synchronizes:
//! - GUI widgets (via watch channels)
//! - Hardware devices (via callbacks)
//! - Storage (via change listeners)
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::parameter::Parameter;
//!
//! // Create parameter with constraints
//! let mut exposure = Parameter::new("exposure_ms")
//!     .with_initial(100.0)
//!     .with_range(1.0, 10000.0)
//!     .with_unit("ms")
//!     .build();
//!
//! // Connect to hardware
//! exposure.connect_to_hardware(
//!     |val| camera.set_exposure(val),  // Write function
//!     || camera.get_exposure(),         // Read function
//! );
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
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::{watch, RwLock};

use crate::core::ParameterBase;
use crate::error::DaqError;

// =============================================================================
// Constraints
// =============================================================================

/// Parameter constraints for validation
#[derive(Clone, Serialize, Deserialize)]
#[derive(Default)]
pub enum Constraints<T> {
    /// No constraints
    #[default]
    None,

    /// Numeric range (min, max)
    Range { min: T, max: T },

    /// Allowed discrete values
    Choices(Vec<T>),

    /// Custom validation function (not serializable)
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
// Parameter<T>
// =============================================================================

/// Typed parameter with automatic synchronization
///
/// Provides declarative parameter management inspired by ScopeFoundry's
/// LoggedQuantity. Parameters automatically synchronize between:
/// - GUI (via watch channels)
/// - Hardware (via read/write callbacks)
/// - Storage (via change listeners)
///
/// # Type Requirements
///
/// T must implement:
/// - Clone: For distributing values to subscribers
/// - Send + Sync: For thread-safe access
/// - PartialEq: For change detection
/// - PartialOrd: For range validation
/// - Debug: For logging and error messages
pub struct Parameter<T>
where
    T: Clone + Send + Sync + PartialEq + PartialOrd + Debug,
{
    /// Parameter name (unique identifier)
    name: String,

    /// Parameter description (for GUI tooltips)
    description: Option<String>,

    /// Unit of measurement (e.g., "ms", "mW", "nm")
    unit: Option<String>,

    /// Current value (observable via watch channel)
    value_rx: watch::Receiver<T>,
    value_tx: watch::Sender<T>,

    /// Hardware write function (optional)
    ///
    /// When set, calling `set()` will write to hardware before updating
    /// the internal value. Function should return error if write fails.
    hardware_writer: Option<Arc<dyn Fn(T) -> Result<()> + Send + Sync>>,

    /// Hardware read function (optional)
    ///
    /// When set, calling `read_from_hardware()` will fetch the current
    /// hardware value and update the internal value.
    hardware_reader: Option<Arc<dyn Fn() -> Result<T> + Send + Sync>>,

    /// Validation constraints
    constraints: Constraints<T>,

    /// Change listeners (called after value changes)
    ///
    /// Useful for side effects like updating dependent parameters or
    /// logging changes to storage.
    change_listeners: Arc<RwLock<Vec<Arc<dyn Fn(&T) + Send + Sync>>>>,

    /// Read-only flag (prevents set() from modifying value)
    read_only: bool,
}

impl<T> Parameter<T>
where
    T: Clone + Send + Sync + PartialEq + PartialOrd + Debug + 'static,
{
    /// Create new parameter with initial value
    pub fn new(name: impl Into<String>, initial: T) -> Self {
        let (value_tx, value_rx) = watch::channel(initial);

        Self {
            name: name.into(),
            description: None,
            unit: None,
            value_rx,
            value_tx,
            hardware_writer: None,
            hardware_reader: None,
            constraints: Constraints::None,
            change_listeners: Arc::new(RwLock::new(Vec::new())),
            read_only: false,
        }
    }

    /// Set parameter description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set parameter unit
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Set numeric range constraints
    pub fn with_range(mut self, min: T, max: T) -> Self
    where
        T: PartialOrd,
    {
        self.constraints = Constraints::Range { min, max };
        self
    }

    /// Set discrete choice constraints
    pub fn with_choices(mut self, choices: Vec<T>) -> Self {
        self.constraints = Constraints::Choices(choices);
        self
    }

    /// Set custom validation function
    pub fn with_validator(
        mut self,
        validator: impl Fn(&T) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.constraints = Constraints::Custom(Arc::new(validator));
        self
    }

    /// Make parameter read-only
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
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
        writer: impl Fn(T) -> Result<()> + Send + Sync + 'static,
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
        reader: impl Fn() -> Result<T> + Send + Sync + 'static,
    ) {
        self.hardware_reader = Some(Arc::new(reader));
    }

    /// Connect both hardware read and write functions
    pub fn connect_to_hardware(
        &mut self,
        writer: impl Fn(T) -> Result<()> + Send + Sync + 'static,
        reader: impl Fn() -> Result<T> + Send + Sync + 'static,
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

    /// Get current value
    pub fn get(&self) -> T {
        self.value_rx.borrow().clone()
    }

    /// Set value (validates, writes to hardware if connected, notifies subscribers)
    ///
    /// This is the main method for changing parameter values. It:
    /// 1. Validates against constraints
    /// 2. Writes to hardware (if connected)
    /// 3. Updates internal value
    /// 4. Notifies all subscribers via watch channel
    /// 5. Calls change listeners
    ///
    /// Returns error if validation fails or hardware write fails.
    pub async fn set(&mut self, value: T) -> Result<()> {
        if self.read_only {
            return Err(DaqError::ParameterReadOnly.into());
        }

        // Validate against constraints
        self.constraints.validate(&value)?;

        // Write to hardware if connected
        if let Some(writer) = &self.hardware_writer {
            writer(value.clone())?;
        }

        // Update internal value (notifies subscribers)
        self.value_tx
            .send(value.clone())
            .map_err(|_| DaqError::ParameterNoSubscribers)?;

        // Call change listeners
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
    pub async fn read_from_hardware(&mut self) -> Result<()> {
        let reader = self
            .hardware_reader
            .as_ref()
            .ok_or_else(|| DaqError::ParameterNoHardwareReader)?;

        let value = reader()?;

        // Update internal value without validation
        self.value_tx
            .send(value.clone())
            .map_err(|_| DaqError::ParameterNoSubscribers)?;

        // Call change listeners
        let listeners = self.change_listeners.read().await;
        for listener in listeners.iter() {
            listener(&value);
        }

        Ok(())
    }

    /// Subscribe to value changes (for GUI widgets)
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
        self.value_rx.clone()
    }

    /// Get parameter metadata
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn constraints(&self) -> &Constraints<T> {
        &self.constraints
    }
}

// =============================================================================
// ParameterBase Implementation (for dynamic collections)
// =============================================================================

impl<T> ParameterBase for Parameter<T>
where
    T: Clone
        + Send
        + Sync
        + PartialEq
        + PartialOrd
        + Debug
        + Serialize
        + for<'de> Deserialize<'de>
        + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn value_json(&self) -> serde_json::Value {
        serde_json::to_value(self.get()).unwrap_or(serde_json::Value::Null)
    }

    fn set_json(&mut self, value: serde_json::Value) -> Result<()> {
        let typed_value: T = serde_json::from_value(value)?;
        futures::executor::block_on(self.set(typed_value))
    }

    fn constraints_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.constraints).unwrap_or(serde_json::Value::Null)
    }
}

// =============================================================================
// Parameter Builder (Fluent API)
// =============================================================================

/// Builder for creating parameters with fluent API
pub struct ParameterBuilder<T>
where
    T: Clone + Send + Sync + PartialEq + PartialOrd + Debug,
{
    name: String,
    initial: T,
    description: Option<String>,
    unit: Option<String>,
    constraints: Constraints<T>,
    read_only: bool,
}

impl<T> ParameterBuilder<T>
where
    T: Clone + Send + Sync + PartialEq + PartialOrd + Debug + 'static,
{
    pub fn new(name: impl Into<String>, initial: T) -> Self {
        Self {
            name: name.into(),
            initial,
            description: None,
            unit: None,
            constraints: Constraints::None,
            read_only: false,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    pub fn range(mut self, min: T, max: T) -> Self
    where
        T: PartialOrd,
    {
        self.constraints = Constraints::Range { min, max };
        self
    }

    pub fn choices(mut self, choices: Vec<T>) -> Self {
        self.constraints = Constraints::Choices(choices);
        self
    }

    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn build(self) -> Parameter<T> {
        let param = Parameter::new(self.name, self.initial);

        let mut param = match self.description {
            Some(desc) => param.with_description(desc),
            None => param,
        };

        param = match self.unit {
            Some(unit) => param.with_unit(unit),
            None => param,
        };

        param.constraints = self.constraints;
        param.read_only = self.read_only;

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
        let mut param = Parameter::new("test", 42.0);
        assert_eq!(param.get(), 42.0);

        param.set(100.0).await.unwrap();
        assert_eq!(param.get(), 100.0);
    }

    #[tokio::test]
    async fn test_parameter_range_validation() {
        let mut param = Parameter::new("test", 50.0).with_range(0.0, 100.0);

        assert!(param.set(50.0).await.is_ok());
        assert!(param.set(150.0).await.is_err()); // Out of range
        assert!(param.set(-10.0).await.is_err()); // Out of range
    }

    #[tokio::test]
    async fn test_parameter_choices() {
        let mut param = Parameter::new("mode", "auto".to_string())
            .with_choices(vec!["auto".to_string(), "manual".to_string()]);

        assert!(param.set("manual".to_string()).await.is_ok());
        assert!(param.set("invalid".to_string()).await.is_err());
    }

    #[tokio::test]
    async fn test_parameter_read_only() {
        let mut param = Parameter::new("readonly", 42.0).read_only();

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
            hw_val_clone.store(val as u64, Ordering::SeqCst);
            Ok(())
        });

        param.set(250.0).await.unwrap();
        assert_eq!(hardware_value.load(Ordering::SeqCst), 250);
    }

    #[tokio::test]
    async fn test_parameter_subscription() {
        let mut param = Parameter::new("test", 0.0);
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

        let mut param = Parameter::new("test", 0.0);
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
