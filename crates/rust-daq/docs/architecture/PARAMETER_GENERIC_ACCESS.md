# Generic Parameter Access via ParameterBase Trait

**Issue**: bd-hkzn
**Status**: Completed
**Date**: 2025-12-02

## Problem

ParameterSet previously stored `Box<dyn Any>` which completely erased parameter methods. This blocked gRPC's `set_parameter` endpoint from accessing parameters generically - it could see parameters existed but couldn't use them.

## Solution

Implemented a trait-based system that enables generic parameter access while preserving type safety:

### 1. ParameterBase Trait

```rust
pub trait ParameterBase: Send + Sync {
    fn name(&self) -> &str;
    fn get_json(&self) -> Result<serde_json::Value>;
    fn set_json(&self, value: serde_json::Value) -> Result<()>;
    fn metadata(&self) -> &ObservableMetadata;
    fn has_subscribers(&self) -> bool;
    fn subscriber_count(&self) -> usize;
}
```

Provides type-erased access to common parameter operations.

### 2. ParameterAny Trait

```rust
pub trait ParameterAny: ParameterBase {
    fn as_any(&self) -> &dyn Any;
}
```

Combines ParameterBase with Any for downcasting when concrete type is needed.

### 3. Observable<T> Implementation

Observable<T> implements both traits when `T: Serialize + DeserializeOwned`:

- `get_json()` serializes current value
- `set_json()` deserializes and validates before setting
- All methods delegate to existing Observable functionality

### 4. Updated ParameterSet

```rust
pub struct ParameterSet {
    parameters: HashMap<String, Box<dyn ParameterAny>>,
}
```

Storage changed from `Box<dyn Any>` to `Box<dyn ParameterAny>`.

#### New API Methods:

- `get(&self, name: &str) -> Option<&dyn ParameterBase>` - Generic access
- `get_typed<T>(&self, name: &str) -> Option<&Observable<T>>` - Typed access (backwards compatible)
- `iter(&self) -> impl Iterator<Item = (&str, &dyn ParameterBase)>` - Iterate all parameters
- `parameters(&self) -> Vec<&dyn ParameterBase>` - Get all parameters as vector

## Usage Pattern (gRPC)

```rust
// Generic access without knowing concrete type
let params = registry.get_parameters("maitai")?;
if let Some(param) = params.get("wavelength_nm") {
    // Get current value
    let current = param.get_json()?;

    // Set new value (type-safe deserialization)
    param.set_json(serde_json::json!(850.0))?;
}
```

## Error Handling

### Type Mismatch
```rust
// Setting string to numeric parameter
param.set_json(serde_json::json!("not a number"))
// Error: "Failed to deserialize parameter 'wavelength_nm': invalid type: string..."
```

### Validation Errors
```rust
// Setting value outside range
param.set_json(serde_json::json!(1200.0))
// Error: "Value 1200.0 out of range [700.0, 1000.0]"
```

## Type Constraints

For Observable<T> to implement ParameterBase/ParameterAny:
- `T: Clone + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static`

This is satisfied by all common parameter types (numeric types, String, bool).

## Backwards Compatibility

Existing code using typed access remains unchanged:

```rust
// Old API still works
let wavelength = params.get_typed::<f64>("wavelength_nm")?;
wavelength.set(850.0)?;
```

## Testing

Added comprehensive tests:
- `test_observable_json_serialization` - Round-trip JSON conversion
- `test_observable_json_type_mismatch` - Type error handling
- `test_parameter_base_trait` - Trait object usage
- `test_parameter_set_generic_access` - Generic iteration
- `test_parameter_set_json_operations` - End-to-end generic access

All existing tests continue to pass.

## Example

See `/examples/parameter_generic_access.rs` for complete demonstration of:
- Listing all parameters generically
- Setting parameters by name without type knowledge
- Error handling for type mismatches
- Error handling for validation failures
- Backwards-compatible typed access

## Next Steps

This unblocks:
- **bd-t988**: gRPC set_parameter fix - can now generically set parameters
- **bd-ysvn**: RunEngine parameter setting - can iterate and set parameters

## Files Modified

- `/src/observable.rs`:
  - Added ParameterBase and ParameterAny traits
  - Implemented traits for Observable<T>
  - Updated ParameterSet storage to Box<dyn ParameterAny>
  - Added get(), iter(), parameters() methods
  - Renamed get() to get_typed() for type-specific access
  - Added comprehensive tests

- `/examples/parameter_generic_access.rs`: New demonstration example

## Design Rationale

### Why Two Traits?

- **ParameterBase**: Clean public API for generic operations
- **ParameterAny**: Internal trait for downcasting when needed

This separation keeps the generic API focused while still enabling type-specific access when required.

### Why JSON for Generic Access?

- **Language agnostic**: Works with gRPC, REST, etc.
- **Type safe**: Serde validates types during deserialization
- **Error messages**: Clear errors on type mismatches
- **Extensible**: Works with any Serialize/Deserialize type

### Performance Considerations

- JSON serialization overhead is negligible for parameter updates (infrequent operations)
- No performance impact on typed access path
- Watch channel notifications unchanged
