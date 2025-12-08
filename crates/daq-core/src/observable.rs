//! Observable Parameters
//!
//! Reactive parameter system using `tokio::sync::watch` for multi-subscriber
//! notifications. Inspired by QCodes Parameter and ScopeFoundry LoggedQuantity.
//!
//! # Features
//!
//! - Type-safe observable values with automatic change notifications
//! - Multi-subscriber support (UI, logging, other modules)
//! - Optional validation constraints (min/max/custom)
//! - Metadata (name, units, description)
//! - Serialization support for snapshots
//! - Generic parameter access via ParameterBase trait
//!
//! # Example
//!
//! ```rust,ignore
//! let threshold = Observable::new("high_threshold", 100.0)
//!     .with_units("mW")
//!     .with_range(0.0, 1000.0);
//!
//! // Subscribe to changes
//! let mut rx = threshold.subscribe();
//! tokio::spawn(async move {
//!     while rx.changed().await.is_ok() {
//!         println!("Threshold changed to: {}", *rx.borrow());
//!     }
//! });
//!
//! // Update value (notifies all subscribers)
//! threshold.set(150.0)?;
//! ```

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::watch;

// =============================================================================
// ParameterBase Trait - Generic Parameter Access
// =============================================================================

/// Base trait for all parameters, providing type-erased access to common operations.
///
/// This enables generic parameter access (e.g., from gRPC endpoints) without
/// knowing the concrete parameter type at compile time.
pub trait ParameterBase: Send + Sync {
    /// Get the parameter name
    fn name(&self) -> &str;

    /// Get the current value as JSON
    fn get_json(&self) -> Result<serde_json::Value>;

    /// Set the value from JSON
    fn set_json(&self, value: serde_json::Value) -> Result<()>;

    /// Get the parameter metadata
    fn metadata(&self) -> &ObservableMetadata;

    /// Check if there are any active subscribers
    fn has_subscribers(&self) -> bool;

    /// Get the number of active subscribers
    fn subscriber_count(&self) -> usize;
}

/// Combines ParameterBase with Any for downcasting when concrete type is needed.
///
/// This allows generic parameter access while still enabling type-specific
/// operations when the concrete type is known.
pub trait ParameterAny: ParameterBase {
    /// Get a reference to this parameter as `&dyn Any` for downcasting
    fn as_any(&self) -> &dyn Any;

    /// Get the type name of the parameter value (e.g., "f64", "bool", "String")
    fn type_name(&self) -> &'static str;

    /// Attempt to get the value as f64 (returns None if not f64 type)
    fn value_as_f64(&self) -> Option<f64>;

    /// Attempt to get the value as bool (returns None if not bool type)
    fn value_as_bool(&self) -> Option<bool>;

    /// Attempt to get the value as String (returns None if not String type)
    fn value_as_string(&self) -> Option<String>;

    /// Attempt to get the value as i64 (returns None if not i64 type)
    fn value_as_i64(&self) -> Option<i64>;
}

// =============================================================================
// Observable<T>
// =============================================================================

/// A thread-safe, observable value with change notifications.
///
/// Uses `tokio::sync::watch` internally for efficient multi-subscriber broadcast.
/// Subscribers can wait for changes asynchronously without polling.
pub struct Observable<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// The watch channel sender (holds current value)
    sender: watch::Sender<T>,
    /// Parameter metadata
    metadata: ObservableMetadata,
    /// Optional validation function
    validator: Option<Arc<dyn Fn(&T) -> Result<()> + Send + Sync>>,
}

impl<T: Clone + Send + Sync + 'static> std::fmt::Debug for Observable<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Observable")
            .field("metadata", &self.metadata)
            .field("has_validator", &self.validator.is_some())
            .finish()
    }
}

impl<T: Clone + Send + Sync + 'static> Clone for Observable<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(), // Clones sender (shares same watch channel)
            metadata: self.metadata.clone(),
            validator: self.validator.clone(), // Arc clone (cheap pointer copy)
        }
    }
}

/// Metadata for an observable parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservableMetadata {
    /// Parameter name (unique within module)
    pub name: String,
    /// Human-readable description
    pub description: Option<String>,
    /// Physical units (e.g., "mW", "Hz", "mm")
    pub units: Option<String>,
    /// Whether this parameter is read-only
    pub read_only: bool,
}

