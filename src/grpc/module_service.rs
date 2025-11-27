//! ModuleService implementation for experiment modules (bd-c0ai)
//!
//! This module provides gRPC endpoints for managing experiment modules,
//! inspired by PyMoDAQ and DynExp patterns. Modules operate on abstract
//! "roles" and have devices assigned at runtime.
//!
//! Key concepts:
//! - Module types define behavior (PowerMonitor, DataLogger, etc.)
//! - Modules are instantiated from types
//! - Devices are assigned to roles within modules at runtime
//! - Modules have lifecycle: Created -> Configured -> Running -> Stopped
//!
//! ## Feature Modes
//!
//! - **Without `modules` feature**: Stub mode with in-memory state only
//! - **With `modules` feature**: Full integration with ModuleRegistry

use crate::grpc::proto::{
    module_service_server::ModuleService, AssignDeviceRequest, AssignDeviceResponse,
    ConfigureModuleRequest, ConfigureModuleResponse, CreateModuleRequest, CreateModuleResponse,
    DeleteModuleRequest, DeleteModuleResponse, DeviceAssignment, GetModuleConfigRequest,
    GetModuleStatusRequest, GetModuleTypeInfoRequest, ListAssignmentsRequest,
    ListAssignmentsResponse, ListModuleTypesRequest, ListModuleTypesResponse, ListModulesRequest,
    ListModulesResponse, ModuleConfig, ModuleDataPoint, ModuleEvent, ModuleParameter, ModuleRole,
    ModuleStatus, ModuleTypeInfo, ModuleTypeSummary, PauseModuleRequest, PauseModuleResponse,
    ResumeModuleRequest, ResumeModuleResponse, StartModuleRequest, StartModuleResponse,
    StopModuleRequest, StopModuleResponse, StreamModuleDataRequest, StreamModuleEventsRequest,
    UnassignDeviceRequest, UnassignDeviceResponse,
};
#[cfg(not(feature = "modules"))]
use crate::grpc::proto::ModuleState;
use crate::hardware::registry::DeviceRegistry;
#[cfg(feature = "modules")]
use crate::modules::ModuleRegistry;
#[cfg(not(feature = "modules"))]
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tonic::{Request, Response, Status};

// =============================================================================
// Stub Mode (without modules feature)
// =============================================================================

/// Module instance state (stub mode only)
#[cfg(not(feature = "modules"))]
#[derive(Clone)]
struct StubModuleInstance {
    module_id: String,
    type_id: String,
    instance_name: String,
    state: ModuleState,
    config: HashMap<String, String>,
    assignments: HashMap<String, String>,
    start_time_ns: Option<u64>,
    events_emitted: u64,
    data_points_produced: u64,
    error_message: Option<String>,
    error_time_ns: Option<u64>,
}

#[cfg(not(feature = "modules"))]
impl StubModuleInstance {
    fn new(type_id: String, instance_name: String) -> Self {
        Self {
            module_id: uuid::Uuid::new_v4().to_string(),
            type_id,
            instance_name,
            state: ModuleState::ModuleCreated,
            config: HashMap::new(),
            assignments: HashMap::new(),
            start_time_ns: None,
            events_emitted: 0,
            data_points_produced: 0,
            error_message: None,
            error_time_ns: None,
        }
    }

    fn to_status(&self) -> ModuleStatus {
        let (required_filled, required_total) = self.get_role_status();
        let uptime_ns = self.start_time_ns.map(|start| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64;
            now.saturating_sub(start)
        });

        ModuleStatus {
            module_id: self.module_id.clone(),
            type_id: self.type_id.clone(),
            instance_name: self.instance_name.clone(),
            state: self.state.into(),
            required_roles_filled: required_filled,
            required_roles_total: required_total,
            ready_to_start: required_filled >= required_total && !self.config.is_empty(),
            start_time_ns: self.start_time_ns.unwrap_or(0),
            uptime_ns: uptime_ns.unwrap_or(0),
            events_emitted: self.events_emitted,
            data_points_produced: self.data_points_produced,
            error_message: self.error_message.clone().unwrap_or_default(),
            error_time_ns: self.error_time_ns.unwrap_or(0),
        }
    }

    #[allow(clippy::unnecessary_to_owned)]
    fn get_role_status(&self) -> (u32, u32) {
        let required_roles = get_module_type_required_roles(&self.type_id);
        let filled = required_roles
            .iter()
            .filter(|role| self.assignments.contains_key(&(*role).to_string()))
            .count() as u32;
        (filled, required_roles.len() as u32)
    }
}

// =============================================================================
// Static Type Information (stub mode only)
// =============================================================================

/// Get required roles for a module type
#[cfg(not(feature = "modules"))]
fn get_module_type_required_roles(type_id: &str) -> Vec<&'static str> {
    match type_id {
        "power_monitor" => vec!["power_meter"],
        "position_tracker" => vec!["stage"],
        "data_logger" => vec!["data_source"],
        "multi_channel_logger" => vec![],
        _ => vec![],
    }
}

/// Built-in module type definitions (stub mode only)
#[cfg(not(feature = "modules"))]
fn get_builtin_module_types() -> Vec<ModuleTypeSummary> {
    vec![
        ModuleTypeSummary {
            type_id: "power_monitor".to_string(),
            display_name: "Power Monitor".to_string(),
            description: "Monitors power readings with threshold alerts".to_string(),
            categories: vec!["monitoring".to_string(), "threshold".to_string()],
        },
        ModuleTypeSummary {
            type_id: "position_tracker".to_string(),
            display_name: "Position Tracker".to_string(),
            description: "Tracks stage position and logs movement".to_string(),
            categories: vec!["monitoring".to_string(), "motion".to_string()],
        },
        ModuleTypeSummary {
            type_id: "data_logger".to_string(),
            display_name: "Data Logger".to_string(),
            description: "Logs scalar readings to file with configurable rate".to_string(),
            categories: vec!["logging".to_string(), "storage".to_string()],
        },
        ModuleTypeSummary {
            type_id: "multi_channel_logger".to_string(),
            display_name: "Multi-Channel Logger".to_string(),
            description: "Logs multiple data sources with synchronized timestamps".to_string(),
            categories: vec!["logging".to_string(), "multi-channel".to_string()],
        },
    ]
}

