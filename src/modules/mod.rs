//! Module System for Experiment Modules
//!
//! This module provides the infrastructure for experiment modules with runtime
//! instrument assignment, inspired by PyMoDAQ, DynExp, and Bluesky patterns.
//!
//! # Key Concepts
//!
//! - **Module**: A reusable experiment component that operates on abstract "roles"
//! - **Role**: A capability requirement (e.g., "power_meter" requires `Readable`)
//! - **ModuleContext**: Provides device access and event/data emission
//! - **ModuleRegistry**: Manages module types and instances
//! - **Observable**: Reactive parameters with change notifications
//! - **Document**: Bluesky-style self-describing data stream
//! - **RunEngine**: Central orchestrator for multi-module experiments
//!
//! # Example
//!
//! ```rust,ignore
//! // Create a power monitor module
//! let mut registry = ModuleRegistry::new(device_registry);
//! let module_id = registry.create_module("power_monitor", "Laser Power").await?;
//! registry.assign_device(&module_id, "power_meter", "newport_1830c").await?;
//! registry.configure_module(&module_id, config).await?;
//! registry.start_module(&module_id).await?;
//! ```

pub mod document;
pub mod power_monitor;
pub mod run_engine;

use crate::grpc::proto::{
    ModuleDataPoint, ModuleEvent, ModuleEventSeverity, ModuleState, ModuleTypeInfo,
};
use crate::hardware::capabilities::Readable;
use crate::hardware::registry::DeviceRegistry;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

// Re-export for convenience
pub use document::{DataKey, Document, StopReason};
pub use crate::observable::{Observable, ObservableMetadata, ParameterSet};
pub use power_monitor::PowerMonitor;
pub use run_engine::{RunConfig, RunEngine, RunReport};

// =============================================================================
// Module Trait
// =============================================================================

/// The core Module trait that all experiment modules implement.
///
/// Modules follow a lifecycle (Bluesky-inspired):
/// 1. Created (new instance)
/// 2. Configured (parameters set)
/// 3. **Staged** (resources allocated, hardware warmed up)
/// 4. Started (execution begins)
/// 5. Running (processing data, emitting events)
/// 6. Paused/Resumed (optional)
/// 7. Stopped (execution halted)
/// 8. **Unstaged** (resources released, guaranteed cleanup)
///
/// The stage/unstage pattern ensures proper resource management even on errors.
#[async_trait]
pub trait Module: Send + Sync + 'static {
    /// Get static information about this module type
    fn type_info() -> ModuleTypeInfo
    where
        Self: Sized;

    /// Get the type ID for this module
    fn type_id(&self) -> &str;

    /// Configure the module with parameters
    fn configure(&mut self, params: HashMap<String, String>) -> Result<Vec<String>>;

    /// Get current configuration
    fn get_config(&self) -> HashMap<String, String>;

    /// Stage the module (Bluesky pattern)
    ///
    /// Called before start() to prepare resources:
    /// - Allocate buffers
    /// - Warm up hardware (e.g., turn on laser at low power)
    /// - Validate device connections
    ///
    /// Default implementation does nothing.
    async fn stage(&mut self, _ctx: &ModuleContext) -> Result<()> {
        Ok(())
    }

    /// Unstage the module (Bluesky pattern)
    ///
    /// Called after stop() to release resources:
    /// - Free buffers
    /// - Return hardware to safe state
    /// - Close connections
    ///
    /// **This is guaranteed to be called even on error.**
    /// Default implementation does nothing.
    async fn unstage(&mut self, _ctx: &ModuleContext) -> Result<()> {
        Ok(())
    }

    /// Start module execution
    ///
    /// This should spawn the module's main loop and return immediately.
    /// The actual work happens in the background task.
    async fn start(&mut self, ctx: ModuleContext) -> Result<()>;

    /// Pause module execution
    async fn pause(&mut self) -> Result<()>;

    /// Resume module execution
    async fn resume(&mut self) -> Result<()>;

    /// Stop module execution
    async fn stop(&mut self) -> Result<()>;

    /// Get current module state
    fn state(&self) -> ModuleState;
}

