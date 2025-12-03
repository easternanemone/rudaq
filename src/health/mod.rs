//! System health monitoring (bd-pauy)
//!
//! This module provides health monitoring capabilities for headless operation,
//! preventing silent failures in background tasks.

pub mod monitor;

#[cfg(feature = "networking")]
pub mod grpc_service;

pub use monitor::{
    ErrorSeverity, HealthError, HealthMonitorConfig, ModuleHealth, SystemHealth,
    SystemHealthMonitor,
};