/// Get detailed info for a module type (stub mode only)
#[cfg(not(feature = "modules"))]
fn get_static_module_type_info(type_id: &str) -> Option<ModuleTypeInfo> {
    match type_id {
        "power_monitor" => Some(ModuleTypeInfo {
            type_id: "power_monitor".to_string(),
            display_name: "Power Monitor".to_string(),
            description: "Monitors power readings with configurable thresholds".to_string(),
            version: "1.0.0".to_string(),
            required_roles: vec![ModuleRole {
                role_id: "power_meter".to_string(),
                display_name: "Power Meter".to_string(),
                description: "Device providing power readings".to_string(),
                required_capability: "readable".to_string(),
                allows_multiple: false,
            }],
            optional_roles: vec![],
            parameters: vec![
                ModuleParameter {
                    param_id: "low_threshold".to_string(),
                    display_name: "Low Threshold".to_string(),
                    description: "Alert when power drops below this value".to_string(),
                    param_type: "float".to_string(),
                    default_value: "0.0".to_string(),
                    min_value: Some("0.0".to_string()),
                    max_value: None,
                    enum_values: vec![],
                    units: "mW".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "high_threshold".to_string(),
                    display_name: "High Threshold".to_string(),
                    description: "Alert when power exceeds this value".to_string(),
                    param_type: "float".to_string(),
                    default_value: "1000.0".to_string(),
                    min_value: Some("0.0".to_string()),
                    max_value: None,
                    enum_values: vec![],
                    units: "mW".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "sample_rate_hz".to_string(),
                    display_name: "Sample Rate".to_string(),
                    description: "How often to read and check thresholds".to_string(),
                    param_type: "float".to_string(),
                    default_value: "10.0".to_string(),
                    min_value: Some("0.1".to_string()),
                    max_value: Some("100.0".to_string()),
                    enum_values: vec![],
                    units: "Hz".to_string(),
                    required: false,
                },
            ],
            event_types: vec![
                "threshold_exceeded".to_string(),
                "threshold_recovered".to_string(),
                "state_change".to_string(),
            ],
            data_types: vec![
                "power_reading".to_string(),
                "power_average".to_string(),
                "statistics".to_string(),
            ],
        }),
        "position_tracker" => Some(ModuleTypeInfo {
            type_id: "position_tracker".to_string(),
            display_name: "Position Tracker".to_string(),
            description: "Tracks and logs stage position over time".to_string(),
            version: "1.0.0".to_string(),
            required_roles: vec![ModuleRole {
                role_id: "stage".to_string(),
                display_name: "Stage".to_string(),
                description: "Movable stage to track".to_string(),
                required_capability: "movable".to_string(),
                allows_multiple: false,
            }],
            optional_roles: vec![],
            parameters: vec![ModuleParameter {
                param_id: "poll_rate_hz".to_string(),
                display_name: "Poll Rate".to_string(),
                description: "How often to query position".to_string(),
                param_type: "float".to_string(),
                default_value: "10.0".to_string(),
                min_value: Some("0.1".to_string()),
                max_value: Some("100.0".to_string()),
                enum_values: vec![],
                units: "Hz".to_string(),
                required: false,
            }],
            event_types: vec!["motion_started".to_string(), "motion_stopped".to_string()],
            data_types: vec!["position".to_string(), "velocity".to_string()],
        }),
        "data_logger" => Some(ModuleTypeInfo {
            type_id: "data_logger".to_string(),
            display_name: "Data Logger".to_string(),
            description: "Logs scalar readings to file".to_string(),
            version: "1.0.0".to_string(),
            required_roles: vec![ModuleRole {
                role_id: "data_source".to_string(),
                display_name: "Data Source".to_string(),
                description: "Device providing scalar readings".to_string(),
                required_capability: "readable".to_string(),
                allows_multiple: false,
            }],
            optional_roles: vec![],
            parameters: vec![
                ModuleParameter {
                    param_id: "sample_rate_hz".to_string(),
                    display_name: "Sample Rate".to_string(),
                    description: "Logging rate".to_string(),
                    param_type: "float".to_string(),
                    default_value: "1.0".to_string(),
                    min_value: Some("0.01".to_string()),
                    max_value: Some("1000.0".to_string()),
                    enum_values: vec![],
                    units: "Hz".to_string(),
                    required: false,
                },
                ModuleParameter {
                    param_id: "output_file".to_string(),
                    display_name: "Output File".to_string(),
                    description: "Path to output file".to_string(),
                    param_type: "string".to_string(),
                    default_value: "data_log.csv".to_string(),
                    min_value: None,
                    max_value: None,
                    enum_values: vec![],
                    units: String::new(),
                    required: true,
                },
            ],
            event_types: vec!["file_rotated".to_string(), "buffer_flushed".to_string()],
            data_types: vec!["logged_value".to_string()],
        }),
        "multi_channel_logger" => Some(ModuleTypeInfo {
            type_id: "multi_channel_logger".to_string(),
            display_name: "Multi-Channel Logger".to_string(),
            description: "Logs multiple data sources with synchronized timestamps".to_string(),
            version: "1.0.0".to_string(),
            required_roles: vec![],
            optional_roles: vec![ModuleRole {
                role_id: "channel".to_string(),
                display_name: "Data Channel".to_string(),
                description: "Additional data source to log".to_string(),
                required_capability: "readable".to_string(),
                allows_multiple: true,
            }],
            parameters: vec![ModuleParameter {
                param_id: "sync_rate_hz".to_string(),
                display_name: "Sync Rate".to_string(),
                description: "Synchronized sampling rate for all channels".to_string(),
                param_type: "float".to_string(),
                default_value: "10.0".to_string(),
                min_value: Some("0.1".to_string()),
                max_value: Some("100.0".to_string()),
                enum_values: vec![],
                units: "Hz".to_string(),
                required: false,
            }],
            event_types: vec!["sync_error".to_string()],
            data_types: vec!["multi_channel_sample".to_string()],
        }),
        _ => None,
    }
}

// =============================================================================
// Module Service Implementation
// =============================================================================

/// Module gRPC service implementation
///
/// This service provides full module management through gRPC:
/// - Module type discovery
/// - Module lifecycle (create, delete, list)
/// - Configuration and device assignment
/// - Execution control (start, pause, resume, stop)
/// - Event and data streaming
pub struct ModuleServiceImpl {
    /// Device registry for hardware access
    #[allow(dead_code)]
    device_registry: Arc<RwLock<DeviceRegistry>>,