impl<T> Observable<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Create a new observable with an initial value.
    pub fn new(name: impl Into<String>, initial_value: T) -> Self {
        let (sender, _) = watch::channel(initial_value);
        Self {
            sender,
            metadata: ObservableMetadata {
                name: name.into(),
                description: None,
                units: None,
                read_only: false,
            },
            validator: None,
        }
    }

    /// Add a description to this observable.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.metadata.description = Some(description.into());
        self
    }

    /// Add units to this observable.
    pub fn with_units(mut self, units: impl Into<String>) -> Self {
        self.metadata.units = Some(units.into());
        self
    }

    /// Mark this observable as read-only.
    pub fn read_only(mut self) -> Self {
        self.metadata.read_only = true;
        self
    }

    /// Add a custom validator function.
    pub fn with_validator<F>(mut self, validator: F) -> Self
    where
        F: Fn(&T) -> Result<()> + Send + Sync + 'static,
    {
        self.validator = Some(Arc::new(validator));
        self
    }

    /// Get the current value (clone).
    pub fn get(&self) -> T {
        self.sender.borrow().clone()
    }

    /// Get the parameter name.
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// Get the metadata.
    pub fn metadata(&self) -> &ObservableMetadata {
        &self.metadata
    }

    /// Set a new value, notifying all subscribers.
    ///
    /// Returns error if:
    /// - Parameter is read-only
    /// - Validation fails
    pub fn set(&self, value: T) -> Result<()> {
        if self.metadata.read_only {
            return Err(anyhow!("Parameter '{}' is read-only", self.metadata.name));
        }

        if let Some(validator) = &self.validator {
            validator(&value)?;
        }

        self.sender.send_replace(value);
        Ok(())
    }

    /// Set value without validation (internal use).
    pub(crate) fn set_unchecked(&self, value: T) {
        self.sender.send_replace(value);
    }

    /// Subscribe to value changes.
    ///
    /// Returns a receiver that can be used to wait for changes:
    /// ```rust,ignore
    /// let mut rx = observable.subscribe();
    /// while rx.changed().await.is_ok() {
    ///     let value = rx.borrow().clone();
    ///     // Handle new value
    /// }
    /// ```
    pub fn subscribe(&self) -> watch::Receiver<T> {
        self.sender.subscribe()
    }

    /// Check if there are any active subscribers.
    pub fn has_subscribers(&self) -> bool {
        self.sender.receiver_count() > 0
    }

    /// Get the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl<T> Observable<T>
where
    T: Clone + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
{
    /// Get the current value as JSON
    pub fn get_json(&self) -> Result<serde_json::Value> {
        let value = self.get();
        serde_json::to_value(&value).map_err(|e| {
            anyhow!(
                "Failed to serialize parameter '{}': {}",
                self.metadata.name,
                e
            )
        })
    }

    /// Set the value from JSON
    pub fn set_json(&self, json_value: serde_json::Value) -> Result<()> {
        let value: T = serde_json::from_value(json_value).map_err(|e| {
            anyhow!(
                "Failed to deserialize parameter '{}': {}. Expected type: {}",
                self.metadata.name,
                e,
                std::any::type_name::<T>()
            )
        })?;
        self.set(value)
    }
}

// Implement ParameterBase for Observable<T> where T supports JSON serialization
impl<T> ParameterBase for Observable<T>
where
    T: Clone + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
{
    fn name(&self) -> &str {
        &self.metadata.name
    }

    fn get_json(&self) -> Result<serde_json::Value> {
        Observable::get_json(self)
    }

    fn set_json(&self, value: serde_json::Value) -> Result<()> {
        Observable::set_json(self, value)
    }

    fn metadata(&self) -> &ObservableMetadata {
        &self.metadata
    }

    fn has_subscribers(&self) -> bool {
        self.sender.receiver_count() > 0
    }

    fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

// Implement ParameterAny for Observable<T>
impl<T> ParameterAny for Observable<T>
where
    T: Clone + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn value_as_f64(&self) -> Option<f64> {
        None // Observable doesn't support type-specific access
    }

    fn value_as_bool(&self) -> Option<bool> {
        None // Observable doesn't support type-specific access
    }

    fn value_as_string(&self) -> Option<String> {
        None // Observable doesn't support type-specific access
    }

    fn value_as_i64(&self) -> Option<i64> {
        None // Observable doesn't support type-specific access
    }
}

