//! FFI-safe module interface using abi_stable's sabi_trait.
//!
//! This wraps the internal Module trait for cross-dylib calls.

#![allow(non_local_definitions)] // abi_stable's sabi_trait generates these

use abi_stable::sabi_trait;
use abi_stable::std_types::{RBox, RHashMap, ROption, RResult, RString, RVec};
use abi_stable::StableAbi;

/// FFI-safe module state (mirrors common::modules::ModuleState)
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
pub enum FfiModuleState {
    Unknown = 0,
    Created = 1,
    Configured = 2,
    Staged = 3,
    Running = 4,
    Paused = 5,
    Stopped = 6,
    Error = 7,
}

/// FFI-safe result type for module operations
pub type FfiModuleResult<T> = RResult<T, RString>;

/// FFI-safe module configuration (key-value pairs)
pub type FfiModuleConfig = RHashMap<RString, RString>;

/// FFI-safe role definition
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct FfiModuleRole {
    pub role_id: RString,
    pub description: RString,
    pub display_name: RString,
    pub required_capability: RString,
    pub allows_multiple: bool,
}

/// FFI-safe parameter definition
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct FfiModuleParameter {
    pub param_id: RString,
    pub display_name: RString,
    pub description: RString,
    pub param_type: RString,
    pub default_value: RString,
    pub min_value: ROption<RString>,
    pub max_value: ROption<RString>,
    pub enum_values: RVec<RString>,
    pub units: RString,
    pub required: bool,
}

/// FFI-safe module type information (mirrors common::modules::ModuleTypeInfo)
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct FfiModuleTypeInfo {
    pub type_id: RString,
    pub display_name: RString,
    pub description: RString,
    pub version: RString,
    pub parameters: RVec<FfiModuleParameter>,
    pub event_types: RVec<RString>,
    pub data_types: RVec<RString>,
    pub required_roles: RVec<FfiModuleRole>,
    pub optional_roles: RVec<FfiModuleRole>,
}

/// FFI-safe event data
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct FfiModuleEvent {
    pub event_type: RString,
    pub severity: u8, // 0=Unknown, 1=Info, 2=Warning, 3=Error, 4=Critical
    pub message: RString,
    pub data: RHashMap<RString, RString>,
}

/// FFI-safe data point
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct FfiModuleDataPoint {
    pub data_type: RString,
    pub timestamp_ns: u64,
    pub values: RHashMap<RString, f64>,
    pub metadata: RHashMap<RString, RString>,
}

/// Callback for emitting events from a module
pub type EventCallback = extern "C" fn(event: &FfiModuleEvent);

/// Callback for emitting data points from a module
pub type DataCallback = extern "C" fn(data: &FfiModuleDataPoint);

/// FFI-safe module context passed to module operations
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct FfiModuleContext {
    /// Module instance ID
    pub module_id: RString,

    /// Device assignments: role_id -> device_id
    pub assignments: RHashMap<RString, RString>,

    /// Opaque pointer to host-side context (for callbacks)
    /// The plugin should not dereference this directly
    pub host_context: usize,
}

/// The FFI-safe module trait.
///
/// This is the core interface that plugin modules implement. It mirrors the internal
/// Module trait but uses FFI-safe types throughout.
///
/// # Lifecycle
///
/// 1. `create()` - Factory creates a new instance
/// 2. `configure()` - Set parameters
/// 3. `stage()` - Prepare resources (optional)
/// 4. `start()` - Begin execution
/// 5. `pause()`/`resume()` - Control execution
/// 6. `stop()` - End execution
/// 7. `unstage()` - Release resources (optional)
///
/// # Note on Async
///
/// The internal Module trait is async, but FFI boundaries don't support async directly.
/// Plugin modules should handle async internally and block at the FFI boundary if needed,
/// or use the polling pattern with `poll_*` methods.
#[sabi_trait]
pub trait ModuleFfi: Send + Sync + 'static {
    /// Get static type information for this module type
    fn type_info(&self) -> FfiModuleTypeInfo;

    /// Get the module type ID
    fn type_id(&self) -> RString;

    /// Get current module state
    fn state(&self) -> FfiModuleState;

    /// Configure the module with parameters
    ///
    /// Returns a list of warnings (empty if none)
    fn configure(&mut self, params: FfiModuleConfig) -> FfiModuleResult<RVec<RString>>;

    /// Get current configuration
    fn get_config(&self) -> FfiModuleConfig;

    /// Stage the module (prepare resources)
    ///
    /// Called before start() to allocate buffers, warm up hardware, etc.
    fn stage(&mut self, ctx: &FfiModuleContext) -> FfiModuleResult<()>;

    /// Unstage the module (release resources)
    ///
    /// Called after stop() to free buffers, return hardware to safe state.
    /// Guaranteed to be called even on error.
    fn unstage(&mut self, ctx: &FfiModuleContext) -> FfiModuleResult<()>;

    /// Start module execution
    ///
    /// This should spawn internal async work and return immediately.
    fn start(&mut self, ctx: FfiModuleContext) -> FfiModuleResult<()>;

    /// Pause module execution
    fn pause(&mut self) -> FfiModuleResult<()>;

    /// Resume module execution
    fn resume(&mut self) -> FfiModuleResult<()>;

    /// Stop module execution
    fn stop(&mut self) -> FfiModuleResult<()>;

    /// Poll for pending events (non-blocking)
    ///
    /// Returns the next event if available, or None.
    /// Host should call this periodically to drain events.
    fn poll_event(&mut self) -> ROption<FfiModuleEvent>;

    /// Poll for pending data points (non-blocking)
    ///
    /// Returns the next data point if available, or None.
    /// Host should call this periodically to drain data.
    fn poll_data(&mut self) -> ROption<FfiModuleDataPoint>;
}

/// Type alias for an owned, boxed FFI module (like `Box<dyn ModuleFfi>`)
pub type ModuleFfiBox = ModuleFfi_TO<RBox<()>>;