    /// Stub module storage (used when modules feature is disabled)
    #[cfg(not(feature = "modules"))]
    stub_modules: Arc<RwLock<HashMap<String, StubModuleInstance>>>,

    /// Real module registry (when modules feature is enabled)
    #[cfg(feature = "modules")]
    module_registry: Arc<RwLock<ModuleRegistry>>,
}

impl ModuleServiceImpl {
    /// Create a new ModuleService (stub mode - without modules feature)
    #[cfg(not(feature = "modules"))]
    pub fn new(registry: Arc<RwLock<DeviceRegistry>>) -> Self {
        Self {
            device_registry: registry,
            stub_modules: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new ModuleService (full mode - with modules feature)
    #[cfg(feature = "modules")]
    pub fn new(registry: Arc<RwLock<DeviceRegistry>>) -> Self {
        let module_registry = ModuleRegistry::new(registry.clone());
        Self {
            device_registry: registry,
            module_registry: Arc::new(RwLock::new(module_registry)),
        }
    }
}

// =============================================================================
// Full ModuleService Implementation (with modules feature)
// =============================================================================

#[cfg(feature = "modules")]
#[tonic::async_trait]
impl ModuleService for ModuleServiceImpl {
    // =========================================================================
    // Module Type Discovery (wired to ModuleRegistry)
    // =========================================================================

    async fn list_module_types(
        &self,
        request: Request<ListModuleTypesRequest>,
    ) -> Result<Response<ListModuleTypesResponse>, Status> {
        let req = request.into_inner();
        let registry = self.module_registry.read().await;

        // Get types from real registry
        let types: Vec<ModuleTypeSummary> = registry
            .list_types()
            .iter()
            .filter(|info| {
                // Filter by capability if specified
                if let Some(ref cap) = req.required_capability {
                    info.required_roles
                        .iter()
                        .any(|r| r.required_capability == *cap)
                        || info
                            .optional_roles
                            .iter()
                            .any(|r| r.required_capability == *cap)
                } else {
                    true
                }
            })
            .map(|info| ModuleTypeSummary {
                type_id: info.type_id.clone(),
                display_name: info.display_name.clone(),
                description: info.description.clone(),
                categories: info.event_types.clone(), // Use event_types as categories for now
            })
            .collect();

        Ok(Response::new(ListModuleTypesResponse {
            module_types: types,
        }))
    }

    async fn get_module_type_info(
        &self,
        request: Request<GetModuleTypeInfoRequest>,
    ) -> Result<Response<ModuleTypeInfo>, Status> {
        let req = request.into_inner();
        let registry = self.module_registry.read().await;

        registry
            .get_type_info(&req.type_id)
            .cloned()
            .map(Response::new)
            .ok_or_else(|| Status::not_found(format!("Unknown module type: {}", req.type_id)))
    }

    // =========================================================================
    // Module Lifecycle (wired to ModuleRegistry)
    // =========================================================================

    async fn create_module(
        &self,
        request: Request<CreateModuleRequest>,
    ) -> Result<Response<CreateModuleResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        match registry.create_module(&req.type_id, &req.instance_name) {
            Ok(module_id) => {
                // Apply initial config if provided
                if !req.initial_config.is_empty() {
                    if let Err(e) = registry.configure_module(&module_id, req.initial_config) {
                        // Module created but config failed - still return success with warning
                        return Ok(Response::new(CreateModuleResponse {
                            success: true,
                            module_id,
                            error_message: format!("Created but config failed: {}", e),
                        }));
                    }
                }

                Ok(Response::new(CreateModuleResponse {
                    success: true,
                    module_id,
                    error_message: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(CreateModuleResponse {
                success: false,
                module_id: String::new(),
                error_message: e.to_string(),
            })),
        }
    }

    async fn delete_module(
        &self,
        request: Request<DeleteModuleRequest>,
    ) -> Result<Response<DeleteModuleResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        match registry.delete_module(&req.module_id, req.force).await {
            Ok(()) => Ok(Response::new(DeleteModuleResponse {
                success: true,
                error_message: String::new(),
            })),
            Err(e) => Ok(Response::new(DeleteModuleResponse {
                success: false,
                error_message: e.to_string(),
            })),
        }
    }

    async fn list_modules(
        &self,
        request: Request<ListModulesRequest>,
    ) -> Result<Response<ListModulesResponse>, Status> {
        let req = request.into_inner();
        let registry = self.module_registry.read().await;
        let device_registry = self.device_registry.read().await;

        let mut modules: Vec<ModuleStatus> = registry
            .list_modules()
            .filter(|instance| {
                // Filter by type if specified
                if let Some(ref type_filter) = req.type_filter {
                    if instance.type_id() != type_filter {
                        return false;
                    }
                }
                // Filter by state if specified
                if let Some(state_filter) = req.state_filter {
                    if instance.state() as i32 != state_filter {
                        return false;
                    }
                }
                true
            })
            .map(|instance| {
                let assignments = instance.get_assignments();
                let type_info = registry.get_type_info(instance.type_id());
                let required_total = type_info
                    .map(|i| i.required_roles.len())
                    .unwrap_or(0) as u32;
                let required_filled = type_info
                    .map(|info| {
                        info.required_roles
                            .iter()
                            .filter(|r| assignments.contains_key(&r.role_id))
                            .count()
                    })
                    .unwrap_or(0) as u32;

                let uptime_ns = instance.start_time_ns.map(|start| {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos() as u64;
                    now.saturating_sub(start)
                });

                // Check all devices are online
                let all_devices_online = assignments.values().all(|device_id| {
                    device_registry.get_device_info(device_id).is_some()
                });

                ModuleStatus {
                    module_id: instance.id.clone(),
                    type_id: instance.type_id().to_string(),
                    instance_name: instance.name.clone(),
                    state: instance.state().into(),
                    required_roles_filled: required_filled,
                    required_roles_total: required_total,
                    ready_to_start: required_filled >= required_total
                        && !instance.get_config().is_empty()
                        && all_devices_online,
                    start_time_ns: instance.start_time_ns.unwrap_or(0),
                    uptime_ns: uptime_ns.unwrap_or(0),
                    events_emitted: instance.events_emitted,
                    data_points_produced: instance.data_points_produced,
                    error_message: instance.error_message.clone().unwrap_or_default(),
                    error_time_ns: 0,
                }
            })
            .collect();

        modules.sort_by(|a, b| a.instance_name.cmp(&b.instance_name));

        Ok(Response::new(ListModulesResponse { modules }))
    }

    async fn get_module_status(
        &self,
        request: Request<GetModuleStatusRequest>,
    ) -> Result<Response<ModuleStatus>, Status> {
        let req = request.into_inner();
        let registry = self.module_registry.read().await;
        let device_registry = self.device_registry.read().await;

        let instance = registry
            .get_module(&req.module_id)
            .ok_or_else(|| Status::not_found(format!("Module not found: {}", req.module_id)))?;

        let assignments = instance.get_assignments();
        let type_info = registry.get_type_info(instance.type_id());
        let required_total = type_info
            .map(|i| i.required_roles.len())
            .unwrap_or(0) as u32;
        let required_filled = type_info
            .map(|info| {
                info.required_roles
                    .iter()
                    .filter(|r| assignments.contains_key(&r.role_id))
                    .count()
            })
            .unwrap_or(0) as u32;

        let uptime_ns = instance.start_time_ns.map(|start| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64;
            now.saturating_sub(start)
        });

        let all_devices_online = assignments
            .values()
            .all(|device_id| device_registry.get_device_info(device_id).is_some());

        Ok(Response::new(ModuleStatus {
            module_id: instance.id.clone(),
            type_id: instance.type_id().to_string(),
            instance_name: instance.name.clone(),
            state: instance.state().into(),
            required_roles_filled: required_filled,
            required_roles_total: required_total,
            ready_to_start: required_filled >= required_total
                && !instance.get_config().is_empty()
                && all_devices_online,
            start_time_ns: instance.start_time_ns.unwrap_or(0),
            uptime_ns: uptime_ns.unwrap_or(0),
            events_emitted: instance.events_emitted,
            data_points_produced: instance.data_points_produced,
            error_message: instance.error_message.clone().unwrap_or_default(),
            error_time_ns: 0,
        }))
    }

    // =========================================================================
    // Module Configuration (wired to ModuleRegistry)
    // =========================================================================

    async fn configure_module(
        &self,
        request: Request<ConfigureModuleRequest>,
    ) -> Result<Response<ConfigureModuleResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        // Get current config if partial update
        let params = if req.partial {
            let instance = registry.get_module(&req.module_id).ok_or_else(|| {
                Status::not_found(format!("Module not found: {}", req.module_id))
            })?;
            let mut current = instance.get_config();
            for (k, v) in req.parameters {
                current.insert(k, v);
            }
            current
        } else {
            req.parameters
        };

        match registry.configure_module(&req.module_id, params) {
            Ok(warnings) => Ok(Response::new(ConfigureModuleResponse {
                success: true,
                error_message: String::new(),
                warnings,
            })),
            Err(e) => Ok(Response::new(ConfigureModuleResponse {
                success: false,
                error_message: e.to_string(),
                warnings: vec![],
            })),
        }
    }

    async fn get_module_config(
        &self,
        request: Request<GetModuleConfigRequest>,
    ) -> Result<Response<ModuleConfig>, Status> {
        let req = request.into_inner();
        let registry = self.module_registry.read().await;
        let device_registry = self.device_registry.read().await;

        let instance = registry
            .get_module(&req.module_id)
            .ok_or_else(|| Status::not_found(format!("Module not found: {}", req.module_id)))?;

        let assignments: Vec<DeviceAssignment> = instance
            .get_assignments()
            .iter()
            .map(|(role_id, device_id)| {
                let device_info = device_registry.get_device_info(device_id);
                DeviceAssignment {
                    role_id: role_id.clone(),
                    device_id: device_id.clone(),
                    device_name: device_info
                        .as_ref()
                        .map(|i| i.name.clone())
                        .unwrap_or_else(|| device_id.clone()),
                    device_online: device_info.is_some(),
                }
            })
            .collect();

        Ok(Response::new(ModuleConfig {
            module_id: instance.id.clone(),
            type_id: instance.type_id().to_string(),
            parameters: instance.get_config(),
            assignments,
        }))
    }

    // =========================================================================
    // Device Assignment (wired to ModuleRegistry)
    // =========================================================================

    async fn assign_device(
        &self,
        request: Request<AssignDeviceRequest>,
    ) -> Result<Response<AssignDeviceResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        match registry
            .assign_device(&req.module_id, &req.role_id, &req.device_id)
            .await
        {
            Ok(()) => {
                // Check if module is now ready
                let instance = registry.get_module(&req.module_id);
                let ready = instance.map_or(false, |inst| {
                    let type_info = registry.get_type_info(inst.type_id());
                    let required_total = type_info.map(|i| i.required_roles.len()).unwrap_or(0);
                    let assignments = inst.get_assignments();
                    let filled = type_info
                        .map(|info| {
                            info.required_roles
                                .iter()
                                .filter(|r| assignments.contains_key(&r.role_id))
                                .count()
                        })
                        .unwrap_or(0);
                    filled >= required_total && !inst.get_config().is_empty()
                });

                Ok(Response::new(AssignDeviceResponse {
                    success: true,
                    error_message: String::new(),
                    module_ready: ready,
                }))
            }
            Err(e) => Ok(Response::new(AssignDeviceResponse {
                success: false,
                error_message: e.to_string(),
                module_ready: false,
            })),
        }
    }

