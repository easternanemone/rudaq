//! gRPC HealthService implementation (bd-pauy)
//!
//! Provides remote monitoring of system health for headless operation.

use crate::grpc::proto::{
    DeviceHealthEntry, DeviceHealthLevel, ErrorSeverityLevel, GetDeviceHealthRequest,
    GetDeviceHealthResponse, GetErrorHistoryRequest, GetErrorHistoryResponse,
    GetModuleHealthRequest, GetModuleHealthResponse, GetSystemHealthRequest,
    GetSystemHealthResponse, HealthErrorRecord, HealthUpdate,
    ModuleHealthStatus as ProtoModuleHealthStatus, StreamHealthUpdatesRequest,
    SystemHealthStatus as ProtoSystemHealthStatus, health_service_server::HealthService,
};
use common::health::{DeviceHealth, ErrorSeverity, SystemHealth, SystemHealthMonitor};
use common::limits::HEALTH_CHECK_INTERVAL;
use hardware::DeviceRegistry;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::interval;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::IntervalStream;
use tonic::{Request, Response, Status};

/// gRPC service for health monitoring
pub struct HealthServiceImpl {
    monitor: Arc<SystemHealthMonitor>,
    registry: Option<Arc<DeviceRegistry>>,
    /// Whether SurrealDB initialized successfully (bd-9n9k.3).
    db_available: bool,
    /// Whether ConfigService is registered in the gRPC server (bd-9n9k.3).
    config_service_available: bool,
    /// DB engine identifier (e.g. "rocksdb", "mem") when DB is available (bd-9n9k.3).
    db_engine: Option<String>,
    /// Human-readable DB state message (bd-9n9k.3).
    db_state_message: Option<String>,
}

impl HealthServiceImpl {
    /// Create a new HealthService with the given monitor
    pub fn new(monitor: Arc<SystemHealthMonitor>) -> Self {
        Self {
            monitor,
            registry: None,
            db_available: false,
            config_service_available: false,
            db_engine: None,
            db_state_message: None,
        }
    }