// =============================================================================
// Numeric Observable Extensions
// =============================================================================

impl<T> Observable<T>
where
    T: Clone + Send + Sync + PartialOrd + Debug + 'static,
{
    /// Add min/max range validation.
    pub fn with_range(mut self, min: T, max: T) -> Self {
        let min = min.clone();
        let max = max.clone();
        self.validator = Some(Arc::new(move |value: &T| {
            if value < &min || value > &max {
                Err(anyhow!(
                    "Value {:?} out of range [{:?}, {:?}]",
                    value,
                    min,
                    max
                ))
            } else {
                Ok(())
            }
        }));
        self
    }
}

// =============================================================================
// ParameterSet - Collection of Observables
// =============================================================================

/// A collection of observable parameters for a module.
///
/// Provides snapshot and restore functionality for parameter state.
/// Stores parameters as trait objects, enabling generic access without
/// knowing concrete types.
#[derive(Default)]
pub struct ParameterSet {
    /// Named parameters (stored as trait objects for generic access)
    parameters: std::collections::HashMap<String, Box<dyn ParameterAny>>,
}

impl std::fmt::Debug for ParameterSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterSet")
            .field(
                "parameters",
                &format!("{} parameters", self.parameters.len()),
            )
            .field("names", &self.names())
            .finish()
    }
}

impl ParameterSet {
    /// Create a new empty parameter set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register any parameter-like object that implements `ParameterAny`.
    pub fn register<P>(&mut self, parameter: P)
    where
        P: ParameterAny + 'static,
    {
        let name = parameter.name().to_string();
        self.parameters.insert(name, Box::new(parameter));
    }

    /// Get a parameter by name with specific concrete type (requires downcasting).
    pub fn get_typed<P>(&self, name: &str) -> Option<&P>
    where
        P: ParameterAny + 'static,
    {
        self.parameters
            .get(name)
            .and_then(|p| p.as_any().downcast_ref::<P>())
    }

    /// Get a parameter by name as a trait object (generic access).
    pub fn get(&self, name: &str) -> Option<&dyn ParameterBase> {
        self.parameters
            .get(name)
            .map(|p| p.as_ref() as &dyn ParameterBase)
    }

