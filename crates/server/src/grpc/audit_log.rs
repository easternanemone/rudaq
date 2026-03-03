//! Audit logging layer for hardware-mutating gRPC operations (bd-1afe.10)
//!
//! Provides a Tower middleware that intercepts hardware-mutating gRPC calls and
//! emits structured audit log entries to the `audit` tracing target. This enables
//! post-incident analysis in lab environments with Class 4 lasers by recording:
//!
//! - **peer_addr**: Who issued the command (client IP/port)
//! - **timestamp**: When the command was issued (via tracing event timestamp)
//! - **method**: Which gRPC method was called (e.g., `/daq.HardwareService/SetEmission`)
//! - **request_id**: Correlation ID (from `x-request-id` header, or auto-generated UUID)
//!
//! # Routing to a Separate Audit Log File
//!
//! The daemon binary should configure a `tracing_subscriber` layer filtered to
//! `target: "audit"` and backed by a file appender for persistent audit storage.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::LazyLock;
use std::task::{Context, Poll};

use http::{Request, Response};
use tower_layer::Layer;
use tower_service::Service;
use uuid::Uuid;

/// gRPC methods that mutate hardware state and require audit logging.
///
/// Uses `LazyLock<HashSet>` for O(1) lookup on the hot path (35 methods).
/// Covers all operations that can change physical device state, including:
/// - Direct device control (move, trigger, laser, shutter)
/// - Plan/scan execution (start, stop, pause, resume)
/// - NI DAQ analog/digital output
/// - Script execution (can indirectly control hardware)
static MUTATING_METHODS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        // HardwareService — direct device control
        "/daq.HardwareService/MoveAbsolute",
        "/daq.HardwareService/MoveRelative",
        "/daq.HardwareService/StopMotion",
        "/daq.HardwareService/Arm",
        "/daq.HardwareService/Trigger",
        "/daq.HardwareService/SetExposure",
        "/daq.HardwareService/SetShutter",
        "/daq.HardwareService/SetWavelength",
        "/daq.HardwareService/SetEmission",
        "/daq.HardwareService/StartStream",
        "/daq.HardwareService/StopStream",
        "/daq.HardwareService/StageDevice",
        "/daq.HardwareService/UnstageDevice",
        "/daq.HardwareService/ExecuteDeviceCommand",
        "/daq.HardwareService/SetParameter",
        // RunEngineService — plan execution
        "/daq.RunEngineService/QueuePlan",
        "/daq.RunEngineService/StartEngine",
        "/daq.RunEngineService/PauseEngine",
        "/daq.RunEngineService/ResumeEngine",
        "/daq.RunEngineService/AbortPlan",
        "/daq.RunEngineService/HaltEngine",
        // NiDaqService — analog/digital I/O
        "/daq.ni_daq.NiDaqService/SetAnalogOutput",
        "/daq.ni_daq.NiDaqService/WriteDigitalIO",
        "/daq.ni_daq.NiDaqService/WriteDigitalPort",
        "/daq.ni_daq.NiDaqService/ConfigureAnalogInput",
        "/daq.ni_daq.NiDaqService/ConfigureAnalogOutput",
        "/daq.ni_daq.NiDaqService/ConfigureDigitalIO",
        "/daq.ni_daq.NiDaqService/ConfigureTrigger",
        "/daq.ni_daq.NiDaqService/ArmCounter",
        "/daq.ni_daq.NiDaqService/DisarmCounter",
        "/daq.ni_daq.NiDaqService/ResetCounter",
        "/daq.ni_daq.NiDaqService/ConfigureCounter",
        // ControlService — script execution
        "/daq.ControlService/StartScript",
        "/daq.ControlService/StopScript",
        // ScanService (deprecated but still active)
        "/daq.ScanService/StartScan",
        "/daq.ScanService/StopScan",
        "/daq.ScanService/PauseScan",
        "/daq.ScanService/ResumeScan",
        // ModuleService — module lifecycle
        "/daq.ModuleService/StartModule",
        "/daq.ModuleService/StopModule",
        "/daq.ModuleService/PauseModule",
        "/daq.ModuleService/ResumeModule",
        // StorageService — recording control
        "/daq.StorageService/StartRecording",
        "/daq.StorageService/StopRecording",
    ])
});

/// Returns `true` if the gRPC method path corresponds to a hardware-mutating operation.
fn is_mutating_method(path: &str) -> bool {
    MUTATING_METHODS.contains(path)
}

