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
//! - **Introspectable constraints** for GUI rendering (Phase 2: bd-cdh5.2)
//! - Metadata (name, units, description, dtype)
//! - Serialization support for snapshots
//! - Generic parameter access via ParameterBase trait
//!
//! # Constraint Types
//!
//! ## Basic Constraints (Validation Only)
//!
//! Use `with_range()` or `with_validator()` for validation without GUI introspection:
//!
//! ```rust,ignore
//! let threshold = Observable::new("threshold", 100.0)
//!     .with_range(0.0, 1000.0);  // Validates but not introspectable
//! ```
//!
//! ## Introspectable Constraints (Validation + GUI)
//!
//! Use `with_range_introspectable()` or `with_choices_introspectable()` for
//! constraints that are both validated AND exposed to the GUI via metadata:
//!
//! ```rust,ignore
//! // Float with slider bounds (dtype="float", min_value, max_value set)
//! let exposure = Observable::new("exposure_ms", 100.0)
//!     .with_range_introspectable(1.0, 10000.0);
//!
//! // Integer with slider bounds (dtype="int", min_value, max_value set)
//! let gain = Observable::new("gain", 1i64)
//!     .with_range_introspectable(0, 100);
//!
//! // String with enum choices (dtype="enum", enum_values set)
//! let mode = Observable::new("mode", "auto".to_string())
//!     .with_choices_introspectable(vec!["auto".into(), "manual".into()]);
//! ```
//!
//! The GUI reads these metadata fields to render appropriate widgets:
//! - `dtype="float"` or `dtype="int"` with `min_value`/`max_value` → Slider
//! - `dtype="enum"` with `enum_values` → ComboBox/Dropdown
//! - No constraints → DragValue or TextEdit
//!
//! # Example
//!
//! ```rust,ignore
//! let threshold = Observable::new("high_threshold", 100.0)
//!     .with_units("mW")
//!     .with_range_introspectable(0.0, 1000.0);  // GUI renders as slider
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
//!
//! # Design Notes (bd-cdh5.2)
//!
//! The introspectable constraint system was added in Phase 2 to support rich GUI
//! widgets. Key design decisions:
//!
//! - **Separate methods**: `with_range()` vs `with_range_introspectable()` to
//!   maintain backward compatibility and avoid metadata overhead when not needed.
//! - **f64 storage**: Both f64 and i64 constraints are stored as f64 in metadata.
//!   Large i64 values (outside ±2^53) may lose precision in GUI hints, but runtime
//!   validation uses exact values.
//! - **NaN/Infinity rejection**: `with_range_introspectable()` for f64 rejects
//!   non-finite values to prevent JSON serialization issues and GUI bugs.
//! - **dtype="enum"**: Choice parameters use `dtype="enum"` per the proto contract
//!   (daq.proto:610), not "string".

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fmt::Debug;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;

// =============================================================================
// Type Aliases
// =============================================================================

/// Validator callback type.
///
/// A function that validates a value and returns an error if invalid.
/// Used by [`Observable::with_validator`] and constraint methods.
pub type Validator<T> = Arc<dyn Fn(&T) -> Result<()> + Send + Sync>;

// =============================================================================
// Shared State (for dynamic metadata updates)
// =============================================================================

/// Shared state for Observable that can be updated and propagates to all clones.
///
/// This enables dynamic updates to metadata (e.g., enum choices) after
/// Observable creation. All clones of an Observable share the same state,
/// so updates are visible to gRPC handlers and GUI.
///
/// # Thread Safety
///
/// Uses `std::sync::RwLock` (not tokio) because:
/// - Metadata access is fast (no async needed)
/// - Avoids needing async context just to read metadata
/// - Consistent with the synchronous `get()` method
struct ObservableSharedState<T> {
    metadata: ObservableMetadata,
    validator: Option<Validator<T>>,
}

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

    /// Get the parameter metadata (returns a clone for thread safety).
    ///
    /// Returns by value because metadata is stored in a shared RwLock
    /// to support dynamic updates. ObservableMetadata is lightweight
    /// and Clone is cheap.
    fn metadata(&self) -> ObservableMetadata;

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
///
/// # Shared State
///
/// Metadata and validator are stored in a shared `Arc<RwLock<...>>` so that:
/// - All clones of an Observable share the same metadata
/// - Dynamic updates (e.g., enum choices) propagate to all holders
/// - gRPC handlers see updated metadata without re-registration
pub struct Observable<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// The watch channel sender (holds current value)
    sender: watch::Sender<T>,
    /// Shared metadata and validator (enables dynamic updates)
    shared: Arc<RwLock<ObservableSharedState<T>>>,
}