    /// Iterate over all parameters as trait objects.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &dyn ParameterBase)> {
        self.parameters
            .iter()
            .map(|(name, param)| (name.as_str(), param.as_ref() as &dyn ParameterBase))
    }

    /// Get all parameters as a vector of trait objects.
    pub fn parameters(&self) -> Vec<&dyn ParameterBase> {
        self.parameters
            .values()
            .map(|p| p.as_ref() as &dyn ParameterBase)
            .collect()
    }

    /// List all parameter names.
    pub fn names(&self) -> Vec<&str> {
        self.parameters.keys().map(|s| s.as_str()).collect()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_observable_basic() {
        let obs = Observable::new("test", 42);
        assert_eq!(obs.get(), 42);
        assert_eq!(obs.name(), "test");

        obs.set(100).unwrap();
        assert_eq!(obs.get(), 100);
    }

    #[test]
    fn test_observable_with_metadata() {
        let obs = Observable::new("threshold", 50.0)
            .with_description("Power threshold for alerts")
            .with_units("mW");

        assert_eq!(obs.metadata().units.as_deref(), Some("mW"));
        assert!(obs.metadata().description.is_some());
    }

    #[test]
    fn test_observable_range_validation() {
        let obs = Observable::new("rate", 10.0).with_range(0.1, 100.0);

        assert!(obs.set(50.0).is_ok());
        assert!(obs.set(0.05).is_err()); // Below min
        assert!(obs.set(150.0).is_err()); // Above max
    }

    #[test]
    fn test_observable_read_only() {
        let obs = Observable::new("version", "1.0.0".to_string()).read_only();

        assert!(obs.set("2.0.0".to_string()).is_err());
        assert_eq!(obs.get(), "1.0.0");
    }

    #[tokio::test]
    async fn test_observable_subscription() {
        let obs = Observable::new("value", 0);
        let mut rx = obs.subscribe();

        // Initial value
        assert_eq!(*rx.borrow(), 0);

        // Update and check
        obs.set(42).unwrap();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), 42);
    }

    #[test]
    fn test_observable_json_serialization() {
        let obs = Observable::new("threshold", 100.0).with_units("mW");

        // Get as JSON
        let json = obs.get_json().unwrap();
        assert_eq!(json, serde_json::json!(100.0));

        // Set from JSON
        obs.set_json(serde_json::json!(150.0)).unwrap();
        assert_eq!(obs.get(), 150.0);
    }

    #[test]
    fn test_observable_json_type_mismatch() {
        let obs = Observable::new("threshold", 100.0);

        // Try to set with wrong type
        let result = obs.set_json(serde_json::json!("not a number"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("deserialize"));
    }

    #[test]
    fn test_parameter_base_trait() {
        let obs = Observable::new("power", 50.0).with_units("mW");
        let param: &dyn ParameterBase = &obs;

        // Test trait methods
        assert_eq!(param.name(), "power");
        assert_eq!(param.metadata().units.as_deref(), Some("mW"));

        // Get and set via JSON
        let json = param.get_json().unwrap();
        assert_eq!(json, serde_json::json!(50.0));

        param.set_json(serde_json::json!(75.0)).unwrap();
        assert_eq!(obs.get(), 75.0); // Verify through concrete type
    }

    #[test]
    fn test_parameter_set() {
        let mut params = ParameterSet::new();

        params.register(Observable::new("threshold", 100.0).with_units("mW"));
        params.register(Observable::new("enabled", true));

        // Test typed access
        assert!(params.get_typed::<Observable<f64>>("threshold").is_some());
        assert!(params.get_typed::<Observable<bool>>("enabled").is_some());
        assert!(params.get_typed::<Observable<i32>>("missing").is_none());
    }

    #[test]
    fn test_parameter_set_generic_access() {
        let mut params = ParameterSet::new();

        params.register(Observable::new("wavelength", 850.0).with_units("nm"));
        params.register(Observable::new("power", 50.0).with_units("mW"));
        params.register(Observable::new("enabled", true));

        // Test generic access
        let param = params.get("wavelength").unwrap();
        assert_eq!(param.name(), "wavelength");
        assert_eq!(param.metadata().units.as_deref(), Some("nm"));

        // Test iteration
        let names: Vec<&str> = params.iter().map(|(name, _)| name).collect();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"wavelength"));
        assert!(names.contains(&"power"));
        assert!(names.contains(&"enabled"));

        // Test parameters() method
        let all_params = params.parameters();
        assert_eq!(all_params.len(), 3);
    }

    #[test]
    fn test_parameter_set_json_operations() {
        let mut params = ParameterSet::new();

        params.register(Observable::new("wavelength", 800.0).with_units("nm"));
        params.register(Observable::new("power", 100.0).with_units("mW"));

        // Get parameter generically and modify via JSON
        if let Some(param) = params.get("wavelength") {
            let current = param.get_json().unwrap();
            assert_eq!(current, serde_json::json!(800.0));

            param.set_json(serde_json::json!(850.0)).unwrap();
        }

        // Verify change through typed access
        let wavelength_param = params.get_typed::<Observable<f64>>("wavelength").unwrap();
        assert_eq!(wavelength_param.get(), 850.0);
    }

    #[tokio::test]
    async fn test_parameter_set_with_parameter() {
        use crate::parameter::Parameter;

        let mut params = ParameterSet::new();
        let param = Parameter::new("exposure", 10.0);

        params.register(param.clone());

        let registered = params
            .get_typed::<Parameter<f64>>("exposure")
            .expect("parameter registered");

        // Changing through the registry copy updates the original (shared watch channel)
        registered.set(25.0).await.unwrap();
        assert_eq!(param.get(), 25.0);
    }
}