// =============================================================================
// Module Context
// =============================================================================

/// Context provided to modules for device access and event/data emission.
///
/// The context provides:
/// - Access to assigned devices by role
/// - Channels for emitting events and data points
/// - Shutdown signal for graceful termination
pub struct ModuleContext {
    /// Module ID
    pub module_id: String,

    /// Assigned devices: role_id -> device_id
    assignments: HashMap<String, String>,

    /// Device registry for accessing hardware
    registry: Arc<RwLock<DeviceRegistry>>,

    /// Channel for emitting events
    event_tx: mpsc::Sender<ModuleEvent>,

    /// Channel for emitting data points
    data_tx: mpsc::Sender<ModuleDataPoint>,

    /// Shutdown signal
    shutdown_rx: broadcast::Receiver<()>,
}

impl std::fmt::Debug for ModuleContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleContext")
            .field("module_id", &self.module_id)
            .field("assignments", &self.assignments)
            .field("registry", &"<Arc<RwLock<DeviceRegistry>>>")
            .field("event_tx", &"<mpsc::Sender>")
            .field("data_tx", &"<mpsc::Sender>")
            .field("shutdown_rx", &"<broadcast::Receiver>")
            .finish()
    }
}

impl ModuleContext {
    /// Create a new module context
    pub fn new(
        module_id: String,
        assignments: HashMap<String, String>,
        registry: Arc<RwLock<DeviceRegistry>>,
        event_tx: mpsc::Sender<ModuleEvent>,
        data_tx: mpsc::Sender<ModuleDataPoint>,
        shutdown_rx: broadcast::Receiver<()>,
    ) -> Self {
        Self {
            module_id,
            assignments,
            registry,
            event_tx,
            data_tx,
            shutdown_rx,
        }
    }

    /// Get a Readable device assigned to a role
    pub async fn get_readable(&self, role_id: &str) -> Option<Arc<dyn Readable>> {
        let device_id = self.assignments.get(role_id)?;
        let registry = self.registry.read().await;
        registry.get_readable(device_id)
    }

    /// Emit an event
    pub async fn emit_event(&self, event_type: &str, severity: ModuleEventSeverity, message: &str) {
        self.emit_event_with_data(event_type, severity, message, HashMap::new())
            .await;
    }

    /// Emit an event with additional data
    pub async fn emit_event_with_data(
        &self,
        event_type: &str,
        severity: ModuleEventSeverity,
        message: &str,
        data: HashMap<String, String>,
    ) {
        let event = ModuleEvent {
            module_id: self.module_id.clone(),
            event_type: event_type.to_string(),
            timestamp_ns: current_time_ns(),
            severity: severity.into(),
            message: message.to_string(),
            data,
        };

        if let Err(e) = self.event_tx.send(event).await {
            warn!("Failed to emit event: {}", e);
        }
    }

    /// Emit a data point
    pub async fn emit_data(&self, data_type: &str, values: HashMap<String, f64>) {
        self.emit_data_with_metadata(data_type, values, HashMap::new())
            .await;
    }

    /// Emit a data point with metadata
    pub async fn emit_data_with_metadata(
        &self,
        data_type: &str,
        values: HashMap<String, f64>,
        metadata: HashMap<String, String>,
    ) {
        let data = ModuleDataPoint {
            module_id: self.module_id.clone(),
            data_type: data_type.to_string(),
            timestamp_ns: current_time_ns(),
            values,
            metadata,
        };

        if let Err(e) = self.data_tx.send(data).await {
            warn!("Failed to emit data: {}", e);
        }
    }

    /// Check if shutdown was requested
    pub fn is_shutdown_requested(&mut self) -> bool {
        self.shutdown_rx.try_recv().is_ok()
    }