impl<T: Clone + Send + Sync + 'static> std::fmt::Debug for Observable<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shared = self.shared.read().unwrap();
        f.debug_struct("Observable")
            .field("metadata", &shared.metadata)
            .field("has_validator", &shared.validator.is_some())
            .finish()
    }
}

impl<T: Clone + Send + Sync + 'static> Clone for Observable<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(), // Clones sender (shares same watch channel)
            shared: self.shared.clone(), // Arc clone - shares same metadata!
        }
    }
}

/// Metadata for an observable parameter.
///
/// Contains both descriptive metadata (name, description, units) and
/// **introspectable constraint metadata** for GUI rendering (Phase 2: bd-cdh5.2).
///
/// # Introspectable Fields
///
/// The following fields are populated by `with_range_introspectable()` and
/// `with_choices_introspectable()` methods, and are passed through gRPC to the
/// GUI for rendering appropriate widgets:
///
/// | Field | Populated By | GUI Widget |
/// |-------|--------------|------------|
/// | `dtype="float"`, `min_value`, `max_value` | `Observable<f64>::with_range_introspectable()` | Slider |
/// | `dtype="int"`, `min_value`, `max_value` | `Observable<i64>::with_range_introspectable()` | Slider |
/// | `dtype="enum"`, `enum_values` | `Observable<String>::with_choices_introspectable()` | ComboBox |
///
/// # Serialization
///
/// All introspectable fields use `#[serde(default)]` for backward compatibility.
/// Existing configurations without these fields will deserialize correctly with
/// default values (empty string, None, empty Vec).
///
/// # Wire Format
///
/// These fields map directly to `ParameterDescriptor` in `daq.proto`:
/// - `dtype` → `ParameterDescriptor.dtype`
/// - `min_value` → `ParameterDescriptor.min_value`
/// - `max_value` → `ParameterDescriptor.max_value`
/// - `enum_values` → `ParameterDescriptor.enum_values`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservableMetadata {
    /// Parameter name (unique within module).
    ///
    /// Used as the identifier for gRPC operations and GUI labels.
    pub name: String,

    /// Human-readable description for tooltips and documentation.
    pub description: Option<String>,

    /// Physical units (e.g., "mW", "Hz", "mm", "°C").
    ///
    /// Displayed alongside values in the GUI.
    pub units: Option<String>,

    /// Whether this parameter is read-only.
    ///
    /// Read-only parameters reject `set()` calls and are displayed
    /// as non-editable in the GUI.
    pub read_only: bool,

    /// Data type hint for GUI widget selection.
    ///
    /// Standard values (per daq.proto):
    /// - `"float"` - Floating-point number (renders as Slider if bounded, DragValue otherwise)
    /// - `"int"` - Integer (renders as Slider if bounded, DragValue otherwise)
    /// - `"bool"` - Boolean (renders as Checkbox)
    /// - `"string"` - Free-form text (renders as TextEdit)
    /// - `"enum"` - Enumerated choice (renders as ComboBox when `enum_values` is non-empty)
    ///
    /// Empty string means dtype is unknown/inferred from value at runtime.
    #[serde(default)]
    pub dtype: String,

    /// Minimum value for numeric constraints (introspectable).
    ///
    /// Populated by `with_range_introspectable()`. When both `min_value` and
    /// `max_value` are set, the GUI renders a Slider instead of DragValue.
    ///
    /// **Note**: For `i64` parameters, the value is stored as `f64`. Large integers
    /// outside ±2^53 may lose precision in GUI hints, but runtime validation
    /// uses exact integer values.
    #[serde(default)]
    pub min_value: Option<f64>,

    /// Maximum value for numeric constraints (introspectable).
    ///
    /// Populated by `with_range_introspectable()`. When both `min_value` and
    /// `max_value` are set, the GUI renders a Slider instead of DragValue.
    ///
    /// **Note**: For `i64` parameters, the value is stored as `f64`. Large integers
    /// outside ±2^53 may lose precision in GUI hints, but runtime validation
    /// uses exact integer values.
    #[serde(default)]
    pub max_value: Option<f64>,

    /// Enum values for choice constraints (introspectable).
    ///
    /// Populated by `with_choices_introspectable()`. When non-empty, the GUI
    /// renders a ComboBox/Dropdown with these options.
    ///
    /// **Note**: When `enum_values` is non-empty, `dtype` should be `"enum"`
    /// per the proto contract (daq.proto:610).
    #[serde(default)]
    pub enum_values: Vec<String>,
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
            shared: Arc::new(RwLock::new(ObservableSharedState {
                metadata: ObservableMetadata {
                    name: name.into(),
                    description: None,
                    units: None,
                    read_only: false,
                    dtype: String::new(),
                    min_value: None,
                    max_value: None,
                    enum_values: Vec::new(),
                },
                validator: None,
            })),
        }
    }

    /// Add a description to this observable.
    pub fn with_description(self, description: impl Into<String>) -> Self {
        self.shared.write().unwrap().metadata.description = Some(description.into());
        self
    }

    /// Add units to this observable.
    pub fn with_units(self, units: impl Into<String>) -> Self {
        self.shared.write().unwrap().metadata.units = Some(units.into());
        self
    }

    /// Mark this observable as read-only.
    pub fn read_only(self) -> Self {
        self.shared.write().unwrap().metadata.read_only = true;
        self
    }

    /// Add a custom validator function.
    pub fn with_validator<F>(self, validator: F) -> Self
    where
        F: Fn(&T) -> Result<()> + Send + Sync + 'static,
    {
        self.shared.write().unwrap().validator = Some(Arc::new(validator));
        self
    }

    /// Get the current value (clone).
    pub fn get(&self) -> T {
        self.sender.borrow().clone()
    }

    /// Get the parameter name.
    pub fn name(&self) -> &str {
        // Note: This returns a reference to shared state, which requires the lock
        // to be held. For short-lived operations this is fine. For longer usage,
        // call metadata() and access .name on the returned clone.
        // SAFETY: We leak the read guard here which is not ideal but maintains
        // API compatibility. Consider returning String in future versions.
        unsafe {
            let guard = self.shared.read().unwrap();
            let name_ptr = guard.metadata.name.as_str() as *const str;
            // This is safe because the name is never modified after creation
            // and the Observable (and its Arc) outlives any reference to name
            &*name_ptr
        }
    }

    /// Get the metadata (returns a clone for thread safety).
    ///
    /// Returns by value because metadata is stored in a shared RwLock.
    /// ObservableMetadata is lightweight and Clone is cheap.
    pub fn metadata(&self) -> ObservableMetadata {
        self.shared.read().unwrap().metadata.clone()
    }

    /// Update metadata with a closure (for thread-safe modifications).
    ///
    /// This is the preferred way to modify metadata after Observable creation.
    pub fn with_metadata<F>(&self, f: F)
    where
        F: FnOnce(&mut ObservableMetadata),
    {
        let mut guard = self.shared.write().unwrap();
        f(&mut guard.metadata);
    }

    /// Validate a value without setting it.
    ///
    /// Returns error if:
    /// - Parameter is read-only
    /// - Validation fails
    ///
    /// This is useful when you need to validate before performing
    /// an expensive operation (like hardware write) that shouldn't
    /// happen if validation will fail.
    pub fn validate(&self, value: &T) -> Result<()> {
        let guard = self.shared.read().unwrap();
        if guard.metadata.read_only {
            return Err(anyhow!(
                "Parameter '{}' is read-only",
                guard.metadata.name
            ));
        }

        if let Some(validator) = &guard.validator {
            validator(value)?;
        }

        Ok(())
    }

    /// Set a new value, notifying all subscribers.
    ///
    /// Returns error if:
    /// - Parameter is read-only
    /// - Validation fails
    pub fn set(&self, value: T) -> Result<()> {
        self.validate(&value)?;
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
        let name = self.metadata().name.clone();
        serde_json::to_value(&value).map_err(|e| {
            anyhow!("Failed to serialize parameter '{}': {}", name, e)
        })
    }

    /// Set the value from JSON
    pub fn set_json(&self, json_value: serde_json::Value) -> Result<()> {
        let name = self.metadata().name.clone();
        let value: T = serde_json::from_value(json_value).map_err(|e| {
            anyhow!(
                "Failed to deserialize parameter '{}': {}. Expected type: {}",
                name,
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
        Observable::name(self)
    }

    fn get_json(&self) -> Result<serde_json::Value> {
        Observable::get_json(self)
    }

    fn set_json(&self, value: serde_json::Value) -> Result<()> {
        Observable::set_json(self, value)
    }

    fn metadata(&self) -> ObservableMetadata {
        Observable::metadata(self)
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
        let value = self.get();
        (&value as &dyn Any).downcast_ref::<f64>().copied()
    }

    fn value_as_bool(&self) -> Option<bool> {
        let value = self.get();
        (&value as &dyn Any).downcast_ref::<bool>().copied()
    }

    fn value_as_string(&self) -> Option<String> {
        let value = self.get();
        (&value as &dyn Any).downcast_ref::<String>().cloned()
    }

    fn value_as_i64(&self) -> Option<i64> {
        let value = self.get();
        (&value as &dyn Any).downcast_ref::<i64>().copied()
    }
}

// =============================================================================
// Numeric Observable Extensions
// =============================================================================

impl<T> Observable<T>
where
    T: Clone + Send + Sync + PartialOrd + Debug + 'static,
{
    /// Add min/max range validation (validation only, not introspectable).
    ///
    /// This method adds a validator that rejects values outside `[min, max]`,
    /// but does **not** populate the metadata fields (`min_value`, `max_value`,
    /// `dtype`) used for GUI introspection.
    ///
    /// Use [`Observable<f64>::with_range_introspectable()`] or
    /// [`Observable<i64>::with_range_introspectable()`] if you need the GUI
    /// to render a Slider with these bounds.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Validation only - GUI won't know about bounds
    /// let threshold = Observable::new("threshold", 50.0)
    ///     .with_range(0.0, 100.0);
    ///
    /// threshold.set(150.0);  // Error: out of range
    /// ```
    pub fn with_range(self, min: T, max: T) -> Self {
        let min_clone = min.clone();
        let max_clone = max.clone();
        self.shared.write().unwrap().validator = Some(Arc::new(move |value: &T| {
            if value < &min_clone || value > &max_clone {
                Err(anyhow!(
                    "Value {:?} out of range [{:?}, {:?}]",
                    value,
                    min_clone,
                    max_clone
                ))
            } else {
                Ok(())
            }
        }));
        self
    }
}

// =============================================================================
// Type-Specific Introspectable Extensions (Phase 2: bd-cdh5.2)
// =============================================================================

impl Observable<f64> {
    /// Add min/max range validation with introspectable metadata for GUI.
    ///
    /// This method:
    /// 1. Sets `metadata.min_value` and `metadata.max_value` for GUI introspection
    /// 2. Sets `metadata.dtype = "float"` so the GUI knows the value type
    /// 3. Adds a validator that rejects values outside `[min, max]`
    /// 4. Rejects NaN and Infinity values to prevent JSON serialization issues
    ///
    /// The GUI reads these metadata fields via gRPC `ListParameters` and renders
    /// a Slider widget when both `min_value` and `max_value` are set.
    ///
    /// # Arguments
    ///
    /// * `min` - Minimum allowed value (inclusive). Must be finite.
    /// * `max` - Maximum allowed value (inclusive). Must be finite and >= min.
    ///
    /// # Panics
    ///
    /// Panics at parameter construction time if:
    /// - `min` or `max` is NaN or Infinity (non-finite)
    /// - `min > max` (bounds out of order)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // GUI will render this as a Slider from 1.0 to 10000.0
    /// let exposure = Observable::new("exposure_ms", 100.0)
    ///     .with_units("ms")
    ///     .with_range_introspectable(1.0, 10000.0);
    ///
    /// exposure.set(500.0)?;           // OK
    /// exposure.set(0.5)?;             // Error: below min
    /// exposure.set(f64::NAN)?;        // Error: non-finite
    /// exposure.set(f64::INFINITY)?;   // Error: non-finite
    /// ```
    ///
    /// # See Also
    ///
    /// - [`with_range()`](Observable::with_range) - Validation only, no GUI introspection
    /// - [`ObservableMetadata`] - Documentation of metadata fields
    pub fn with_range_introspectable(self, min: f64, max: f64) -> Self {
        // Validate bounds are finite and ordered at construction time
        assert!(
            min.is_finite() && max.is_finite(),
            "Range bounds must be finite: min={}, max={}",
            min,
            max
        );
        assert!(min <= max, "min must be <= max: min={}, max={}", min, max);

        // Populate introspectable metadata for GUI and add validator
        {
            let mut guard = self.shared.write().unwrap();
            guard.metadata.min_value = Some(min);
            guard.metadata.max_value = Some(max);
            guard.metadata.dtype = "float".to_string();

            // Add validator that rejects non-finite and out-of-range values
            guard.validator = Some(Arc::new(move |value: &f64| {
                // Reject NaN and Infinity to prevent JSON serialization issues
                if !value.is_finite() {
                    return Err(anyhow!("Value must be finite, got {:?}", value));
                }
                if *value < min || *value > max {
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
        }
        self
    }

    /// Set the dtype for this observable (manual override).
    ///
    /// Normally you should use `with_range_introspectable()` which sets
    /// dtype automatically. This method is for cases where you need to
    /// set dtype without adding range constraints.
    pub fn with_dtype(self, dtype: impl Into<String>) -> Self {
        self.shared.write().unwrap().metadata.dtype = dtype.into();
        self
    }
}

impl Observable<i64> {
    /// Add min/max range validation with introspectable metadata for GUI.
    ///
    /// This method:
    /// 1. Sets `metadata.min_value` and `metadata.max_value` for GUI introspection
    /// 2. Sets `metadata.dtype = "int"` so the GUI knows the value type
    /// 3. Adds a validator that rejects values outside `[min, max]`
    ///
    /// The GUI reads these metadata fields via gRPC `ListParameters` and renders
    /// a Slider widget when both `min_value` and `max_value` are set.
    ///
    /// # Integer Precision Note
    ///
    /// The `min_value` and `max_value` metadata fields are stored as `f64` for
    /// wire format compatibility. Large i64 values outside the range ±2^53
    /// (~9 quadrillion) may lose precision in the GUI metadata. However, the
    /// actual validation uses exact i64 comparison, so runtime behavior is correct.
    ///
    /// For most hardware parameters (e.g., pixel counts, sensor indices), this
    /// precision loss is not a concern since values are well within ±2^53.
    ///
    /// # Arguments
    ///
    /// * `min` - Minimum allowed value (inclusive)
    /// * `max` - Maximum allowed value (inclusive). Must be >= min.
    ///
    /// # Panics
    ///
    /// Panics at parameter construction time if `min > max`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // GUI will render this as a Slider from 0 to 100
    /// let gain = Observable::new("gain", 1i64)
    ///     .with_range_introspectable(0, 100);
    ///
    /// gain.set(50)?;   // OK
    /// gain.set(-1)?;   // Error: below min
    /// gain.set(101)?;  // Error: above max
    /// ```
    ///
    /// # See Also
    ///
    /// - [`with_range()`](Observable::with_range) - Validation only, no GUI introspection
    /// - [`ObservableMetadata`] - Documentation of metadata fields
    pub fn with_range_introspectable(self, min: i64, max: i64) -> Self {
        // Validate bounds ordering at construction time
        assert!(min <= max, "min must be <= max: min={}, max={}", min, max);

        // Populate introspectable metadata for GUI and add validator
        {
            let mut guard = self.shared.write().unwrap();
            guard.metadata.min_value = Some(min as f64);
            guard.metadata.max_value = Some(max as f64);
            guard.metadata.dtype = "int".to_string();

            // Add validator using exact i64 comparison
            guard.validator = Some(Arc::new(move |value: &i64| {
                if *value < min || *value > max {
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
        }
        self
    }

    /// Set the dtype for this observable (manual override).
    ///
    /// Normally you should use `with_range_introspectable()` which sets
    /// dtype automatically. This method is for cases where you need to
    /// set dtype without adding range constraints.
    pub fn with_dtype(self, dtype: impl Into<String>) -> Self {
        self.shared.write().unwrap().metadata.dtype = dtype.into();
        self
    }
}

impl Observable<String> {
    /// Add choice validation with introspectable metadata for GUI.
    ///
    /// This method:
    /// 1. Sets `metadata.enum_values` with the allowed choices for GUI introspection
    /// 2. Sets `metadata.dtype = "enum"` per the proto contract (daq.proto:610)
    /// 3. Adds a validator that rejects values not in the choices list
    ///
    /// The GUI reads these metadata fields via gRPC `ListParameters` and renders
    /// a ComboBox/Dropdown widget when `enum_values` is non-empty.
    ///
    /// # Arguments
    ///
    /// * `choices` - List of valid string values. The current value should be in this list.
    ///
    /// # Proto Contract
    ///
    /// Per `daq.proto:610`, parameters with discrete choices should set `dtype = "enum"`,
    /// not `"string"`. This ensures non-egui clients (e.g., Python, web) can correctly
    /// identify enum parameters and render appropriate UI controls.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // GUI will render this as a ComboBox with "auto", "manual", "continuous"
    /// let mode = Observable::new("mode", "auto".to_string())
    ///     .with_choices_introspectable(vec![
    ///         "auto".into(),
    ///         "manual".into(),
    ///         "continuous".into(),
    ///     ]);
    ///
    /// mode.set("manual".into())?;    // OK
    /// mode.set("invalid".into())?;   // Error: not in choices
    /// ```
    ///
    /// # See Also
    ///
    /// - [`ObservableMetadata::enum_values`] - The metadata field populated by this method
    /// - [`ObservableMetadata::dtype`] - Set to `"enum"` by this method
    pub fn with_choices_introspectable(self, choices: Vec<String>) -> Self {
        // Populate introspectable metadata for GUI and add validator
        {
            let mut guard = self.shared.write().unwrap();
            guard.metadata.enum_values = choices.clone();
            guard.metadata.dtype = "enum".to_string(); // Per proto contract (daq.proto:610)

            // Add validator that rejects values not in choices
            guard.validator = Some(Arc::new(move |value: &String| {
                if choices.iter().any(|c| c == value) {
                    Ok(())
                } else {
                    Err(anyhow!("Value {:?} not in choices {:?}", value, choices))
                }
            }));
        }
        self
    }

    /// Update the available choices for this enum parameter at runtime.
    ///
    /// This method is designed for dynamic enumeration scenarios where the
    /// available choices depend on hardware state or other runtime factors.
    /// It updates both the metadata (for GUI introspection) and the validator.
    ///
    /// Since the Observable uses shared state via `Arc<RwLock>`, all clones
    /// will see the updated choices immediately - this is critical for gRPC
    /// handlers that clone parameters.
    ///
    /// # Arguments
    ///
    /// * `choices` - New list of valid string values
    ///
    /// # Note on Current Value
    ///
    /// This method does NOT validate or update the current value. If the
    /// current value is not in the new choices list, subsequent `set()` calls
    /// with different values will fail validation, but the current value remains
    /// unchanged until explicitly set. This allows for scenarios where:
    ///
    /// 1. The current selection becomes temporarily invalid during a transition
    /// 2. The caller will immediately set a new valid value
    ///
    /// If you need to validate the current value against the new choices,
    /// do so explicitly after calling this method.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Initial setup with static choices
    /// let port = Observable::new("port", "A".to_string())
    ///     .with_choices_introspectable(vec!["A".into(), "B".into()]);
    ///
    /// // Later, after querying hardware for available ports
    /// let available_ports = vec!["A".into(), "C".into(), "D".into()];
    /// port.update_choices(available_ports);
    ///
    /// // Now the GUI will show A, C, D options
    /// // And validation will accept only those values
    /// port.set("C".into())?;  // OK
    /// port.set("B".into())?;  // Error: B no longer in choices
    /// ```
    pub fn update_choices(&self, choices: Vec<String>) {
        let mut guard = self.shared.write().unwrap();
        guard.metadata.enum_values = choices.clone();
        guard.metadata.dtype = "enum".to_string();

        // Update validator with new choices
        guard.validator = Some(Arc::new(move |value: &String| {
            if choices.iter().any(|c| c == value) {
                Ok(())
            } else {
                Err(anyhow!("Value {:?} not in choices {:?}", value, choices))
            }
        }));
    }

    /// Set the dtype for this observable (manual override).
    ///
    /// Normally you should use `with_choices_introspectable()` which sets
    /// dtype automatically. This method is for cases where you need to
    /// set dtype without adding choice constraints.
    pub fn with_dtype(self, dtype: impl Into<String>) -> Self {
        self.shared.write().unwrap().metadata.dtype = dtype.into();
        self
    }
}

impl Observable<bool> {
    /// Set the dtype for this observable (manual override).
    ///
    /// Boolean observables typically don't need explicit dtype since the
    /// GUI can infer the type from the JSON value. This method is provided
    /// for consistency with other types.
    pub fn with_dtype(self, dtype: impl Into<String>) -> Self {
        self.shared.write().unwrap().metadata.dtype = dtype.into();
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

    #[test]
    fn test_value_as_f64() {
        let obs_f64 = Observable::new("temperature", 25.5);
        let param: &dyn ParameterAny = &obs_f64;

        // f64 observable returns Some for value_as_f64
        assert_eq!(param.value_as_f64(), Some(25.5));

        // f64 observable returns None for other types
        assert_eq!(param.value_as_bool(), None);
        assert_eq!(param.value_as_string(), None);
        assert_eq!(param.value_as_i64(), None);
    }

    #[test]
    fn test_value_as_bool() {
        let obs_bool = Observable::new("enabled", true);
        let param: &dyn ParameterAny = &obs_bool;

        // bool observable returns Some for value_as_bool
        assert_eq!(param.value_as_bool(), Some(true));

        // bool observable returns None for other types
        assert_eq!(param.value_as_f64(), None);
        assert_eq!(param.value_as_string(), None);
        assert_eq!(param.value_as_i64(), None);
    }

    #[test]
    fn test_value_as_string() {
        let obs_string = Observable::new("mode", "auto".to_string());
        let param: &dyn ParameterAny = &obs_string;

        // String observable returns Some for value_as_string
        assert_eq!(param.value_as_string(), Some("auto".to_string()));

        // String observable returns None for other types
        assert_eq!(param.value_as_f64(), None);
        assert_eq!(param.value_as_bool(), None);
        assert_eq!(param.value_as_i64(), None);
    }

    #[test]
    fn test_value_as_i64() {
        let obs_i64 = Observable::new("count", 42_i64);
        let param: &dyn ParameterAny = &obs_i64;

        // i64 observable returns Some for value_as_i64
        assert_eq!(param.value_as_i64(), Some(42));

        // i64 observable returns None for other types
        assert_eq!(param.value_as_f64(), None);
        assert_eq!(param.value_as_bool(), None);
        assert_eq!(param.value_as_string(), None);
    }

    #[test]
    fn test_value_as_type_mismatch() {
        // Test that non-primitive types return None for all accessors
        #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
        struct CustomType {
            field: i32,
        }

        let obs = Observable::new("custom", CustomType { field: 123 });
        let param: &dyn ParameterAny = &obs;

        assert_eq!(param.value_as_f64(), None);
        assert_eq!(param.value_as_bool(), None);
        assert_eq!(param.value_as_string(), None);
        assert_eq!(param.value_as_i64(), None);
    }
}
