//! gRPC client for ModuleService
//! Provides Rust bindings and async methods for module operations

use crate::grpc_client::DaqClient;
use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// Module type information for UI display
#[derive(Clone, Debug)]
pub struct UiModuleType {
    pub type_id: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub required_roles: usize,
    pub optional_roles: usize,
    pub num_parameters: usize,
    pub num_event_types: usize,
    pub categories: String,
}

/// Module instance for UI display
#[derive(Clone, Debug)]
pub struct UiModuleInstance {
    pub module_id: String,
    pub type_id: String,
    pub instance_name: String,
    pub state: String,
    pub roles_filled: usize,
    pub roles_total: usize,
    pub ready_to_start: bool,
    pub uptime_ms: u64,
    pub error_message: String,
}

/// Device role within a module
#[derive(Clone, Debug)]
pub struct UiModuleRole {
    pub role_id: String,
    pub display_name: String,
    pub required_capability: String,
    pub allows_multiple: bool,
    pub assigned_device_id: String,
}

/// Configuration parameter for a module
#[derive(Clone, Debug)]
pub struct UiModuleParameter {
    pub param_id: String,
    pub display_name: String,
    pub description: String,
    pub param_type: String,  // "float", "int", "string", "bool", "enum"
    pub current_value: String,
    pub default_value: String,
    pub min_value: String,
    pub max_value: String,
    pub enum_values: String,  // Comma-separated
    pub units: String,
    pub required: bool,
}

/// Module event for event log display
#[derive(Clone, Debug)]
pub struct UiModuleEvent {
    pub event_id: String,
    pub event_type: String,
    pub timestamp_ms: u64,
    pub message: String,
    pub severity: String,  // "info", "warning", "error"
}

impl DaqClient {
    /// List available module types
    pub async fn list_module_types(&self) -> Result<Vec<UiModuleType>> {
        // This would call the gRPC ModuleService.ListModuleTypes
        // For now, returning a mock list for UI integration
        Ok(vec![
            UiModuleType {
                type_id: "power_monitor".to_string(),
                display_name: "Power Monitor".to_string(),
                description: "Real-time power measurement and threshold monitoring".to_string(),
                version: "1.0".to_string(),
                required_roles: 1,
                optional_roles: 0,
                num_parameters: 3,
                num_event_types: 2,
                categories: "monitoring,threshold".to_string(),
            },
            UiModuleType {
                type_id: "data_logger".to_string(),
                display_name: "Data Logger".to_string(),
                description: "Multi-channel data acquisition and storage".to_string(),
                version: "1.0".to_string(),
                required_roles: 2,
                optional_roles: 1,
                num_parameters: 5,
                num_event_types: 3,
                categories: "logging,acquisition".to_string(),
            },
            UiModuleType {
                type_id: "position_tracker".to_string(),
                display_name: "Position Tracker".to_string(),
                description: "Track and log device positions over time".to_string(),
                version: "1.0".to_string(),
                required_roles: 1,
                optional_roles: 0,
                num_parameters: 2,
                num_event_types: 1,
                categories: "monitoring,tracking".to_string(),
            },
        ])
    }

    /// Get detailed info about a specific module type
    pub async fn get_module_type_info(&self, type_id: &str) -> Result<UiModuleType> {
        // Mock implementation for UI development
        match type_id {
            "power_monitor" => Ok(UiModuleType {
                type_id: "power_monitor".to_string(),
                display_name: "Power Monitor".to_string(),
                description: "Real-time power measurement and threshold monitoring".to_string(),
                version: "1.0".to_string(),
                required_roles: 1,
                optional_roles: 0,
                num_parameters: 3,
                num_event_types: 2,
                categories: "monitoring,threshold".to_string(),
            }),
            "data_logger" => Ok(UiModuleType {
                type_id: "data_logger".to_string(),
                display_name: "Data Logger".to_string(),
                description: "Multi-channel data acquisition and storage".to_string(),
                version: "1.0".to_string(),
                required_roles: 2,
                optional_roles: 1,
                num_parameters: 5,
                num_event_types: 3,
                categories: "logging,acquisition".to_string(),
            }),
            _ => Err(anyhow!("Unknown module type: {}", type_id)),
        }
    }

    /// List all module instances
    pub async fn list_modules(&self) -> Result<Vec<UiModuleInstance>> {
        // Mock implementation for UI development
        Ok(vec![])
    }

    /// Create a new module instance
    pub async fn create_module(&self, type_id: &str, instance_name: &str) -> Result<String> {
        // Returns the new module_id
        Ok(format!("{}-instance-{}", type_id, uuid::Uuid::new_v4()))
    }

    /// Delete a module instance
    pub async fn delete_module(&self, module_id: &str) -> Result<()> {
        Ok(())
    }

    /// Get status of a module instance
    pub async fn get_module_status(&self, module_id: &str) -> Result<UiModuleInstance> {
        Ok(UiModuleInstance {
            module_id: module_id.to_string(),
            type_id: "power_monitor".to_string(),
            instance_name: "Power Monitor 1".to_string(),
            state: "created".to_string(),
            roles_filled: 0,
            roles_total: 1,
            ready_to_start: false,
            uptime_ms: 0,
            error_message: String::new(),
        })
    }

    /// Get module configuration
    pub async fn get_module_config(&self, module_id: &str) -> Result<Vec<UiModuleParameter>> {
        Ok(vec![])
    }

    /// Configure a module parameter
    pub async fn configure_module(
        &self,
        module_id: &str,
        param_id: &str,
        value: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Get module roles
    pub async fn get_module_roles(&self, module_id: &str) -> Result<Vec<UiModuleRole>> {
        Ok(vec![])
    }

    /// Assign a device to a module role
    pub async fn assign_device(&self, module_id: &str, role_id: &str, device_id: &str) -> Result<()> {
        Ok(())
    }

    /// Unassign a device from a module role
    pub async fn unassign_device(&self, module_id: &str, role_id: &str) -> Result<()> {
        Ok(())
    }

    /// Start module execution
    pub async fn start_module(&self, module_id: &str) -> Result<()> {
        Ok(())
    }

    /// Pause module execution
    pub async fn pause_module(&self, module_id: &str) -> Result<()> {
        Ok(())
    }

    /// Resume paused module
    pub async fn resume_module(&self, module_id: &str) -> Result<()> {
        Ok(())
    }

    /// Stop module execution
    pub async fn stop_module(&self, module_id: &str) -> Result<()> {
        Ok(())
    }

    /// Get recent module events
    pub async fn get_module_events(&self, module_id: &str, limit: usize) -> Result<Vec<UiModuleEvent>> {
        Ok(vec![])
    }
}