    /// Wait for shutdown signal
    pub async fn wait_for_shutdown(&mut self) {
        let _ = self.shutdown_rx.recv().await;
    }
}

impl Clone for ModuleContext {
    fn clone(&self) -> Self {
        Self {
            module_id: self.module_id.clone(),
            assignments: self.assignments.clone(),
            registry: Arc::clone(&self.registry),
            event_tx: self.event_tx.clone(),
            data_tx: self.data_tx.clone(),
            shutdown_rx: self.shutdown_rx.resubscribe(),
        }
    }
}

// =============================================================================
// Module Instance
// =============================================================================

/// Runtime state for a module instance
pub struct ModuleInstance {
    /// Unique instance ID
    pub id: String,

    /// User-friendly name
    pub name: String,

    /// The module implementation
    module: Box<dyn Module>,

    /// Device assignments: role_id -> device_id
    assignments: HashMap<String, String>,

    /// Event sender for this module
    event_tx: mpsc::Sender<ModuleEvent>,

    /// Event receiver (for streaming)
    event_rx: Option<mpsc::Receiver<ModuleEvent>>,

    /// Data sender for this module
    data_tx: mpsc::Sender<ModuleDataPoint>,

    /// Data receiver (for streaming)
    data_rx: Option<mpsc::Receiver<ModuleDataPoint>>,

    /// Shutdown signal sender
    shutdown_tx: broadcast::Sender<()>,

    /// Runtime statistics
    pub start_time_ns: Option<u64>,
    pub events_emitted: u64,
    pub data_points_produced: u64,
    pub error_message: Option<String>,
}

impl std::fmt::Debug for ModuleInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleInstance")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("module", &"<Box<dyn Module>>")
            .field("assignments", &self.assignments)
            .field("event_tx", &"<mpsc::Sender>")
            .field("event_rx", &format!("{:?}", self.event_rx.is_some()))
            .field("data_tx", &"<mpsc::Sender>")
            .field("data_rx", &format!("{:?}", self.data_rx.is_some()))
            .field("shutdown_tx", &"<broadcast::Sender>")
            .field("start_time_ns", &self.start_time_ns)
            .field("events_emitted", &self.events_emitted)
            .field("data_points_produced", &self.data_points_produced)
            .field("error_message", &self.error_message)
            .finish()
    }
}

