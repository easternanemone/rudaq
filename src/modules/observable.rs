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
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::watch;

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
            return Err(anyhow!(
                "Parameter '{}' is read-only",
                self.metadata.name
            ));
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
#[derive(Default)]
pub struct ParameterSet {
    /// Named parameters (type-erased for heterogeneous storage)
    parameters: std::collections::HashMap<String, Box<dyn std::any::Any + Send + Sync>>,
}

impl ParameterSet {
    /// Create a new empty parameter set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an observable parameter.
    pub fn register<T>(&mut self, observable: Observable<T>)
    where
        T: Clone + Send + Sync + 'static,
    {
        let name = observable.metadata.name.clone();
        self.parameters.insert(name, Box::new(observable));
    }

    /// Get a parameter by name.
    pub fn get<T>(&self, name: &str) -> Option<&Observable<T>>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.parameters
            .get(name)
            .and_then(|p| p.downcast_ref::<Observable<T>>())
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
    fn test_parameter_set() {
        let mut params = ParameterSet::new();

        params.register(Observable::new("threshold", 100.0).with_units("mW"));
        params.register(Observable::new("enabled", true));

        assert!(params.get::<f64>("threshold").is_some());
        assert!(params.get::<bool>("enabled").is_some());
        assert!(params.get::<i32>("missing").is_none());
    }
}