    async fn unassign_device(
        &self,
        request: Request<UnassignDeviceRequest>,
    ) -> Result<Response<UnassignDeviceResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        match registry.unassign_device(&req.module_id, &req.role_id) {
            Ok(()) => Ok(Response::new(UnassignDeviceResponse {
                success: true,
                error_message: String::new(),
            })),
            Err(e) => Ok(Response::new(UnassignDeviceResponse {
                success: false,
                error_message: e.to_string(),
            })),
        }
    }

    async fn list_assignments(
        &self,
        request: Request<ListAssignmentsRequest>,
    ) -> Result<Response<ListAssignmentsResponse>, Status> {
        let req = request.into_inner();
        let registry = self.module_registry.read().await;
        let device_registry = self.device_registry.read().await;

        let instance = registry
            .get_module(&req.module_id)
            .ok_or_else(|| Status::not_found(format!("Module not found: {}", req.module_id)))?;

        let assignments: Vec<DeviceAssignment> = instance
            .get_assignments()
            .iter()
            .map(|(role_id, device_id)| {
                let device_info = device_registry.get_device_info(device_id);
                DeviceAssignment {
                    role_id: role_id.clone(),
                    device_id: device_id.clone(),
                    device_name: device_info
                        .as_ref()
                        .map(|i| i.name.clone())
                        .unwrap_or_else(|| device_id.clone()),
                    device_online: device_info.is_some(),
                }
            })
            .collect();

        Ok(Response::new(ListAssignmentsResponse { assignments }))
    }

    // =========================================================================
    // Module Execution Control (wired to ModuleRegistry)
    // =========================================================================

    async fn start_module(
        &self,
        request: Request<StartModuleRequest>,
    ) -> Result<Response<StartModuleResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        match registry.start_module(&req.module_id).await {
            Ok(start_time_ns) => Ok(Response::new(StartModuleResponse {
                success: true,
                error_message: String::new(),
                start_time_ns,
            })),
            Err(e) => Ok(Response::new(StartModuleResponse {
                success: false,
                error_message: e.to_string(),
                start_time_ns: 0,
            })),
        }
    }

    async fn pause_module(
        &self,
        request: Request<PauseModuleRequest>,
    ) -> Result<Response<PauseModuleResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        match registry.pause_module(&req.module_id).await {
            Ok(()) => Ok(Response::new(PauseModuleResponse {
                success: true,
                error_message: String::new(),
            })),
            Err(e) => Ok(Response::new(PauseModuleResponse {
                success: false,
                error_message: e.to_string(),
            })),
        }
    }

    async fn resume_module(
        &self,
        request: Request<ResumeModuleRequest>,
    ) -> Result<Response<ResumeModuleResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        match registry.resume_module(&req.module_id).await {
            Ok(()) => Ok(Response::new(ResumeModuleResponse {
                success: true,
                error_message: String::new(),
            })),
            Err(e) => Ok(Response::new(ResumeModuleResponse {
                success: false,
                error_message: e.to_string(),
            })),
        }
    }

    async fn stop_module(
        &self,
        request: Request<StopModuleRequest>,
    ) -> Result<Response<StopModuleResponse>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        match registry.stop_module(&req.module_id).await {
            Ok((uptime_ns, events_emitted)) => Ok(Response::new(StopModuleResponse {
                success: true,
                error_message: String::new(),
                uptime_ns,
                events_emitted,
            })),
            Err(e) => Ok(Response::new(StopModuleResponse {
                success: false,
                error_message: e.to_string(),
                uptime_ns: 0,
                events_emitted: 0,
            })),
        }
    }

    // =========================================================================
    // Module Data Streaming (wired to ModuleInstance event/data receivers)
    // =========================================================================

    type StreamModuleEventsStream =
        tokio_stream::wrappers::ReceiverStream<Result<ModuleEvent, Status>>;

    async fn stream_module_events(
        &self,
        request: Request<StreamModuleEventsRequest>,
    ) -> Result<Response<Self::StreamModuleEventsStream>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        // Get the module's event receiver
        let instance = registry
            .get_module_mut(&req.module_id)
            .ok_or_else(|| Status::not_found(format!("Module not found: {}", req.module_id)))?;

        let event_rx = instance.take_event_rx().ok_or_else(|| {
            Status::resource_exhausted("Event stream already taken for this module")
        })?;

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let event_types = req.event_types;

        // Forward events from module to gRPC stream
        tokio::spawn(async move {
            let mut event_rx = event_rx;
            while let Some(event) = event_rx.recv().await {
                // Filter by event type if specified
                if !event_types.is_empty() && !event_types.contains(&event.event_type) {
                    continue;
                }

                if tx.send(Ok(event)).await.is_err() {
                    break; // Client disconnected
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    type StreamModuleDataStream =
        tokio_stream::wrappers::ReceiverStream<Result<ModuleDataPoint, Status>>;

    async fn stream_module_data(
        &self,
        request: Request<StreamModuleDataRequest>,
    ) -> Result<Response<Self::StreamModuleDataStream>, Status> {
        let req = request.into_inner();
        let mut registry = self.module_registry.write().await;

        // Get the module's data receiver
        let instance = registry
            .get_module_mut(&req.module_id)
            .ok_or_else(|| Status::not_found(format!("Module not found: {}", req.module_id)))?;

        let data_rx = instance
            .take_data_rx()
            .ok_or_else(|| Status::resource_exhausted("Data stream already taken for this module"))?;

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let data_types = req.data_types;
        let max_rate_hz = req.max_rate_hz;

        // Forward data from module to gRPC stream with optional rate limiting
        tokio::spawn(async move {
            let mut data_rx = data_rx;
            let mut rate_limiter = if max_rate_hz > 0 {
                Some(tokio::time::interval(std::time::Duration::from_secs_f64(
                    1.0 / max_rate_hz as f64,
                )))
            } else {
                None
            };

            while let Some(data) = data_rx.recv().await {
                // Rate limit if configured
                if let Some(ref mut limiter) = rate_limiter {
                    limiter.tick().await;
                }

                // Filter by data type if specified
                if !data_types.is_empty() && !data_types.contains(&data.data_type) {
                    continue;
                }

                if tx.send(Ok(data)).await.is_err() {
                    break; // Client disconnected
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}

// =============================================================================
// Stub ModuleService Implementation (without modules feature)
// =============================================================================

#[cfg(not(feature = "modules"))]
#[tonic::async_trait]
impl ModuleService for ModuleServiceImpl {
    async fn list_module_types(
        &self,
        request: Request<ListModuleTypesRequest>,
    ) -> Result<Response<ListModuleTypesResponse>, Status> {
        let req = request.into_inner();
        let mut types = get_builtin_module_types();

        if let Some(cap) = req.required_capability {
            types.retain(|t| {
                if let Some(info) = get_static_module_type_info(&t.type_id) {
                    info.required_roles
                        .iter()
                        .any(|r| r.required_capability == cap)
                        || info
                            .optional_roles
                            .iter()
                            .any(|r| r.required_capability == cap)
                } else {
                    false
                }
            });
        }

        Ok(Response::new(ListModuleTypesResponse {
            module_types: types,
        }))
    }

    async fn get_module_type_info(
        &self,
        request: Request<GetModuleTypeInfoRequest>,
    ) -> Result<Response<ModuleTypeInfo>, Status> {
        let req = request.into_inner();
        get_static_module_type_info(&req.type_id)
            .map(Response::new)
            .ok_or_else(|| Status::not_found(format!("Unknown module type: {}", req.type_id)))
    }

    async fn create_module(
        &self,
        request: Request<CreateModuleRequest>,
    ) -> Result<Response<CreateModuleResponse>, Status> {
        let req = request.into_inner();

        if get_static_module_type_info(&req.type_id).is_none() {
            return Ok(Response::new(CreateModuleResponse {
                success: false,
                module_id: String::new(),
                error_message: format!("Unknown module type: {}", req.type_id),
            }));
        }

        let mut module = StubModuleInstance::new(req.type_id, req.instance_name);

        if !req.initial_config.is_empty() {
            module.config = req.initial_config;
            module.state = ModuleState::ModuleConfigured;
        }

        let module_id = module.module_id.clone();
        self.stub_modules
            .write()
            .await
            .insert(module_id.clone(), module);

        Ok(Response::new(CreateModuleResponse {
            success: true,
            module_id,
            error_message: String::new(),
        }))
    }

    async fn delete_module(
        &self,
        request: Request<DeleteModuleRequest>,
    ) -> Result<Response<DeleteModuleResponse>, Status> {
        let req = request.into_inner();
        let mut modules = self.stub_modules.write().await;

        if let Some(module) = modules.get(&req.module_id) {
            if module.state == ModuleState::ModuleRunning && !req.force {
                return Ok(Response::new(DeleteModuleResponse {
                    success: false,
                    error_message: "Module is running. Stop it first or use force=true".to_string(),
                }));
            }

            modules.remove(&req.module_id);
            Ok(Response::new(DeleteModuleResponse {
                success: true,
                error_message: String::new(),
            }))
        } else {
            Ok(Response::new(DeleteModuleResponse {
                success: false,
                error_message: format!("Module not found: {}", req.module_id),
            }))
        }
    }

    async fn list_modules(
        &self,
        request: Request<ListModulesRequest>,
    ) -> Result<Response<ListModulesResponse>, Status> {
        let req = request.into_inner();
        let modules = self.stub_modules.read().await;

        let mut module_list: Vec<ModuleStatus> = modules
            .values()
            .filter(|m| {
                if let Some(ref type_filter) = req.type_filter {
                    if &m.type_id != type_filter {
                        return false;
                    }
                }
                if let Some(state_filter) = req.state_filter {
                    if m.state as i32 != state_filter {
                        return false;
                    }
                }
                true
            })
            .map(|m| m.to_status())
            .collect();

        module_list.sort_by(|a, b| a.instance_name.cmp(&b.instance_name));

        Ok(Response::new(ListModulesResponse {
            modules: module_list,
        }))
    }

    async fn get_module_status(
        &self,
        request: Request<GetModuleStatusRequest>,
    ) -> Result<Response<ModuleStatus>, Status> {
        let req = request.into_inner();
        let modules = self.stub_modules.read().await;

        modules
            .get(&req.module_id)
            .map(|m| Response::new(m.to_status()))
            .ok_or_else(|| Status::not_found(format!("Module not found: {}", req.module_id)))
    }

    async fn configure_module(
        &self,
        request: Request<ConfigureModuleRequest>,
    ) -> Result<Response<ConfigureModuleResponse>, Status> {
        let req = request.into_inner();
        let mut modules = self.stub_modules.write().await;

        if let Some(module) = modules.get_mut(&req.module_id) {
            if module.state == ModuleState::ModuleRunning {
                return Ok(Response::new(ConfigureModuleResponse {
                    success: false,
                    error_message: "Cannot configure a running module. Stop it first.".to_string(),
                    warnings: vec![],
                }));
            }

            if req.partial {
                for (k, v) in req.parameters {
                    module.config.insert(k, v);
                }
            } else {
                module.config = req.parameters;
            }

            if !module.config.is_empty()
                && (module.state == ModuleState::ModuleCreated
                    || module.state == ModuleState::ModuleStopped)
            {
                module.state = ModuleState::ModuleConfigured;
            }

            Ok(Response::new(ConfigureModuleResponse {
                success: true,
                error_message: String::new(),
                warnings: vec![],
            }))
        } else {
            Ok(Response::new(ConfigureModuleResponse {
                success: false,
                error_message: format!("Module not found: {}", req.module_id),
                warnings: vec![],
            }))
        }
    }

    async fn get_module_config(
        &self,
        request: Request<GetModuleConfigRequest>,
    ) -> Result<Response<ModuleConfig>, Status> {
        let req = request.into_inner();
        let modules = self.stub_modules.read().await;
        let registry = self.device_registry.read().await;

        if let Some(module) = modules.get(&req.module_id) {
            let assignments: Vec<DeviceAssignment> = module
                .assignments
                .iter()
                .map(|(role_id, device_id)| {
                    let device_info = registry.get_device_info(device_id);
                    DeviceAssignment {
                        role_id: role_id.clone(),
                        device_id: device_id.clone(),
                        device_name: device_info
                            .as_ref()
                            .map(|i| i.name.clone())
                            .unwrap_or_else(|| device_id.clone()),
                        device_online: device_info.is_some(),
                    }
                })
                .collect();

            Ok(Response::new(ModuleConfig {
                module_id: module.module_id.clone(),
                type_id: module.type_id.clone(),
                parameters: module.config.clone(),
                assignments,
            }))
        } else {
            Err(Status::not_found(format!(
                "Module not found: {}",
                req.module_id
            )))
        }
    }

    async fn assign_device(
        &self,
        request: Request<AssignDeviceRequest>,
    ) -> Result<Response<AssignDeviceResponse>, Status> {
        let req = request.into_inner();

        {
            let registry = self.device_registry.read().await;
            if registry.get_device_info(&req.device_id).is_none() {
                return Ok(Response::new(AssignDeviceResponse {
                    success: false,
                    error_message: format!("Device not found: {}", req.device_id),
                    module_ready: false,
                }));
            }
        }

        let mut modules = self.stub_modules.write().await;

        if let Some(module) = modules.get_mut(&req.module_id) {
            let type_info = get_static_module_type_info(&module.type_id);
            if let Some(info) = type_info {
                let valid_role = info.required_roles.iter().any(|r| r.role_id == req.role_id)
                    || info.optional_roles.iter().any(|r| r.role_id == req.role_id);

                if !valid_role {
                    return Ok(Response::new(AssignDeviceResponse {
                        success: false,
                        error_message: format!(
                            "Invalid role '{}' for module type '{}'",
                            req.role_id, module.type_id
                        ),
                        module_ready: false,
                    }));
                }
            }

            module
                .assignments
                .insert(req.role_id.clone(), req.device_id);

            let (filled, total) = module.get_role_status();
            let ready = filled >= total && !module.config.is_empty();

            Ok(Response::new(AssignDeviceResponse {
                success: true,
                error_message: String::new(),
                module_ready: ready,
            }))
        } else {
            Ok(Response::new(AssignDeviceResponse {
                success: false,
                error_message: format!("Module not found: {}", req.module_id),
                module_ready: false,
            }))
        }
    }

    async fn unassign_device(
        &self,
        request: Request<UnassignDeviceRequest>,
    ) -> Result<Response<UnassignDeviceResponse>, Status> {
        let req = request.into_inner();
        let mut modules = self.stub_modules.write().await;

        if let Some(module) = modules.get_mut(&req.module_id) {
            if module.state == ModuleState::ModuleRunning {
                return Ok(Response::new(UnassignDeviceResponse {
                    success: false,
                    error_message: "Cannot unassign device from running module".to_string(),
                }));
            }

            module.assignments.remove(&req.role_id);

            Ok(Response::new(UnassignDeviceResponse {
                success: true,
                error_message: String::new(),
            }))
        } else {
            Ok(Response::new(UnassignDeviceResponse {
                success: false,
                error_message: format!("Module not found: {}", req.module_id),
            }))
        }
    }

    async fn list_assignments(
        &self,
        request: Request<ListAssignmentsRequest>,
    ) -> Result<Response<ListAssignmentsResponse>, Status> {
        let req = request.into_inner();
        let modules = self.stub_modules.read().await;
        let registry = self.device_registry.read().await;

        if let Some(module) = modules.get(&req.module_id) {
            let assignments: Vec<DeviceAssignment> = module
                .assignments
                .iter()
                .map(|(role_id, device_id)| {
                    let device_info = registry.get_device_info(device_id);
                    DeviceAssignment {
                        role_id: role_id.clone(),
                        device_id: device_id.clone(),
                        device_name: device_info
                            .as_ref()
                            .map(|i| i.name.clone())
                            .unwrap_or_else(|| device_id.clone()),
                        device_online: device_info.is_some(),
                    }
                })
                .collect();

            Ok(Response::new(ListAssignmentsResponse { assignments }))
        } else {
            Err(Status::not_found(format!(
                "Module not found: {}",
                req.module_id
            )))
        }
    }

    async fn start_module(
        &self,
        request: Request<StartModuleRequest>,
    ) -> Result<Response<StartModuleResponse>, Status> {
        let req = request.into_inner();
        let mut modules = self.stub_modules.write().await;

        if let Some(module) = modules.get_mut(&req.module_id) {
            if module.state == ModuleState::ModuleRunning {
                return Ok(Response::new(StartModuleResponse {
                    success: false,
                    error_message: "Module is already running".to_string(),
                    start_time_ns: 0,
                }));
            }

            let (filled, total) = module.get_role_status();
            if filled < total {
                return Ok(Response::new(StartModuleResponse {
                    success: false,
                    error_message: format!(
                        "Module not ready: {}/{} required roles filled",
                        filled, total
                    ),
                    start_time_ns: 0,
                }));
            }

            let start_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64;

            module.state = ModuleState::ModuleRunning;
            module.start_time_ns = Some(start_time);
            module.events_emitted = 0;
            module.data_points_produced = 0;

            Ok(Response::new(StartModuleResponse {
                success: true,
                error_message: String::new(),
                start_time_ns: start_time,
            }))
        } else {
            Ok(Response::new(StartModuleResponse {
                success: false,
                error_message: format!("Module not found: {}", req.module_id),
                start_time_ns: 0,
            }))
        }
    }

    async fn pause_module(
        &self,
        request: Request<PauseModuleRequest>,
    ) -> Result<Response<PauseModuleResponse>, Status> {
        let req = request.into_inner();
        let mut modules = self.stub_modules.write().await;

        if let Some(module) = modules.get_mut(&req.module_id) {
            if module.state != ModuleState::ModuleRunning {
                return Ok(Response::new(PauseModuleResponse {
                    success: false,
                    error_message: "Module is not running".to_string(),
                }));
            }

            module.state = ModuleState::ModulePaused;

            Ok(Response::new(PauseModuleResponse {
                success: true,
                error_message: String::new(),
            }))
        } else {
            Ok(Response::new(PauseModuleResponse {
                success: false,
                error_message: format!("Module not found: {}", req.module_id),
            }))
        }
    }

    async fn resume_module(
        &self,
        request: Request<ResumeModuleRequest>,
    ) -> Result<Response<ResumeModuleResponse>, Status> {
        let req = request.into_inner();
        let mut modules = self.stub_modules.write().await;

        if let Some(module) = modules.get_mut(&req.module_id) {
            if module.state != ModuleState::ModulePaused {
                return Ok(Response::new(ResumeModuleResponse {
                    success: false,
                    error_message: "Module is not paused".to_string(),
                }));
            }

            module.state = ModuleState::ModuleRunning;

            Ok(Response::new(ResumeModuleResponse {
                success: true,
                error_message: String::new(),
            }))
        } else {
            Ok(Response::new(ResumeModuleResponse {
                success: false,
                error_message: format!("Module not found: {}", req.module_id),
            }))
        }
    }

    async fn stop_module(
        &self,
        request: Request<StopModuleRequest>,
    ) -> Result<Response<StopModuleResponse>, Status> {
        let req = request.into_inner();
        let mut modules = self.stub_modules.write().await;

        if let Some(module) = modules.get_mut(&req.module_id) {
            if module.state != ModuleState::ModuleRunning
                && module.state != ModuleState::ModulePaused
            {
                return Ok(Response::new(StopModuleResponse {
                    success: false,
                    error_message: "Module is not running or paused".to_string(),
                    uptime_ns: 0,
                    events_emitted: 0,
                }));
            }

            let uptime = module.start_time_ns.map(|start| {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64;
                now.saturating_sub(start)
            });

            let events = module.events_emitted;
            module.state = ModuleState::ModuleStopped;
            module.start_time_ns = None;

            Ok(Response::new(StopModuleResponse {
                success: true,
                error_message: String::new(),
                uptime_ns: uptime.unwrap_or(0),
                events_emitted: events,
            }))
        } else {
            Ok(Response::new(StopModuleResponse {
                success: false,
                error_message: format!("Module not found: {}", req.module_id),
                uptime_ns: 0,
                events_emitted: 0,
            }))
        }
    }

    type StreamModuleEventsStream =
        tokio_stream::wrappers::ReceiverStream<Result<ModuleEvent, Status>>;

    async fn stream_module_events(
        &self,
        request: Request<StreamModuleEventsRequest>,
    ) -> Result<Response<Self::StreamModuleEventsStream>, Status> {
        let req = request.into_inner();

        {
            let modules = self.stub_modules.read().await;
            if !modules.contains_key(&req.module_id) {
                return Err(Status::not_found(format!(
                    "Module not found: {}",
                    req.module_id
                )));
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let module_id = req.module_id.clone();

        // Stub mode: keep channel open but don't emit real events
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if tx.is_closed() {
                    break;
                }
            }
            let _ = module_id;
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    type StreamModuleDataStream =
        tokio_stream::wrappers::ReceiverStream<Result<ModuleDataPoint, Status>>;

    async fn stream_module_data(
        &self,
        request: Request<StreamModuleDataRequest>,
    ) -> Result<Response<Self::StreamModuleDataStream>, Status> {
        let req = request.into_inner();

        {
            let modules = self.stub_modules.read().await;
            if !modules.contains_key(&req.module_id) {
                return Err(Status::not_found(format!(
                    "Module not found: {}",
                    req.module_id
                )));
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let module_id = req.module_id.clone();

        // Stub mode: keep channel open but don't emit real data
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if tx.is_closed() {
                    break;
                }
            }
            let _ = module_id;
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_service() -> ModuleServiceImpl {
        let registry = Arc::new(RwLock::new(DeviceRegistry::new()));
        ModuleServiceImpl::new(registry)
    }

    #[tokio::test]
    async fn test_list_module_types() {
        let service = create_test_service();
        let request = Request::new(ListModuleTypesRequest {
            required_capability: None,
        });

        let response = service.list_module_types(request).await.unwrap();
        let types = response.into_inner().module_types;

        assert!(!types.is_empty());
        assert!(types.iter().any(|t| t.type_id == "power_monitor"));
    }

    #[tokio::test]
    async fn test_get_module_type_info() {
        let service = create_test_service();
        let request = Request::new(GetModuleTypeInfoRequest {
            type_id: "power_monitor".to_string(),
        });

        let response = service.get_module_type_info(request).await.unwrap();
        let info = response.into_inner();

        assert_eq!(info.type_id, "power_monitor");
        assert_eq!(info.required_roles.len(), 1);
        assert_eq!(info.required_roles[0].role_id, "power_meter");
    }

    #[tokio::test]
    async fn test_create_and_delete_module() {
        let service = create_test_service();

        let create_req = Request::new(CreateModuleRequest {
            type_id: "power_monitor".to_string(),
            instance_name: "test_monitor".to_string(),
            initial_config: HashMap::new(),
        });

        let create_resp = service.create_module(create_req).await.unwrap().into_inner();
        assert!(create_resp.success);
        assert!(!create_resp.module_id.is_empty());

        let delete_req = Request::new(DeleteModuleRequest {
            module_id: create_resp.module_id.clone(),
            force: false,
        });

        let delete_resp = service.delete_module(delete_req).await.unwrap().into_inner();
        assert!(delete_resp.success);
    }

    #[tokio::test]
    async fn test_configure_module() {
        let service = create_test_service();

        let create_req = Request::new(CreateModuleRequest {
            type_id: "power_monitor".to_string(),
            instance_name: "test".to_string(),
            initial_config: HashMap::new(),
        });
        let create_resp = service.create_module(create_req).await.unwrap().into_inner();
        let module_id = create_resp.module_id;

        let mut params = HashMap::new();
        params.insert("high_threshold".to_string(), "500.0".to_string());

        let config_req = Request::new(ConfigureModuleRequest {
            module_id: module_id.clone(),
            parameters: params,
            partial: false,
        });

        let config_resp = service
            .configure_module(config_req)
            .await
            .unwrap()
            .into_inner();
        assert!(config_resp.success);

        let get_config_req = Request::new(GetModuleConfigRequest {
            module_id: module_id.clone(),
        });
        let config = service
            .get_module_config(get_config_req)
            .await
            .unwrap()
            .into_inner();
        // Note: stub stores verbatim ("500.0"), real module parses ("500")
        // Compare numerically to work in both modes
        let threshold: f64 = config
            .parameters
            .get("high_threshold")
            .unwrap()
            .parse()
            .unwrap();
        assert!((threshold - 500.0).abs() < 0.001);
    }

    // Lifecycle test requires device assignment when modules feature is enabled.
    // In stub mode, start_module succeeds without devices.
    #[cfg(not(feature = "modules"))]
    #[tokio::test]
    async fn test_module_lifecycle() {
        let service = create_test_service();

        let mut initial_config = HashMap::new();
        initial_config.insert("sample_rate_hz".to_string(), "10.0".to_string());

        let create_req = Request::new(CreateModuleRequest {
            type_id: "multi_channel_logger".to_string(),
            instance_name: "test_logger".to_string(),
            initial_config,
        });
        let create_resp = service.create_module(create_req).await.unwrap().into_inner();
        let module_id = create_resp.module_id;

        let start_req = Request::new(StartModuleRequest {
            module_id: module_id.clone(),
        });
        let start_resp = service.start_module(start_req).await.unwrap().into_inner();
        assert!(start_resp.success);
        assert!(start_resp.start_time_ns > 0);

        let pause_req = Request::new(PauseModuleRequest {
            module_id: module_id.clone(),
        });
        let pause_resp = service.pause_module(pause_req).await.unwrap().into_inner();
        assert!(pause_resp.success);

        let resume_req = Request::new(ResumeModuleRequest {
            module_id: module_id.clone(),
        });
        let resume_resp = service.resume_module(resume_req).await.unwrap().into_inner();
        assert!(resume_resp.success);

        let stop_req = Request::new(StopModuleRequest {
            module_id: module_id.clone(),
            force: false,
        });
        let stop_resp = service.stop_module(stop_req).await.unwrap().into_inner();
        assert!(stop_resp.success);
    }
}