impl ModuleInstance {
    /// Create a new module instance
    pub fn new(id: String, name: String, module: Box<dyn Module>) -> Self {
        let (event_tx, event_rx) = mpsc::channel(100);
        let (data_tx, data_rx) = mpsc::channel(100);
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            id,
            name,
            module,
            assignments: HashMap::new(),
            event_tx,
            event_rx: Some(event_rx),
            data_tx,
            data_rx: Some(data_rx),
            shutdown_tx,
            start_time_ns: None,
            events_emitted: 0,
            data_points_produced: 0,
            error_message: None,
        }
    }

    /// Get the module type ID
    pub fn type_id(&self) -> &str {
        self.module.type_id()
    }

    /// Get current state
    pub fn state(&self) -> ModuleState {
        self.module.state()
    }

    /// Configure the module
    pub fn configure(&mut self, params: HashMap<String, String>) -> Result<Vec<String>> {
        self.module.configure(params)
    }

    /// Get current configuration
    pub fn get_config(&self) -> HashMap<String, String> {
        self.module.get_config()
    }

    /// Assign a device to a role
    pub fn assign_device(&mut self, role_id: String, device_id: String) {
        self.assignments.insert(role_id, device_id);
    }

    /// Unassign a device from a role
    pub fn unassign_device(&mut self, role_id: &str) {
        self.assignments.remove(role_id);
    }

    /// Get all assignments
    pub fn get_assignments(&self) -> &HashMap<String, String> {
        &self.assignments
    }

    /// Stage the module (Bluesky pattern - prepare resources before start)
    pub async fn stage(&mut self, registry: Arc<RwLock<DeviceRegistry>>) -> Result<()> {
        let ctx = ModuleContext::new(
            self.id.clone(),
            self.assignments.clone(),
            registry,
            self.event_tx.clone(),
            self.data_tx.clone(),
            self.shutdown_tx.subscribe(),
        );
        self.module.stage(&ctx).await
    }

    /// Unstage the module (Bluesky pattern - release resources after stop)
    pub async fn unstage(&mut self, registry: Arc<RwLock<DeviceRegistry>>) -> Result<()> {
        let ctx = ModuleContext::new(
            self.id.clone(),
            self.assignments.clone(),
            registry,
            self.event_tx.clone(),
            self.data_tx.clone(),
            self.shutdown_tx.subscribe(),
        );
        self.module.unstage(&ctx).await
    }

    /// Start the module
    pub async fn start(&mut self, registry: Arc<RwLock<DeviceRegistry>>) -> Result<()> {
        let ctx = ModuleContext::new(
            self.id.clone(),
            self.assignments.clone(),
            registry,
            self.event_tx.clone(),
            self.data_tx.clone(),
            self.shutdown_tx.subscribe(),
        );

        self.start_time_ns = Some(current_time_ns());
        self.module.start(ctx).await
    }

    /// Pause the module
    pub async fn pause(&mut self) -> Result<()> {
        self.module.pause().await
    }

    /// Resume the module
    pub async fn resume(&mut self) -> Result<()> {
        self.module.resume().await
    }

    /// Stop the module
    pub async fn stop(&mut self) -> Result<()> {
        // Send shutdown signal
        let _ = self.shutdown_tx.send(());
        self.module.stop().await
    }

    /// Take the event receiver (for streaming)
    pub fn take_event_rx(&mut self) -> Option<mpsc::Receiver<ModuleEvent>> {
        self.event_rx.take()
    }

    /// Take the data receiver (for streaming)
    pub fn take_data_rx(&mut self) -> Option<mpsc::Receiver<ModuleDataPoint>> {
        self.data_rx.take()
    }
}

// =============================================================================
// Module Registry
// =============================================================================

/// Factory function for creating modules
pub type ModuleFactory = fn() -> Box<dyn Module>;

/// Registry for module types and instances
pub struct ModuleRegistry {
    /// Device registry for hardware access
    device_registry: Arc<RwLock<DeviceRegistry>>,

    /// Registered module types: type_id -> factory
    module_types: HashMap<String, ModuleFactory>,

    /// Module type info cache
    type_info_cache: HashMap<String, ModuleTypeInfo>,

    /// Active module instances: module_id -> instance
    instances: HashMap<String, ModuleInstance>,
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleRegistry")
            .field("device_registry", &"<Arc<RwLock<DeviceRegistry>>>")
            .field("module_types", &format!("{} registered types", self.module_types.len()))
            .field("type_info_cache", &self.type_info_cache.keys().collect::<Vec<_>>())
            .field("instances", &format!("{} active instances", self.instances.len()))
            .finish()
    }
}

impl ModuleRegistry {
    /// Create a new module registry
    pub fn new(device_registry: Arc<RwLock<DeviceRegistry>>) -> Self {
        let mut registry = Self {
            device_registry,
            module_types: HashMap::new(),
            type_info_cache: HashMap::new(),
            instances: HashMap::new(),
        };

        // Register built-in modules
        registry.register_builtin_modules();

        registry
    }

    /// Register built-in module types
    fn register_builtin_modules(&mut self) {
        self.register_type::<PowerMonitor>();
    }

