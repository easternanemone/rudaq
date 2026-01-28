//! Native plugin support for dynamically loaded modules.
//!
//! This module provides the bridge between `daq-plugin-api`'s FFI types
//! and the internal `Module` trait used by `ModuleRegistry`.

use crate::modules::{Module, ModuleContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use common::modules::{
    ModuleDataPoint, ModuleEvent, ModuleEventSeverity, ModuleParameter, ModuleRole, ModuleState,
    ModuleTypeInfo,
};
use daq_plugin_api::prelude::*;
use std::collections::HashMap;
use std::sync::Mutex;

// =============================================================================
// Type Conversions: FFI -> Internal
// =============================================================================

fn convert_state(state: FfiModuleState) -> ModuleState {
    match state {
        FfiModuleState::Unknown => ModuleState::Unknown,
        FfiModuleState::Created => ModuleState::Created,
        FfiModuleState::Configured => ModuleState::Configured,
        FfiModuleState::Staged => ModuleState::Staged,
        FfiModuleState::Running => ModuleState::Running,
        FfiModuleState::Paused => ModuleState::Paused,
        FfiModuleState::Stopped => ModuleState::Stopped,
        FfiModuleState::Error => ModuleState::Error,
    }
}

fn convert_role(role: &FfiModuleRole) -> ModuleRole {
    ModuleRole {
        role_id: role.role_id.to_string(),
        description: role.description.to_string(),
        display_name: role.display_name.to_string(),
        required_capability: role.required_capability.to_string(),
        allows_multiple: role.allows_multiple,
    }
}

fn convert_parameter(param: &FfiModuleParameter) -> ModuleParameter {
    ModuleParameter {
        param_id: param.param_id.to_string(),
        display_name: param.display_name.to_string(),
        description: param.description.to_string(),
        param_type: param.param_type.to_string(),
        default_value: param.default_value.to_string(),
        min_value: param
            .min_value
            .as_ref()
            .map(|v| v.to_string())
            .into_option(),
        max_value: param
            .max_value
            .as_ref()
            .map(|v| v.to_string())
            .into_option(),
        enum_values: param.enum_values.iter().map(|v| v.to_string()).collect(),
        units: param.units.to_string(),
        required: param.required,
    }
}

fn convert_type_info(info: &FfiModuleTypeInfo) -> ModuleTypeInfo {
    ModuleTypeInfo {
        type_id: info.type_id.to_string(),
        display_name: info.display_name.to_string(),
        description: info.description.to_string(),
        version: info.version.to_string(),
        parameters: info.parameters.iter().map(convert_parameter).collect(),
        event_types: info.event_types.iter().map(|s| s.to_string()).collect(),
        data_types: info.data_types.iter().map(|s| s.to_string()).collect(),
        required_roles: info.required_roles.iter().map(convert_role).collect(),
        optional_roles: info.optional_roles.iter().map(convert_role).collect(),
    }
}

fn severity_from_u8(value: u8) -> ModuleEventSeverity {
    match value {
        1 => ModuleEventSeverity::Info,
        2 => ModuleEventSeverity::Warning,
        3 => ModuleEventSeverity::Error,
        4 => ModuleEventSeverity::Critical,
        _ => ModuleEventSeverity::Unknown,
    }
}

fn convert_rhashmap_strings(map: &RHashMap<RString, RString>) -> HashMap<String, String> {
    map.iter()
        .map(|tuple| (tuple.0.to_string(), tuple.1.to_string()))
        .collect()
}

fn convert_rhashmap_f64(map: &RHashMap<RString, f64>) -> HashMap<String, f64> {
    map.iter()
        .map(|tuple| (tuple.0.to_string(), *tuple.1))
        .collect()
}

#[allow(dead_code)]
fn convert_ffi_event(event: &FfiModuleEvent, module_id: &str) -> ModuleEvent {
    ModuleEvent {
        module_id: module_id.to_string(),
        event_type: event.event_type.to_string(),
        timestamp_ns: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
        severity: severity_from_u8(event.severity),
        message: event.message.to_string(),
        data: convert_rhashmap_strings(&event.data),
    }
}

#[allow(dead_code)]
fn convert_ffi_data(data: &FfiModuleDataPoint, module_id: &str) -> ModuleDataPoint {
    ModuleDataPoint {
        module_id: module_id.to_string(),
        data_type: data.data_type.to_string(),
        timestamp_ns: data.timestamp_ns,
        values: convert_rhashmap_f64(&data.values),
        metadata: convert_rhashmap_strings(&data.metadata),
    }
}

// =============================================================================
// FfiModuleWrapper
// =============================================================================

/// Wrapper that adapts an FFI module to the internal `Module` trait.
pub struct FfiModuleWrapper {
    inner: Mutex<ModuleFfiBox>,
    type_info: ModuleTypeInfo,
    #[allow(dead_code)]
    module_id: String,
}

impl FfiModuleWrapper {
    pub fn new(inner: ModuleFfiBox, module_id: String) -> Self {
        let type_info = convert_type_info(&inner.type_info());
        Self {
            inner: Mutex::new(inner),
            type_info,
            module_id,
        }
    }

    fn convert_result<T>(result: RResult<T, RString>) -> Result<T> {
        match result {
            RResult::ROk(v) => Ok(v),
            RResult::RErr(e) => Err(anyhow!("{}", e)),
        }
    }

    fn to_ffi_config(params: &HashMap<String, String>) -> FfiModuleConfig {
        params
            .iter()
            .map(|(k, v)| (RString::from(k.as_str()), RString::from(v.as_str())))
            .collect()
    }

    fn from_ffi_config(config: &FfiModuleConfig) -> HashMap<String, String> {
        convert_rhashmap_strings(config)
    }

    fn to_ffi_context(ctx: &ModuleContext) -> FfiModuleContext {
        FfiModuleContext {
            module_id: RString::from(ctx.module_id.as_str()),
            assignments: RHashMap::new(),
            host_context: 0,
        }
    }
}

#[async_trait]
impl Module for FfiModuleWrapper {
    fn type_info() -> ModuleTypeInfo
    where
        Self: Sized,
    {
        panic!("FfiModuleWrapper::type_info() should not be called - use instance type_info")
    }

    fn type_id(&self) -> &str {
        &self.type_info.type_id
    }

    fn configure(&mut self, params: HashMap<String, String>) -> Result<Vec<String>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| anyhow!("Lock poisoned: {}", e))?;
        let ffi_config = Self::to_ffi_config(&params);
        let result = inner.configure(ffi_config);
        let warnings = Self::convert_result(result)?;
        Ok(warnings.iter().map(|s| s.to_string()).collect())
    }

    fn get_config(&self) -> HashMap<String, String> {
        let inner = self.inner.lock().expect("Lock poisoned");
        Self::from_ffi_config(&inner.get_config())
    }

    async fn stage(&mut self, ctx: &ModuleContext) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| anyhow!("Lock poisoned: {}", e))?;
        let ffi_ctx = Self::to_ffi_context(ctx);
        Self::convert_result(inner.stage(&ffi_ctx))
    }

    async fn unstage(&mut self, ctx: &ModuleContext) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| anyhow!("Lock poisoned: {}", e))?;
        let ffi_ctx = Self::to_ffi_context(ctx);
        Self::convert_result(inner.unstage(&ffi_ctx))
    }

    async fn start(&mut self, ctx: ModuleContext) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| anyhow!("Lock poisoned: {}", e))?;
        let ffi_ctx = Self::to_ffi_context(&ctx);
        Self::convert_result(inner.start(ffi_ctx))?;
        Ok(())
    }

    async fn pause(&mut self) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| anyhow!("Lock poisoned: {}", e))?;
        Self::convert_result(inner.pause())
    }

    async fn resume(&mut self) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| anyhow!("Lock poisoned: {}", e))?;
        Self::convert_result(inner.resume())
    }

    async fn stop(&mut self) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| anyhow!("Lock poisoned: {}", e))?;
        Self::convert_result(inner.stop())
    }

    fn state(&self) -> ModuleState {
        let inner = self.inner.lock().expect("Lock poisoned");
        convert_state(inner.state())
    }
}

