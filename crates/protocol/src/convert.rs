use crate::daq;
use common::modules;

/// Trait for converting proto types to domain types
pub trait ToDomain<T> {
    fn to_domain(self) -> T;
}

// Domain -> Proto: factories are simpler or From impls where allowed

impl From<modules::ModuleState> for daq::ModuleState {
    fn from(state: modules::ModuleState) -> Self {
        match state {
            modules::ModuleState::Unknown => daq::ModuleState::Unspecified,
            modules::ModuleState::Created => daq::ModuleState::ModuleCreated,
            modules::ModuleState::Configured => daq::ModuleState::ModuleConfigured,
            modules::ModuleState::Staged => daq::ModuleState::ModuleStaged,
            modules::ModuleState::Running => daq::ModuleState::ModuleRunning,
            modules::ModuleState::Paused => daq::ModuleState::ModulePaused,
            modules::ModuleState::Stopped => daq::ModuleState::ModuleStopped,
            modules::ModuleState::Error => daq::ModuleState::ModuleError,
        }
    }
}

// Proto -> Domain
impl ToDomain<modules::ModuleState> for daq::ModuleState {
    fn to_domain(self) -> modules::ModuleState {
        match self {
            daq::ModuleState::Unspecified => modules::ModuleState::Unknown,
            daq::ModuleState::ModuleCreated => modules::ModuleState::Created,
            daq::ModuleState::ModuleConfigured => modules::ModuleState::Configured,
            daq::ModuleState::ModuleStaged => modules::ModuleState::Staged,
            daq::ModuleState::ModuleRunning => modules::ModuleState::Running,
            daq::ModuleState::ModulePaused => modules::ModuleState::Paused,
            daq::ModuleState::ModuleStopped => modules::ModuleState::Stopped,
            daq::ModuleState::ModuleError => modules::ModuleState::Error,
        }
    }
}

// ModuleEvent conversion
impl From<modules::ModuleEvent> for daq::ModuleEvent {
    fn from(event: modules::ModuleEvent) -> Self {
        daq::ModuleEvent {
            module_id: event.module_id,
            event_type: event.event_type,
            timestamp_ns: event.timestamp_ns,
            severity: match event.severity {
                modules::ModuleEventSeverity::Unknown => daq::ModuleEventSeverity::Unspecified,
                modules::ModuleEventSeverity::Info => daq::ModuleEventSeverity::Info,
                modules::ModuleEventSeverity::Warning => daq::ModuleEventSeverity::Warning,
                modules::ModuleEventSeverity::Error => daq::ModuleEventSeverity::Error,
                modules::ModuleEventSeverity::Critical => daq::ModuleEventSeverity::Critical,
            } as i32,
            message: event.message,
            data: event.data,
        }
    }
}

impl ToDomain<modules::ModuleEvent> for daq::ModuleEvent {
    fn to_domain(self) -> modules::ModuleEvent {
        modules::ModuleEvent {
            module_id: self.module_id,
            event_type: self.event_type,
            timestamp_ns: self.timestamp_ns,
            severity: match daq::ModuleEventSeverity::try_from(self.severity)
                .unwrap_or(daq::ModuleEventSeverity::Unspecified)
            {
                daq::ModuleEventSeverity::Unspecified => modules::ModuleEventSeverity::Unknown,
                daq::ModuleEventSeverity::Info => modules::ModuleEventSeverity::Info,
                daq::ModuleEventSeverity::Warning => modules::ModuleEventSeverity::Warning,
                daq::ModuleEventSeverity::Error => modules::ModuleEventSeverity::Error,
                daq::ModuleEventSeverity::Critical => modules::ModuleEventSeverity::Critical,
            },
            message: self.message,
            data: self.data,
        }
    }
}

// ModuleDataPoint conversion
impl From<modules::ModuleDataPoint> for daq::ModuleDataPoint {
    fn from(dp: modules::ModuleDataPoint) -> Self {
        daq::ModuleDataPoint {
            module_id: dp.module_id,
            data_type: dp.data_type,
            timestamp_ns: dp.timestamp_ns,
            values: dp.values,
            metadata: dp.metadata,
        }
    }
}

impl ToDomain<modules::ModuleDataPoint> for daq::ModuleDataPoint {
    fn to_domain(self) -> modules::ModuleDataPoint {
        modules::ModuleDataPoint {
            module_id: self.module_id,
            data_type: self.data_type,
            timestamp_ns: self.timestamp_ns,
            values: self.values,
            metadata: self.metadata,
        }
    }
}

// ModuleTypeInfo conversion
impl From<modules::ModuleTypeInfo> for daq::ModuleTypeInfo {
    fn from(info: modules::ModuleTypeInfo) -> Self {
        daq::ModuleTypeInfo {
            type_id: info.type_id,
            display_name: info.display_name,
            description: info.description,
            version: info.version,
            required_roles: info.required_roles.into_iter().map(|r| r.into()).collect(),
            optional_roles: info.optional_roles.into_iter().map(|r| r.into()).collect(),
            parameters: info.parameters.into_iter().map(|p| p.into()).collect(),
            event_types: info.event_types,
            data_types: info.data_types,
        }
    }
}

impl ToDomain<modules::ModuleTypeInfo> for daq::ModuleTypeInfo {
    fn to_domain(self) -> modules::ModuleTypeInfo {
        modules::ModuleTypeInfo {
            type_id: self.type_id,
            display_name: self.display_name,
            description: self.description,
            version: self.version,
            required_roles: self
                .required_roles
                .into_iter()
                .map(|r| r.to_domain())
                .collect(),
            optional_roles: self
                .optional_roles
                .into_iter()
                .map(|r| r.to_domain())
                .collect(),
            parameters: self.parameters.into_iter().map(|p| p.to_domain()).collect(),
            event_types: self.event_types,
            data_types: self.data_types,
        }
    }
}

// ModuleParameter conversion
impl From<modules::ModuleParameter> for daq::ModuleParameter {
    fn from(param: modules::ModuleParameter) -> Self {
        daq::ModuleParameter {
            param_id: param.param_id,
            display_name: param.display_name,
            description: param.description,
            param_type: param.param_type,
            default_value: param.default_value,
            min_value: param.min_value,
            max_value: param.max_value,
            enum_values: param.enum_values,
            units: param.units,
            required: param.required,
        }
    }
}

impl ToDomain<modules::ModuleParameter> for daq::ModuleParameter {
    fn to_domain(self) -> modules::ModuleParameter {
        modules::ModuleParameter {
            param_id: self.param_id,
            display_name: self.display_name,
            description: self.description,
            param_type: self.param_type,
            default_value: self.default_value,
            min_value: self.min_value,
            max_value: self.max_value,
            enum_values: self.enum_values,
            units: self.units,
            required: self.required,
        }
    }
}

// ModuleRole conversion
impl From<modules::ModuleRole> for daq::ModuleRole {
    fn from(role: modules::ModuleRole) -> Self {
        daq::ModuleRole {
            role_id: role.role_id,
            display_name: role.display_name,
            description: role.description,
            required_capability: role.required_capability,
            allows_multiple: role.allows_multiple,
        }
    }
}

impl ToDomain<modules::ModuleRole> for daq::ModuleRole {
    fn to_domain(self) -> modules::ModuleRole {
        modules::ModuleRole {
            role_id: self.role_id,
            display_name: self.display_name,
            description: self.description,
            required_capability: self.required_capability,
            allows_multiple: self.allows_multiple,
        }
    }
}