    /// Register a module type
    pub fn register_type<M: Module + Default + 'static>(&mut self) {
        let info = M::type_info();
        let type_id = info.type_id.clone();
        self.type_info_cache.insert(type_id.clone(), info);
        self.module_types
            .insert(type_id, || Box::new(M::default()));
    }

    /// List all registered module types
    pub fn list_types(&self) -> Vec<&ModuleTypeInfo> {
        self.type_info_cache.values().collect()
    }

    /// Get info for a specific module type
    pub fn get_type_info(&self, type_id: &str) -> Option<&ModuleTypeInfo> {
        self.type_info_cache.get(type_id)
    }

    /// Create a new module instance
    pub fn create_module(&mut self, type_id: &str, name: &str) -> Result<String> {
        let factory = self
            .module_types
            .get(type_id)
            .ok_or_else(|| anyhow!("Unknown module type: {}", type_id))?;

        let module = factory();
        let id = Uuid::new_v4().to_string();
        let instance = ModuleInstance::new(id.clone(), name.to_string(), module);
        self.instances.insert(id.clone(), instance);

        info!("Created module instance: {} (type: {})", id, type_id);
        Ok(id)
    }

    /// Delete a module instance
    pub async fn delete_module(&mut self, module_id: &str, force: bool) -> Result<()> {
        if let Some(instance) = self.instances.get(module_id) {
            let state = instance.state();
            if state == ModuleState::ModuleRunning && !force {
                return Err(anyhow!(
                    "Module is running. Stop it first or use force=true"
                ));
            }
        } else {
            return Err(anyhow!("Module not found: {}", module_id));
        }

        // Stop if running
        if let Some(instance) = self.instances.get_mut(module_id) {
            if instance.state() == ModuleState::ModuleRunning {
                instance.stop().await?;
            }
        }

        self.instances.remove(module_id);
        info!("Deleted module instance: {}", module_id);
        Ok(())
    }

    /// Get a module instance
    pub fn get_module(&self, module_id: &str) -> Option<&ModuleInstance> {
        self.instances.get(module_id)
    }

    /// Get a mutable module instance
    pub fn get_module_mut(&mut self, module_id: &str) -> Option<&mut ModuleInstance> {
        self.instances.get_mut(module_id)
    }

    /// List all module instances
    pub fn list_modules(&self) -> impl Iterator<Item = &ModuleInstance> {
        self.instances.values()
    }

    /// Configure a module
    pub fn configure_module(
        &mut self,
        module_id: &str,
        params: HashMap<String, String>,
    ) -> Result<Vec<String>> {
        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        if instance.state() == ModuleState::ModuleRunning {
            return Err(anyhow!("Cannot configure a running module"));
        }

        instance.configure(params)
    }

    /// Assign a device to a module role
    pub async fn assign_device(
        &mut self,
        module_id: &str,
        role_id: &str,
        device_id: &str,
    ) -> Result<()> {
        // Verify device exists
        {
            let registry = self.device_registry.read().await;
            if registry.get_device_info(device_id).is_none() {
                return Err(anyhow!("Device not found: {}", device_id));
            }
        }

        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        // Validate role exists for this module type
        let type_id = instance.type_id();
        let type_info = self
            .type_info_cache
            .get(type_id)
            .ok_or_else(|| anyhow!("Module type info not found: {}", type_id))?;

        // Check if role exists in required or optional roles
        let role_exists = type_info
            .required_roles
            .iter()
            .chain(type_info.optional_roles.iter())
            .any(|role| role.role_id == role_id);

        if !role_exists {
            // Build helpful error message listing valid roles
            let mut valid_roles = Vec::new();
            for role in &type_info.required_roles {
                valid_roles.push(format!("{} (required)", role.role_id));
            }
            for role in &type_info.optional_roles {
                valid_roles.push(format!("{} (optional)", role.role_id));
            }

            let valid_roles_str = if valid_roles.is_empty() {
                "none".to_string()
            } else {
                valid_roles.join(", ")
            };

            return Err(anyhow!(
                "Invalid role '{}' for module type '{}'. Valid roles: {}",
                role_id,
                type_id,
                valid_roles_str
            ));
        }

        instance.assign_device(role_id.to_string(), device_id.to_string());
        info!(
            "Assigned device {} to role {} in module {}",
            device_id, role_id, module_id
        );
        Ok(())
    }

    /// Unassign a device from a module role
    pub fn unassign_device(&mut self, module_id: &str, role_id: &str) -> Result<()> {
        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        if instance.state() == ModuleState::ModuleRunning {
            return Err(anyhow!("Cannot unassign device from a running module"));
        }

        instance.unassign_device(role_id);
        Ok(())
    }

    /// Start a module
    pub async fn start_module(&mut self, module_id: &str) -> Result<u64> {
        let registry = Arc::clone(&self.device_registry);
        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        instance.start(registry).await?;
        Ok(instance.start_time_ns.unwrap_or(0))
    }

    /// Pause a module
    pub async fn pause_module(&mut self, module_id: &str) -> Result<()> {
        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        instance.pause().await
    }

    /// Resume a module
    pub async fn resume_module(&mut self, module_id: &str) -> Result<()> {
        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        instance.resume().await
    }

    /// Stop a module
    pub async fn stop_module(&mut self, module_id: &str) -> Result<(u64, u64)> {
        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        instance.stop().await?;

        let uptime = instance.start_time_ns.map_or(0, |start| {
            current_time_ns().saturating_sub(start)
        });

        Ok((uptime, instance.events_emitted))
    }

    /// Get the device registry
    pub fn device_registry(&self) -> Arc<RwLock<DeviceRegistry>> {
        Arc::clone(&self.device_registry)
    }

    /// Stage a module (Bluesky pattern - prepare resources before start)
    pub async fn stage_module(&mut self, module_id: &str) -> Result<()> {
        let registry = Arc::clone(&self.device_registry);
        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        instance.stage(registry).await
    }

    /// Unstage a module (Bluesky pattern - release resources after stop)
    pub async fn unstage_module(&mut self, module_id: &str) -> Result<()> {
        let registry = Arc::clone(&self.device_registry);
        let instance = self
            .instances
            .get_mut(module_id)
            .ok_or_else(|| anyhow!("Module not found: {}", module_id))?;

        instance.unstage(registry).await
    }
}