impl std::fmt::Debug for FfiModuleWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FfiModuleWrapper")
            .field("type_info", &self.type_info)
            .field("module_id", &self.module_id)
            .finish()
    }
}

// =============================================================================
// PluginModuleFactory
// =============================================================================

/// Factory for creating wrapped FFI modules from a plugin.
pub struct PluginModuleFactory {
    plugin_id: String,
    type_id: String,
    type_info: ModuleTypeInfo,
}

impl PluginModuleFactory {
    pub fn new(plugin_id: String, type_id: String, ffi_type_info: &FfiModuleTypeInfo) -> Self {
        Self {
            plugin_id,
            type_id,
            type_info: convert_type_info(ffi_type_info),
        }
    }

    #[allow(dead_code)]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    #[allow(dead_code)]
    pub fn type_id(&self) -> &str {
        &self.type_id
    }

    pub fn type_info(&self) -> &ModuleTypeInfo {
        &self.type_info
    }

    #[allow(dead_code)]
    pub fn create(&self, manager: &PluginManager, instance_id: String) -> Result<FfiModuleWrapper> {
        let plugin = manager
            .get_plugin(&self.plugin_id)
            .ok_or_else(|| anyhow!("Plugin not loaded: {}", self.plugin_id))?;

        let ffi_module = plugin
            .create_module(&self.type_id)
            .map_err(|e| anyhow!("Failed to create module: {}", e))?;

        Ok(FfiModuleWrapper::new(ffi_module, instance_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_conversion() {
        assert_eq!(convert_state(FfiModuleState::Created), ModuleState::Created);
        assert_eq!(convert_state(FfiModuleState::Running), ModuleState::Running);
        assert_eq!(convert_state(FfiModuleState::Error), ModuleState::Error);
    }

    #[test]
    fn test_severity_conversion() {
        assert_eq!(severity_from_u8(0), ModuleEventSeverity::Unknown);
        assert_eq!(severity_from_u8(1), ModuleEventSeverity::Info);
        assert_eq!(severity_from_u8(2), ModuleEventSeverity::Warning);
        assert_eq!(severity_from_u8(3), ModuleEventSeverity::Error);
        assert_eq!(severity_from_u8(4), ModuleEventSeverity::Critical);
    }
}