/// Tower layer that adds audit logging for hardware-mutating gRPC calls.
///
/// Insert after authentication so that only validated requests are audited.
/// Non-mutating calls (reads, health checks) pass through with zero overhead
/// beyond a string comparison against the method list.
#[derive(Clone, Debug, Default)]
pub struct AuditLogLayer;

impl AuditLogLayer {
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for AuditLogLayer {
    type Service = AuditLogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuditLogService { inner }
    }
}

/// Tower service wrapper that intercepts and logs hardware-mutating gRPC calls.
#[derive(Clone, Debug)]
pub struct AuditLogService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for AuditLogService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Future: Send + 'static,
    S::Error: std::fmt::Display + Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let path = req.uri().path().to_owned();

        // Fast path: skip audit overhead for non-mutating methods (reads, health checks)
        if !is_mutating_method(&path) {
            return Box::pin(self.inner.call(req));
        }

        // Extract peer address from tonic's TCP connection info
        let peer_addr = req
            .extensions()
            .get::<tonic::transport::server::TcpConnectInfo>()
            .and_then(|info| info.remote_addr())
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| "unknown".into());

        // Use client-provided request ID for correlation, or generate one
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let start = std::time::Instant::now();

        tracing::info!(
            target: "audit",
            grpc_method = %path,
            peer_addr = %peer_addr,
            request_id = %request_id,
            "hardware-mutating gRPC call"
        );

        let fut = self.inner.call(req);

        Box::pin(async move {
            let result = fut.await;
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: value is bounded and fits in target type
            let elapsed_ms = start.elapsed().as_millis() as u64;

            match &result {
                Ok(response) => {
                    let http_status = response.status().as_u16();
                    tracing::info!(
                        target: "audit",
                        grpc_method = %path,
                        peer_addr = %peer_addr,
                        request_id = %request_id,
                        http_status = http_status,
                        elapsed_ms = elapsed_ms,
                        "hardware-mutating gRPC call completed"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        target: "audit",
                        grpc_method = %path,
                        peer_addr = %peer_addr,
                        request_id = %request_id,
                        error = %err,
                        elapsed_ms = elapsed_ms,
                        "hardware-mutating gRPC call failed (transport error)"
                    );
                }
            }

            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_methods_detected() {
        // Direct device control
        assert!(is_mutating_method("/daq.HardwareService/MoveAbsolute"));
        assert!(is_mutating_method("/daq.HardwareService/SetEmission"));
        assert!(is_mutating_method("/daq.HardwareService/SetShutter"));
        assert!(is_mutating_method("/daq.HardwareService/SetWavelength"));
        assert!(is_mutating_method("/daq.HardwareService/Trigger"));
        // Plan execution
        assert!(is_mutating_method("/daq.RunEngineService/StartEngine"));
        assert!(is_mutating_method("/daq.RunEngineService/AbortPlan"));
        // NI DAQ
        assert!(is_mutating_method(
            "/daq.ni_daq.NiDaqService/SetAnalogOutput"
        ));
        assert!(is_mutating_method(
            "/daq.ni_daq.NiDaqService/WriteDigitalIO"
        ));
        // Scripts
        assert!(is_mutating_method("/daq.ControlService/StartScript"));
        // Modules
        assert!(is_mutating_method("/daq.ModuleService/StartModule"));
        // Storage
        assert!(is_mutating_method("/daq.StorageService/StartRecording"));
    }

    #[test]
    fn read_only_methods_not_flagged() {
        assert!(!is_mutating_method("/daq.HardwareService/ListDevices"));
        assert!(!is_mutating_method("/daq.HardwareService/GetDeviceState"));
        assert!(!is_mutating_method("/daq.HardwareService/ReadValue"));
        assert!(!is_mutating_method("/grpc.health.v1.Health/Check"));
        assert!(!is_mutating_method("/daq.PresetService/ListPresets"));
        assert!(!is_mutating_method(
            "/daq.StorageService/GetRecordingStatus"
        ));
        assert!(!is_mutating_method("/daq.RunEngineService/GetEngineStatus"));
    }

    #[test]
    fn empty_and_invalid_paths_not_flagged() {
        assert!(!is_mutating_method(""));
        assert!(!is_mutating_method("/"));
        assert!(!is_mutating_method("not-a-grpc-path"));
        assert!(!is_mutating_method("/daq.HardwareService/"));
        assert!(!is_mutating_method("MoveAbsolute"));
    }
}