// =============================================================================
// Utility Functions
// =============================================================================

/// Get current time in nanoseconds since Unix epoch
fn current_time_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_registry_creation() {
        let device_registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let registry = ModuleRegistry::new(device_registry);

        // Should have built-in types registered
        let types: Vec<_> = registry.list_types().into_iter().collect();
        assert!(!types.is_empty());
        assert!(types.iter().any(|t| t.type_id == "power_monitor"));
    }

    #[tokio::test]
    async fn test_create_module() {
        let device_registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let mut registry = ModuleRegistry::new(device_registry);

        let module_id = registry
            .create_module("power_monitor", "Test Monitor")
            .unwrap();
        assert!(!module_id.is_empty());

        let instance = registry.get_module(&module_id).unwrap();
        assert_eq!(instance.name, "Test Monitor");
        assert_eq!(instance.type_id(), "power_monitor");
    }

    #[tokio::test]
    async fn test_configure_module() {
        let device_registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let mut registry = ModuleRegistry::new(device_registry);

        let module_id = registry
            .create_module("power_monitor", "Test Monitor")
            .unwrap();

        let mut params = HashMap::new();
        params.insert("sample_rate_hz".to_string(), "20.0".to_string());
        params.insert("high_threshold".to_string(), "100.0".to_string());

        let warnings = registry.configure_module(&module_id, params).unwrap();
        assert!(warnings.is_empty());

        let instance = registry.get_module(&module_id).unwrap();
        let config = instance.get_config();
        assert_eq!(config.get("sample_rate_hz"), Some(&"20".to_string()));
    }

    #[tokio::test]
    async fn test_delete_module() {
        let device_registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        let mut registry = ModuleRegistry::new(device_registry);

        let module_id = registry
            .create_module("power_monitor", "Test Monitor")
            .unwrap();

        registry.delete_module(&module_id, false).await.unwrap();
        assert!(registry.get_module(&module_id).is_none());
    }
}
