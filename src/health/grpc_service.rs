//! gRPC HealthService implementation (bd-pauy)
//!
//! Provides remote monitoring of system health for headless operation.

use crate::grpc::proto::{
    health_service_server::HealthService, ErrorSeverityLevel, GetErrorHistoryRequest,
    GetErrorHistoryResponse, GetModuleHealthRequest, GetModuleHealthResponse,
    GetSystemHealthRequest, GetSystemHealthResponse, HealthErrorRecord, HealthUpdate,
    ModuleHealthStatus as ProtoModuleHealthStatus, StreamHealthUpdatesRequest,
    SystemHealthStatus as ProtoSystemHealthStatus,
};
use crate::health::{ErrorSeverity, SystemHealth, SystemHealthMonitor};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

/// gRPC service for health monitoring
pub struct HealthServiceImpl {
    monitor: Arc<SystemHealthMonitor>,
}

impl HealthServiceImpl {
    /// Create a new HealthService with the given monitor
    pub fn new(monitor: Arc<SystemHealthMonitor>) -> Self {
        Self { monitor }
    }
}

/// Convert Rust Instant to nanoseconds since UNIX epoch
fn instant_to_ns(instant: std::time::Instant) -> u64 {
    // We need to convert from Instant (monotonic) to SystemTime (wall clock)
    // This is approximate but sufficient for display purposes
    let now_instant = std::time::Instant::now();
    let now_system = SystemTime::now();

    let elapsed = now_instant.duration_since(instant);
    let system_time = now_system - elapsed;

    system_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Get current timestamp in nanoseconds
fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Convert SystemHealth to proto enum
fn system_health_to_proto(health: SystemHealth) -> ProtoSystemHealthStatus {
    match health {
        SystemHealth::Healthy => ProtoSystemHealthStatus::SystemHealthHealthy,
        SystemHealth::Degraded => ProtoSystemHealthStatus::SystemHealthDegraded,
        SystemHealth::Critical => ProtoSystemHealthStatus::SystemHealthCritical,
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
    async fn get_system_health(
        &self,
        _request: Request<GetSystemHealthRequest>,
    ) -> Result<Response<GetSystemHealthResponse>, Status> {
        let health_status = self.monitor.get_system_health().await;
        let modules = self.monitor.get_module_health().await;
        let errors = self.monitor.get_error_history(None).await;

        let healthy_count = modules.iter().filter(|m| m.is_healthy).count() as u32;
        let unhealthy_count = (modules.len() as u32).saturating_sub(healthy_count);

        let critical_count = errors
            .iter()
            .filter(|e| e.severity == ErrorSeverity::Critical)
            .count() as u32;

        let response = GetSystemHealthResponse {
            status: system_health_to_proto(health_status) as i32,
            total_modules: modules.len() as u32,
            healthy_modules: healthy_count,
            unhealthy_modules: unhealthy_count,
            total_errors: errors.len() as u32,
            critical_errors: critical_count,
            timestamp_ns: now_ns(),
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

    async fn get_error_history(
        &self,
        request: Request<GetErrorHistoryRequest>,
    ) -> Result<Response<GetErrorHistoryResponse>, Status> {
        let req = request.into_inner();

        let limit = if req.limit.unwrap_or(0) > 0 {
            Some(req.limit.unwrap() as usize)
        } else {
            Some(100) // Default limit
        };

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
            Duration::from_millis(req.update_interval_ms as u64)
        } else {
            Duration::from_secs(5) // Default 5 seconds
        };

        let monitor = self.monitor.clone();

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
                })
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }
}