    /// Attach a device registry for per-device health reporting (bd-qa36.4.3).
    pub fn with_registry(mut self, registry: Arc<DeviceRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Set database availability and metadata (bd-9n9k.3).
    pub fn with_db_state(
        mut self,
        available: bool,
        engine: Option<String>,
        message: Option<String>,
    ) -> Self {
        self.db_available = available;
        self.config_service_available = available;
        self.db_engine = engine;
        self.db_state_message = message;
        self
    }
}

/// Convert Rust Instant to nanoseconds since UNIX epoch
fn instant_to_ns(instant: std::time::Instant) -> u64 {
    // We need to convert from Instant (monotonic) to SystemTime (wall clock)
    // This is approximate but sufficient for display purposes
    let now_instant = std::time::Instant::now();
    let now_system = SystemTime::now();

    let elapsed = now_instant.duration_since(instant);
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    #[allow(clippy::cast_possible_truncation)]
    let system_time = now_system - elapsed;

    system_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Get current timestamp in nanoseconds
fn now_ns() -> u64 {
    common::time::now_ns()
}

/// Convert SystemHealth to proto enum
fn system_health_to_proto(health: SystemHealth) -> ProtoSystemHealthStatus {
    match health {
        SystemHealth::Healthy => ProtoSystemHealthStatus::SystemHealthHealthy,
        SystemHealth::Degraded => ProtoSystemHealthStatus::SystemHealthDegraded,
        SystemHealth::Critical => ProtoSystemHealthStatus::SystemHealthCritical,
    }
}

/// Convert a DeviceHealthState into a proto DeviceHealthEntry.
fn health_state_to_proto(
    device_id: &str,
    state: &common::health::DeviceHealthState,
) -> DeviceHealthEntry {
    DeviceHealthEntry {
        device_id: device_id.to_string(),
        health: device_health_to_proto(state.health) as i32,
        consecutive_failures: state.consecutive_failures,
        restart_attempts: state.restart_attempts,
        last_error: state.last_error.clone(),
        state_since_ns: instant_to_ns(state.state_since),
        last_failure_ns: state.last_failure.map(instant_to_ns),
    }
}

/// Convert DeviceHealth to proto enum
fn device_health_to_proto(health: DeviceHealth) -> DeviceHealthLevel {
    match health {
        DeviceHealth::Healthy => DeviceHealthLevel::DeviceHealthHealthy,
        DeviceHealth::Degraded => DeviceHealthLevel::DeviceHealthDegraded,
        DeviceHealth::Faulted => DeviceHealthLevel::DeviceHealthFaulted,
        DeviceHealth::Recovering => DeviceHealthLevel::DeviceHealthRecovering,
    }
}

/// Convert ErrorSeverity to proto enum
fn error_severity_to_proto(severity: ErrorSeverity) -> ErrorSeverityLevel {
    match severity {
        ErrorSeverity::Info => ErrorSeverityLevel::ErrorSeverityInfo,
        ErrorSeverity::Warning => ErrorSeverityLevel::ErrorSeverityWarning,
        ErrorSeverity::Error => ErrorSeverityLevel::ErrorSeverityError,
        ErrorSeverity::Critical => ErrorSeverityLevel::ErrorSeverityCritical,
    }
}

/// Convert proto ErrorSeverityLevel to ErrorSeverity
fn proto_to_error_severity(level: ErrorSeverityLevel) -> ErrorSeverity {
    match level {
        ErrorSeverityLevel::ErrorSeverityInfo => ErrorSeverity::Info,
        ErrorSeverityLevel::ErrorSeverityWarning => ErrorSeverity::Warning,
        ErrorSeverityLevel::ErrorSeverityError => ErrorSeverity::Error,
        ErrorSeverityLevel::ErrorSeverityCritical => ErrorSeverity::Critical,
        _ => ErrorSeverity::Info,
    }
}

#[tonic::async_trait]
impl HealthService for HealthServiceImpl {
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    async fn get_system_health(
        &self,
        _request: Request<GetSystemHealthRequest>,
    ) -> Result<Response<GetSystemHealthResponse>, Status> {
        let health_status = self.monitor.get_system_health().await;
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let modules = self.monitor.get_module_health().await;
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let errors = self.monitor.get_error_history(None).await;

        let healthy_count = modules.iter().filter(|m| m.is_healthy).count() as u32;
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let unhealthy_count = (modules.len() as u32).saturating_sub(healthy_count);

        let critical_count = errors
            .iter()
            .filter(|e| e.severity == ErrorSeverity::Critical)
            .count() as u32;

        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let response = GetSystemHealthResponse {
            status: system_health_to_proto(health_status) as i32,
            total_modules: modules.len() as u32,
            healthy_modules: healthy_count,
            unhealthy_modules: unhealthy_count,
            total_errors: errors.len() as u32,
            critical_errors: critical_count,
            timestamp_ns: now_ns(),
            // Database readiness (bd-9n9k.3)
            db_available: self.db_available,
            config_service_available: self.config_service_available,
            db_engine: self.db_engine.clone(),
            db_state_message: self.db_state_message.clone(),
        };

        Ok(Response::new(response))
    }

    async fn get_module_health(
        &self,
        _request: Request<GetModuleHealthRequest>,
    ) -> Result<Response<GetModuleHealthResponse>, Status> {
        let modules = self.monitor.get_module_health().await;
        let now = std::time::Instant::now();

        let proto_modules = modules
            .iter()
            .map(|m| {
                let seconds_since = now.duration_since(m.last_heartbeat).as_secs();
                ProtoModuleHealthStatus {
                    name: m.name.clone(),
                    is_healthy: m.is_healthy,
                    last_heartbeat_ns: instant_to_ns(m.last_heartbeat),
                    seconds_since_heartbeat: seconds_since,
                    status_message: m.status_message.clone(),
                }
            })
            .collect();

        let response = GetModuleHealthResponse {
            modules: proto_modules,
            timestamp_ns: now_ns(),
        };

        Ok(Response::new(response))
    }

    async fn get_device_health(
        &self,
        request: Request<GetDeviceHealthRequest>,
    ) -> Result<Response<GetDeviceHealthResponse>, Status> {
        let Some(registry) = &self.registry else {
            return Err(Status::unavailable(
                "Device registry not available for health reporting",
            ));
        };

        let req = request.into_inner();

        let entries: Vec<DeviceHealthEntry> = if let Some(device_id) = req.device_id {
            // Single device query
            match registry.get_device_health(&device_id) {
                Some(state) => vec![health_state_to_proto(&device_id, &state)],
                None => {
                    return Err(Status::not_found(format!(
                        "Device '{}' not found",
                        device_id
                    )));
                }
            }
        } else {
            // All devices
            registry
                .list_device_health()
                .into_iter()
                .map(|(id, state)| health_state_to_proto(&id, &state))
                .collect()
        };

        let response = GetDeviceHealthResponse {
            devices: entries,
            timestamp_ns: now_ns(),
        };

        Ok(Response::new(response))
    }

    async fn get_error_history(
        &self,
        request: Request<GetErrorHistoryRequest>,
    ) -> Result<Response<GetErrorHistoryResponse>, Status> {
        let req = request.into_inner();

        let limit = Some(req.limit.filter(|&l| l > 0).map_or(100, |l| l as usize));

        let errors = if let Some(module_name) = req.module_name {
            self.monitor.get_module_errors(&module_name, limit).await
        } else {
            self.monitor.get_error_history(limit).await
        };

        // Filter by severity if requested
        let min_severity = req
            .min_severity
            .and_then(|level| ErrorSeverityLevel::try_from(level).ok())
            .map(proto_to_error_severity)
            .unwrap_or(ErrorSeverity::Info);

        let filtered_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.severity >= min_severity)
            .collect();

        let proto_errors = filtered_errors
            .iter()
            .map(|e| HealthErrorRecord {
                module_name: e.module_name.clone(),
                severity: error_severity_to_proto(e.severity) as i32,
                message: e.message.clone(),
                timestamp_ns: instant_to_ns(e.timestamp),
                context: e.context.clone(),
            })
            .collect();

        let total_count = self.monitor.error_count().await;

        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let response = GetErrorHistoryResponse {
            errors: proto_errors,
            total_error_count: total_count as u32,
            timestamp_ns: now_ns(),
        };

        Ok(Response::new(response))
    }

    type StreamHealthUpdatesStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<HealthUpdate, Status>> + Send>>;

    async fn stream_health_updates(
        &self,
        request: Request<StreamHealthUpdatesRequest>,
    ) -> Result<Response<<Self as HealthService>::StreamHealthUpdatesStream>, Status> {
        let req = request.into_inner();
        let update_interval = if req.update_interval_ms > 0 {
            Duration::from_millis(u64::from(req.update_interval_ms))
        } else {
            HEALTH_CHECK_INTERVAL // Default 5 seconds
        };

        let monitor = self.monitor.clone();
        let stream_db_available = self.db_available;
        let stream_config_available = self.config_service_available;

        let stream = IntervalStream::new(interval(update_interval)).then(move |_| {
            let monitor = monitor.clone();
            async move {
                let health_status = monitor.get_system_health().await;
                let modules = monitor.get_module_health().await;
                let errors = monitor.get_error_history(Some(1)).await;

                let now = std::time::Instant::now();
                let proto_modules = modules
                    .iter()
                    .map(|m| {
                        let seconds_since = now.duration_since(m.last_heartbeat).as_secs();
                        ProtoModuleHealthStatus {
                            name: m.name.clone(),
                            is_healthy: m.is_healthy,
                            last_heartbeat_ns: instant_to_ns(m.last_heartbeat),
                            seconds_since_heartbeat: seconds_since,
                            status_message: m.status_message.clone(),
                        }
                    })
                    .collect();

                let latest_error = errors.first().map(|e| HealthErrorRecord {
                    module_name: e.module_name.clone(),
                    severity: error_severity_to_proto(e.severity) as i32,
                    message: e.message.clone(),
                    timestamp_ns: instant_to_ns(e.timestamp),
                    context: e.context.clone(),
                });

                Ok(HealthUpdate {
                    system_status: system_health_to_proto(health_status) as i32,
                    modules: proto_modules,
                    latest_error,
                    timestamp_ns: now_ns(),
                    // Database readiness (bd-9n9k.3)
                    db_available: stream_db_available,
                    config_service_available: stream_config_available,
                })
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc::proto::health_service_server::HealthService as HealthServiceTrait;
    use common::health::HealthMonitorConfig;

    fn make_monitor() -> Arc<SystemHealthMonitor> {
        Arc::new(SystemHealthMonitor::new(HealthMonitorConfig::default()))
    }

    #[tokio::test]
    async fn test_db_available_reported_in_health() {
        let svc = HealthServiceImpl::new(make_monitor()).with_db_state(
            true,
            Some("rocksdb".to_string()),
            Some("schema v7, 3 drivers".to_string()),
        );

        let resp = svc
            .get_system_health(Request::new(GetSystemHealthRequest {}))
            .await
            .expect("get_system_health should succeed");
        let body = resp.into_inner();

        assert!(body.db_available, "db_available should be true");
        assert!(
            body.config_service_available,
            "config_service_available should be true"
        );
        assert_eq!(body.db_engine.as_deref(), Some("rocksdb"));
        assert!(
            body.db_state_message
                .as_ref()
                .is_some_and(|m| m.contains("schema v7"))
        );
    }

    #[tokio::test]
    async fn test_db_unavailable_reported_in_health() {
        let svc = HealthServiceImpl::new(make_monitor()).with_db_state(
            false,
            None,
            Some("Database initialization failed".to_string()),
        );

        let resp = svc
            .get_system_health(Request::new(GetSystemHealthRequest {}))
            .await
            .expect("get_system_health should succeed");
        let body = resp.into_inner();

        assert!(!body.db_available, "db_available should be false");
        assert!(
            !body.config_service_available,
            "config_service_available should be false"
        );
        assert!(body.db_engine.is_none());
        assert!(
            body.db_state_message
                .as_ref()
                .is_some_and(|m| m.contains("failed"))
        );
    }

    #[tokio::test]
    async fn test_db_failure_degrades_system_health() {
        let monitor = make_monitor();

        // Simulate DB init failure recording
        monitor
            .report_error(
                "database",
                ErrorSeverity::Warning,
                "SurrealDB init failed: connection refused",
                vec![("engine", "rocksdb")],
            )
            .await;

        let health = monitor.get_system_health().await;
        assert!(
            matches!(health, SystemHealth::Degraded),
            "System health should be Degraded after DB warning, got {:?}",
            health
        );
    }

    #[tokio::test]
    async fn test_default_db_state_is_unavailable() {
        // Without calling with_db_state, DB should report as unavailable
        let svc = HealthServiceImpl::new(make_monitor());

        let resp = svc
            .get_system_health(Request::new(GetSystemHealthRequest {}))
            .await
            .expect("get_system_health should succeed");
        let body = resp.into_inner();

        assert!(!body.db_available, "default db_available should be false");
        assert!(
            !body.config_service_available,
            "default config_service_available should be false"
        );
    }
}
