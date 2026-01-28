use crate::grpc::proto::run_engine_service_server::RunEngineServiceServer;
use crate::grpc::proto::{DaemonInfoRequest, DaemonInfoResponse, SystemStatus};
#[cfg(feature = "scripting")]
use crate::grpc::proto::{
    ListExecutionsRequest, ListExecutionsResponse, ListScriptsRequest, ListScriptsResponse,
    ScriptInfo, ScriptStatus, StartRequest, StartResponse, StatusRequest, StopRequest,
    StopResponse,
    control_service_server::{ControlService, ControlServiceServer},
};
use crate::grpc::run_engine_service::RunEngineServiceImpl;
#[cfg(feature = "serial")]
use crate::grpc::{PluginServiceImpl, PluginServiceServer};
use common::core::Measurement;
#[cfg(feature = "scripting")]
use common::limits;
#[cfg(feature = "scripting")]
use daq_scripting::ScriptEngine; // Trait import
// use common::error::DaqError; // Unused
#[cfg(feature = "scripting")]
use daq_experiment::RunEngine;
#[cfg(feature = "scripting")]
use daq_scripting::RhaiEngine;
#[cfg(feature = "scripting")]
use daq_scripting::ScriptPlanRunner;
use protocol::daq::{UploadRequest, UploadResponse};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "storage_hdf5")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::System;
#[cfg(feature = "storage_hdf5")]
use tokio::sync::mpsc;
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;
use tonic::service::interceptor::interceptor;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use uuid::Uuid;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use http::HeaderValue;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};

#[cfg(feature = "storage_hdf5")]
use daq_storage::hdf5_writer::HDF5Writer;

#[cfg(feature = "scripting")]
/// Metadata about an uploaded script
#[derive(Clone, Debug)]
struct ScriptMetadata {
    name: String,
    upload_time: u64,
    metadata: HashMap<String, String>,
}

#[cfg(feature = "scripting")]
/// State of a script execution
#[derive(Clone, Debug)]
struct ExecutionState {
    script_id: String,
    state: String,
    start_time: u64,
    end_time: Option<u64>,
    error: Option<String>,
    progress_percent: u32,
    current_line: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
struct GrpcConfigFile {
    grpc: GrpcSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct GrpcSettings {
    tls_cert_path: Option<PathBuf>,
    tls_key_path: Option<PathBuf>,
    auth_enabled: bool,
    auth_token: Option<String>,
    allowed_origins: Vec<String>,
    bind_address: Option<IpAddr>,
}

impl Default for GrpcSettings {
    fn default() -> Self {
        Self {
            tls_cert_path: None,
            tls_key_path: None,
            auth_enabled: false,
            auth_token: None,
            allowed_origins: Vec::new(),
            bind_address: Some(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    exp: Option<usize>,
    iss: Option<String>,
    aud: Option<String>,
    sub: Option<String>,
}

impl GrpcSettings {
    fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_path = PathBuf::from("config/config.v4.toml");
        let mut figment = Figment::from(Serialized::defaults(GrpcConfigFile::default()))
            .merge(Env::prefixed("RUSTDAQ_").split("__"));

        if config_path.exists() {
            figment = figment.merge(Toml::file(&config_path));
        } else {
            eprintln!(
                "⚠️  gRPC config file not found at {} (using defaults/env overrides)",
                config_path.display()
            );
        }

        let settings: GrpcConfigFile = figment.extract()?;
        Ok(settings.grpc)
    }

    fn auth_token(&self) -> Option<&str> {
        self.auth_token
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    fn bind_socket(&self, default_port: u16) -> SocketAddr {
        let bind_ip = self
            .bind_address
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        SocketAddr::new(bind_ip, default_port)
    }
}

fn build_tls_config(
    settings: &GrpcSettings,
) -> Result<Option<ServerTlsConfig>, Box<dyn std::error::Error>> {
    let (cert_path, key_path) = match (&settings.tls_cert_path, &settings.tls_key_path) {
        (Some(cert), Some(key)) => (cert, key),
        (None, None) => return Ok(None),
        _ => {
            return Err("gRPC TLS requires both grpc.tls_cert_path and grpc.tls_key_path".into());
        }
    };

    let cert = std::fs::read(cert_path)
        .map_err(|e| format!("Failed to read TLS cert {}: {}", cert_path.display(), e))?;
    let key = std::fs::read(key_path)
        .map_err(|e| format!("Failed to read TLS key {}: {}", key_path.display(), e))?;
    let identity = Identity::from_pem(cert, key);
    Ok(Some(ServerTlsConfig::new().identity(identity)))
}

fn build_cors_layer(settings: &GrpcSettings) -> Result<CorsLayer, Box<dyn std::error::Error>> {
    let mut cors = CorsLayer::new().allow_headers(Any).allow_methods(Any);

    if settings.allowed_origins.is_empty() {
        eprintln!("⚠️  grpc.allowed_origins is empty; gRPC-web requests will be blocked by CORS");
        cors = cors.allow_origin(AllowOrigin::list(Vec::<HeaderValue>::new()));
    } else {
        let origins = settings
            .allowed_origins
            .iter()
            .map(|origin| HeaderValue::from_str(origin))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Invalid origin in grpc.allowed_origins: {}", e))?;
        cors = cors.allow_origin(AllowOrigin::list(origins));
    }

    Ok(cors)
}

fn validate_auth(settings: &GrpcSettings, request: &Request<()>) -> Result<(), Status> {
    if !settings.auth_enabled {
        return Ok(());
    }

    let expected = settings.auth_token().ok_or_else(|| {
        Status::unauthenticated("auth enabled but grpc.auth_token is not configured")
    })?;

    let metadata = request.metadata();
    let header_token = metadata
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(extract_bearer_token);

    let api_key_header = metadata
        .get("x-api-key")
        .and_then(|value| value.to_str().ok());

    let candidate = header_token.or(api_key_header);
    let token = match candidate {
        Some(token) => token,
        None => return Err(Status::unauthenticated("missing authorization token")),
    };

    if token == expected {
        return Ok(());
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    let decoding_key = DecodingKey::from_secret(expected.as_bytes());
    decode::<JwtClaims>(token, &decoding_key, &validation)
        .map(|_| ())
        .map_err(|_| Status::unauthenticated("invalid authentication token"))
}

fn extract_bearer_token(header_value: &str) -> Option<&str> {
    let trimmed = header_value.trim();
    let mut parts = trimmed.splitn(2, ' ');
    let scheme = parts.next()?;
    let token = parts.next();
    if token.is_none() {
        return Some(trimmed);
    }
    if scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("apikey") {
        return token.map(str::trim);
    }
    Some(trimmed)
}

// DataPoint is imported from crate::measurement_types (see above)

/// DAQ gRPC server implementation
///
/// Provides gRPC services for data acquisition control. When the `scripting` feature is enabled,
/// includes ControlService for script execution and measurement streaming.
pub struct DaqServer {
    #[cfg(feature = "scripting")]
    script_engine: Arc<RwLock<RhaiEngine>>,
    #[cfg(feature = "scripting")]
    scripts: Arc<RwLock<HashMap<String, String>>>,
    #[cfg(feature = "scripting")]
    script_metadata: Arc<RwLock<HashMap<String, ScriptMetadata>>>,
    #[cfg(feature = "scripting")]
    executions: Arc<RwLock<HashMap<String, ExecutionState>>>,
    #[cfg(feature = "scripting")]
    /// JoinHandles for running script tasks, keyed by execution_id.
    /// Used for cancellation - calling abort() on the handle stops the script.
    running_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
    #[cfg(feature = "scripting")]
    /// Shared RunEngine for executing yielded plans from scripts (bd-si2c)
    /// Scripts use ScriptPlanRunner which executes plans through this engine,
    /// ensuring all script operations emit Documents and coordinate with gRPC services.
    run_engine: Arc<RunEngine>,
    start_time: SystemTime,

    /// Broadcast channel for distributing hardware measurements to multiple consumers.
    /// Receivers can be cloned for gRPC clients, storage writers, etc.
    data_tx: Arc<broadcast::Sender<Measurement>>,

    /// Optional ring buffer for persistent storage (only when storage features enabled)
    #[cfg(feature = "storage_hdf5")]
    ring_buffer: Option<Arc<daq_storage::ring_buffer::RingBuffer>>,
}

impl DaqServer {
    /// Create a new DAQ server instance.
    ///
    /// # Arguments
    /// * `ring_buffer` - Optional RingBuffer for persistent data storage (when storage features enabled)
    /// * `run_engine` - Shared RunEngine for coordinating script execution with gRPC services (bd-si2c)
    ///
    /// # Example
    /// ```ignore
    /// // Create shared RunEngine first
    /// let registry = DeviceRegistry::new();
    /// let run_engine = Arc::new(RunEngine::new(Arc::new(registry)));
    ///
    /// // Without storage
    /// let server = DaqServer::new(None, run_engine.clone())?;
    ///
    /// // With storage (requires storage_hdf5 + storage_arrow features)
    /// let ring_buffer = Arc::new(RingBuffer::create(Path::new("/tmp/daq_ring"), 100)?);
    /// let server = DaqServer::new(Some(ring_buffer), run_engine)?;
    /// ```
    #[cfg(all(feature = "storage_hdf5", feature = "scripting"))]
    pub fn new(
        ring_buffer: Option<Arc<daq_storage::ring_buffer::RingBuffer>>,
        run_engine: Arc<RunEngine>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Create broadcast channel for data distribution (capacity 1000 in-flight messages)
        let (data_tx, mut data_rx) = broadcast::channel(1000);
        let data_tx = Arc::new(data_tx);

        // Spawn background task to write data to RingBuffer if provided
        if let Some(rb) = ring_buffer.clone() {
            let rb_chan = std::env::var("DAQ_PIPELINE_RINGBUF_CH")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(512);
            let (rb_tx, mut rb_rx) = mpsc::channel(rb_chan);
            let drop_counter = Arc::new(AtomicU64::new(0));

            // Forward broadcast stream into bounded channel with drop metrics
            tokio::spawn({
                let drop_counter = drop_counter.clone();
                async move {
                    let rb_tx = rb_tx;
                    // Throttle lag warnings (bd-jnfu.15)
                    let mut total_lagged: u64 = 0;
                    loop {
                        match data_rx.recv().await {
                            Ok(data_point) => {
                                if let Err(err) = rb_tx.try_send(data_point) {
                                    if matches!(err, mpsc::error::TrySendError::Full(_)) {
                                        let dropped =
                                            drop_counter.fetch_add(1, Ordering::Relaxed) + 1;
                                        if dropped % 100 == 0 {
                                            tracing::warn!(
                                                dropped = dropped,
                                                "Dropped {} measurements while ring buffer writer was saturated",
                                                dropped
                                            );
                                        }
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                // Throttle lag warnings to every 100 events (bd-jnfu.15)
                                total_lagged += skipped;
                                if total_lagged % 100 == 0 || skipped > 50 {
                                    tracing::warn!(
                                        skipped = total_lagged,
                                        "Measurement stream lagged, total skipped {} messages",
                                        total_lagged
                                    );
                                }
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            });

            tokio::spawn(async move {
                while let Some(measurement) = rb_rx.recv().await {
                    match encode_measurement_frame(&measurement) {
                        Ok(frame) => {
                            if let Err(e) = rb.write(&frame) {
                                tracing::error!(error = %e, "Failed to write measurement to ring buffer");
                            }
                        }
                        Err(e) => tracing::error!(error = %e, "Failed to encode measurement frame"),
                    }

                    // Yield to allow other tasks to run
                    tokio::task::yield_now().await;
                }
            });
        }

        Ok(Self {
            #[cfg(feature = "scripting")]
            script_engine: Arc::new(RwLock::new(RhaiEngine::with_hardware().map_err(|e| {
                format!(
                    "failed to initialize RhaiEngine with hardware bindings: {}",
                    e
                )
            })?)),
            #[cfg(feature = "scripting")]
            scripts: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "scripting")]
            script_metadata: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "scripting")]
            executions: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "scripting")]
            running_tasks: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "scripting")]
            run_engine,
            start_time: SystemTime::now(),
            data_tx,
            #[cfg(feature = "storage_hdf5")]
            ring_buffer,
        })
    }

    /// Create a new DAQ server instance without storage features.
    /// Requires `run_engine` when scripting feature is enabled (bd-si2c).
    #[cfg(all(not(feature = "storage_hdf5"), feature = "scripting"))]
    pub fn new(run_engine: Arc<RunEngine>) -> Result<Self, Box<dyn std::error::Error>> {
        // Create broadcast channel for data distribution
        let (data_tx, _rx) = broadcast::channel(1000);
        let data_tx = Arc::new(data_tx);

        Ok(Self {
            script_engine: Arc::new(RwLock::new(RhaiEngine::with_hardware().map_err(|e| {
                format!(
                    "failed to initialize RhaiEngine with hardware bindings: {}",
                    e
                )
            })?)),
            scripts: Arc::new(RwLock::new(HashMap::new())),
            script_metadata: Arc::new(RwLock::new(HashMap::new())),
            executions: Arc::new(RwLock::new(HashMap::new())),
            running_tasks: Arc::new(RwLock::new(HashMap::new())),
            run_engine,
            start_time: SystemTime::now(),
            data_tx,
        })
    }

    /// Create a new DAQ server instance with storage but no scripting support.
    #[cfg(all(feature = "storage_hdf5", not(feature = "scripting")))]
    pub fn new(
        ring_buffer: Option<Arc<daq_storage::ring_buffer::RingBuffer>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Create broadcast channel for data distribution (capacity 1000 in-flight messages)
        let (data_tx, _rx) = broadcast::channel(1000);
        let data_tx = Arc::new(data_tx);

        Ok(Self {
            start_time: SystemTime::now(),
            data_tx,
            ring_buffer,
        })
    }

    /// Create a new DAQ server instance without storage or scripting features.
    #[cfg(all(not(feature = "storage_hdf5"), not(feature = "scripting")))]
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // Create broadcast channel for data distribution
        let (data_tx, _rx) = broadcast::channel(1000);
        let data_tx = Arc::new(data_tx);

        Ok(Self {
            start_time: SystemTime::now(),
            data_tx,
        })
    }

    /// Get a clone of the data broadcast sender for hardware drivers.
    ///
    /// Hardware drivers should call this during initialization to get a sender
    /// they can use to publish measurements.
    pub fn data_sender(&self) -> Arc<broadcast::Sender<Measurement>> {
        Arc::clone(&self.data_tx)
    }
}

fn encode_measurement_frame(measurement: &Measurement) -> Result<Vec<u8>, bincode::Error> {
    let payload = bincode::serialize(measurement)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

#[cfg(feature = "scripting")]
impl std::fmt::Debug for DaqServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaqServer")
            .field("script_engine", &"<RwLock<RhaiEngine>>")
            .field(
                "scripts",
                &format!(
                    "{} scripts",
                    self.scripts.try_read().map(|s| s.len()).unwrap_or(0)
                ),
            )
            .field(
                "script_metadata",
                &format!(
                    "{} metadata entries",
                    self.script_metadata
                        .try_read()
                        .map(|m| m.len())
                        .unwrap_or(0)
                ),
            )
            .field(
                "executions",
                &format!(
                    "{} executions",
                    self.executions.try_read().map(|e| e.len()).unwrap_or(0)
                ),
            )
            .field(
                "running_tasks",
                &format!(
                    "{} running tasks",
                    self.running_tasks.try_read().map(|t| t.len()).unwrap_or(0)
                ),
            )
            .field("start_time", &self.start_time)
            .field("data_tx", &"<broadcast::Sender>")
            .finish()
    }
}

#[cfg(not(feature = "scripting"))]
impl std::fmt::Debug for DaqServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaqServer")
            .field("start_time", &self.start_time)
            .field("data_tx", &"<broadcast::Sender>")
            .finish()
    }
}

/// Default is only available when scripting is disabled (bd-si2c)
/// When scripting is enabled, you must create a DaqServer with a shared RunEngine
/// using DaqServer::new(run_engine) or DaqServer::new(ring_buffer, run_engine)
#[cfg(not(feature = "scripting"))]
impl Default for DaqServer {
    #[cfg(feature = "storage_hdf5")]
    fn default() -> Self {
        Self::new(None).expect("failed to create DaqServer")
    }

    #[cfg(not(feature = "storage_hdf5"))]
    fn default() -> Self {
        Self::new().expect("failed to create DaqServer")
    }
}

#[cfg(feature = "scripting")]
#[tonic::async_trait]
impl ControlService for DaqServer {
    /// Upload and validate a script
    async fn upload_script(
        &self,
        request: Request<UploadRequest>,
    ) -> Result<Response<UploadResponse>, Status> {
        let req = request.into_inner();
        let script_id = Uuid::new_v4().to_string();

        let script_size = req.script_content.len();
        if script_size > limits::MAX_SCRIPT_SIZE {
            return Ok(Response::new(UploadResponse {
                script_id: String::new(),
                success: false,
                error_message: format!(
                    "Script too large: {} bytes (max {})",
                    script_size,
                    limits::MAX_SCRIPT_SIZE
                ),
            }));
        }

        // Validate script syntax
        let engine = self.script_engine.read().await;
        if let Err(e) = engine.validate_script(&req.script_content).await {
            return Ok(Response::new(UploadResponse {
                script_id: String::new(),
                success: false,
                error_message: format!("Validation failed: {}", e),
            }));
        }

        // Store validated script
        self.scripts
            .write()
            .await
            .insert(script_id.clone(), req.script_content);

        // Store metadata
        self.script_metadata.write().await.insert(
            script_id.clone(),
            ScriptMetadata {
                name: req.name,
                upload_time: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
                metadata: req.metadata,
            },
        );

        Ok(Response::new(UploadResponse {
            script_id,
            success: true,
            error_message: String::new(),
        }))
    }

    /// Start execution of an uploaded script
    async fn start_script(
        &self,
        request: Request<StartRequest>,
    ) -> Result<Response<StartResponse>, Status> {
        let req = request.into_inner();
        let scripts = self.scripts.read().await;

        let script = scripts
            .get(&req.script_id)
            .ok_or_else(|| Status::not_found("Script not found"))?;

        let execution_id = Uuid::new_v4().to_string();

        // Record execution start
        self.executions.write().await.insert(
            execution_id.clone(),
            ExecutionState {
                script_id: req.script_id.clone(),
                state: "RUNNING".to_string(),
                start_time: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
                end_time: None,
                error: None,
                progress_percent: 0,
                current_line: String::new(),
            },
        );

        // Execute script via ScriptPlanRunner using shared RunEngine (bd-si2c)
        // This ensures all yielded plans emit Documents and coordinate with gRPC services
        let script_clone = script.clone();
        let run_engine_clone = self.run_engine.clone();
        let executions_clone = self.executions.clone();
        let exec_id_clone = execution_id.clone();
        let running_tasks_clone = self.running_tasks.clone();
        let exec_id_for_cleanup = execution_id.clone();

        let handle = tokio::spawn(async move {
            // Create a ScriptPlanRunner with the shared RunEngine
            let runner = ScriptPlanRunner::new(run_engine_clone);
            let result = runner.run(&script_clone).await;

            // Update execution state with result
            let mut executions = executions_clone.write().await;
            if let Some(exec) = executions.get_mut(&exec_id_clone) {
                match &result {
                    Ok(report) => {
                        exec.state = if report.success { "COMPLETED" } else { "ERROR" }.to_string();
                        if let Some(ref error_msg) = report.error {
                            exec.error = Some(error_msg.clone());
                        }
                    }
                    Err(e) => {
                        exec.state = "ERROR".to_string();
                        exec.error = Some(e.to_string());
                    }
                }
                exec.end_time = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64,
                );
                exec.progress_percent = 100;
            }

            // Remove from running_tasks now that we're done
            running_tasks_clone
                .write()
                .await
                .remove(&exec_id_for_cleanup);
        });

        // Store handle for potential cancellation
        self.running_tasks
            .write()
            .await
            .insert(execution_id.clone(), handle);

        Ok(Response::new(StartResponse {
            started: true,
            execution_id,
        }))
    }

    /// Stop a running script execution
    ///
    /// For force=true, the task is immediately aborted.
    /// For force=false (graceful), the task is also aborted since Rhai scripts
    /// run synchronously and cannot be interrupted mid-execution.
    async fn stop_script(
        &self,
        request: Request<StopRequest>,
    ) -> Result<Response<StopResponse>, Status> {
        let req = request.into_inner();

        // First check if execution exists and is running
        {
            let executions = self.executions.read().await;
            let exec = executions
                .get(&req.execution_id)
                .ok_or_else(|| Status::not_found("Execution not found"))?;

            if exec.state != "RUNNING" {
                return Ok(Response::new(StopResponse {
                    stopped: false,
                    message: format!("Cannot stop execution in state: {}", exec.state),
                }));
            }
        }

        // Abort the running task
        let handle = self.running_tasks.write().await.remove(&req.execution_id);

        let msg = if let Some(handle) = handle {
            handle.abort();
            if req.force {
                "Force stopped: task aborted"
            } else {
                "Gracefully stopped: task aborted (Rhai scripts cannot be interrupted mid-execution)"
            }
        } else {
            // Task completed between our check and removal - race condition
            "Task already completed"
        };

        // Update execution state
        let mut executions = self.executions.write().await;
        if let Some(exec) = executions.get_mut(&req.execution_id) {
            exec.state = "STOPPED".to_string();
            exec.end_time = Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
            );
        }

        Ok(Response::new(StopResponse {
            stopped: true,
            message: msg.to_string(),
        }))
    }

    /// Get current status of a script execution
    async fn get_script_status(
        &self,
        request: Request<StatusRequest>,
    ) -> Result<Response<ScriptStatus>, Status> {
        let req = request.into_inner();
        let executions = self.executions.read().await;

        let exec = executions
            .get(&req.execution_id)
            .ok_or_else(|| Status::not_found("Execution not found"))?;

        Ok(Response::new(ScriptStatus {
            execution_id: req.execution_id,
            state: exec.state.clone(),
            error_message: exec.error.clone().unwrap_or_default(),
            start_time_ns: exec.start_time,
            end_time_ns: exec.end_time.unwrap_or(0),
            script_id: exec.script_id.clone(),
            progress_percent: exec.progress_percent,
            current_line: exec.current_line.clone(),
        }))
    }

    type StreamStatusStream = tokio_stream::wrappers::ReceiverStream<Result<SystemStatus, Status>>;

    /// Stream system status updates at 10Hz (bd-obmt)
    ///
    /// Provides real system metrics:
    /// - CPU usage percentage (global across all cores)
    /// - Memory usage in MB (used_memory from sysinfo)
    /// - Current engine state (RUNNING, IDLE, ERROR)
    async fn stream_status(
        &self,
        _request: Request<StatusRequest>,
    ) -> Result<Response<Self::StreamStatusStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Spawn background task to send status updates at 10Hz
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
            let mut sys = System::new_all();

            loop {
                interval.tick().await;

                // Refresh all system metrics
                sys.refresh_all();

                // Get CPU usage as a percentage (0.0 to 100.0)
                let cpu_usage_percent = sys.global_cpu_usage();

                // Get memory usage in KB, convert to MB
                let used_memory_kb = sys.used_memory();
                let used_memory_mb = used_memory_kb as f64 / 1024.0;

                // Determine engine state based on CPU activity
                // This is a simple heuristic: if CPU > 1%, we consider it RUNNING
                let current_state = if cpu_usage_percent > 1.0 {
                    "RUNNING".to_string()
                } else {
                    "IDLE".to_string()
                };

                let status = SystemStatus {
                    current_state,
                    current_memory_usage_mb: used_memory_mb,
                    live_values: HashMap::new(),
                    timestamp_ns: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64,
                };

                if tx.send(Ok(status)).await.is_err() {
                    break; // Client disconnected
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    type StreamMeasurementsStream =
        tokio_stream::wrappers::ReceiverStream<Result<crate::grpc::proto::DataPoint, Status>>;

    /// Stream measurement data from specified channels
    async fn stream_measurements(
        &self,
        request: Request<crate::grpc::proto::MeasurementRequest>,
    ) -> Result<Response<Self::StreamMeasurementsStream>, Status> {
        let req = request.into_inner();
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Subscribe to hardware data broadcast
        let mut data_rx = self.data_tx.subscribe();
        let channels = req.channels;
        let max_rate_hz = req.max_rate_hz;

        // Spawn background task to forward hardware measurements to gRPC client
        tokio::spawn(async move {
            // Setup rate limiting if specified (applied to SEND side, not receive)
            let mut rate_limiter = if max_rate_hz > 0 {
                Some(tokio::time::interval(std::time::Duration::from_secs_f64(
                    1.0 / max_rate_hz as f64,
                )))
            } else {
                None
            };

            // Throttle lag warnings to prevent log spam (bd-jnfu.15)
            let mut last_lag_warning = std::time::Instant::now();
            let mut total_skipped: u64 = 0;

            loop {
                // Receive data from hardware broadcast FIRST (drain to get latest)
                // This fixes bd-jnfu.15: rate limiting was causing broadcast overflow
                let data_point = match data_rx.recv().await {
                    Ok(dp) => dp,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // Throttle lag warnings to once per second max (bd-jnfu.15)
                        total_skipped += skipped;
                        if last_lag_warning.elapsed() > std::time::Duration::from_secs(1) {
                            tracing::debug!(
                                skipped = total_skipped,
                                "gRPC client lagged behind hardware stream, skipped measurements"
                            );
                            total_skipped = 0;
                            last_lag_warning = std::time::Instant::now();
                        }
                        continue; // Skip to next measurement
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break; // Broadcast channel closed, exit task
                    }
                };

                // Apply rate limiting to SEND side (after receiving latest data)
                if let Some(ref mut limiter) = rate_limiter {
                    limiter.tick().await;
                }

                // Extract channel and value from Measurement for filtering and conversion
                let (name, value, timestamp_ns) = match &data_point {
                    Measurement::Scalar {
                        name,
                        value,
                        timestamp,
                        ..
                    } => {
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        (name.clone(), *value, ts_ns)
                    }
                    Measurement::Vector {
                        name,
                        values,
                        timestamp,
                        ..
                    } => {
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        // For vectors, we can emit the length or first value
                        (format!("{}_len", name), values.len() as f64, ts_ns)
                    }
                    Measurement::Image {
                        name,
                        width,
                        height,
                        timestamp,
                        ..
                    } => {
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        (name.clone(), (width * height) as f64, ts_ns)
                    }
                    Measurement::Spectrum {
                        name,
                        amplitudes,
                        timestamp,
                        ..
                    } => {
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        (format!("{}_spectrum", name), amplitudes.len() as f64, ts_ns)
                    }
                };

                // Filter by channel if specified
                if !channels.is_empty() && !channels.contains(&name) {
                    continue;
                }

                // Convert to proto DataPoint
                let proto_data_point = crate::grpc::proto::DataPoint {
                    channel: name,
                    value,
                    timestamp_ns,
                };

                // Forward to gRPC client
                if tx.send(Ok(proto_data_point)).await.is_err() {
                    break; // Client disconnected
                }

                // Yield to allow other tasks to run
                tokio::task::yield_now().await;
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    /// List all uploaded scripts
    async fn list_scripts(
        &self,
        _request: Request<ListScriptsRequest>,
    ) -> Result<Response<ListScriptsResponse>, Status> {
        let metadata = self.script_metadata.read().await;

        let script_infos: Vec<ScriptInfo> = metadata
            .iter()
            .map(|(id, meta)| ScriptInfo {
                script_id: id.clone(),
                name: meta.name.clone(),
                upload_time_ns: meta.upload_time,
                metadata: meta.metadata.clone(),
            })
            .collect();

        Ok(Response::new(ListScriptsResponse {
            scripts: script_infos,
        }))
    }

    /// List all script executions (optionally filtered)
    async fn list_executions(
        &self,
        request: Request<ListExecutionsRequest>,
    ) -> Result<Response<ListExecutionsResponse>, Status> {
        let req = request.into_inner();
        let executions = self.executions.read().await;

        let mut execution_list: Vec<ScriptStatus> = executions
            .iter()
            .filter(|(_, exec)| {
                // Filter by script_id if provided
                if let Some(ref script_id) = req.script_id
                    && &exec.script_id != script_id
                {
                    return false;
                }
                // Filter by state if provided
                if let Some(ref state) = req.state
                    && &exec.state != state
                {
                    return false;
                }
                true
            })
            .map(|(exec_id, exec)| ScriptStatus {
                execution_id: exec_id.clone(),
                state: exec.state.clone(),
                error_message: exec.error.clone().unwrap_or_default(),
                start_time_ns: exec.start_time,
                end_time_ns: exec.end_time.unwrap_or(0),
                script_id: exec.script_id.clone(),
                progress_percent: exec.progress_percent,
                current_line: exec.current_line.clone(),
            })
            .collect();

        // Sort by start time, newest first
        execution_list.sort_by(|a, b| b.start_time_ns.cmp(&a.start_time_ns));

        Ok(Response::new(ListExecutionsResponse {
            executions: execution_list,
        }))
    }

    /// Get daemon version and capabilities
    async fn get_daemon_info(
        &self,
        _request: Request<DaemonInfoRequest>,
    ) -> Result<Response<DaemonInfoResponse>, Status> {
        let mut features = Vec::new();

        #[cfg(feature = "networking")]
        features.push("networking".to_string());

        #[cfg(feature = "storage_hdf5")]
        features.push("storage_hdf5".to_string());

        let uptime = self.start_time.elapsed().unwrap_or_default().as_secs();

        Ok(Response::new(DaemonInfoResponse {
            version: env!("CARGO_PKG_VERSION").to_string(),
            features,
            available_hardware: vec!["MockStage".to_string(), "MockCamera".to_string()],
            uptime_seconds: uptime,
        }))
    }
}

/// Start the DAQ gRPC server
///
/// Provides RunEngineService and optionally ControlService (when `scripting` feature is enabled).
/// ControlService includes script execution, stream_measurements, and stream_status methods.
pub async fn start_server(addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    use crate::grpc::health_service::HealthServiceImpl;
    use crate::grpc::proto::health::health_check_response::ServingStatus;
    use crate::grpc::proto::health::health_server::HealthServer;

    let grpc_settings = GrpcSettings::load()?;
    if grpc_settings.auth_enabled && grpc_settings.auth_token().is_none() {
        return Err("grpc.auth_enabled is true but grpc.auth_token is not configured".into());
    }

    let bind_addr = grpc_settings.bind_socket(addr.port());
    if bind_addr.ip() != addr.ip() {
        eprintln!(
            "⚠️  Overriding gRPC bind address {} -> {} (set grpc.bind_address to change)",
            addr.ip(),
            bind_addr.ip()
        );
    }

    // Create shared RunEngine FIRST (bd-si2c)
    let registry = std::sync::Arc::new(daq_hardware::registry::DeviceRegistry::new());
    let run_engine_instance = std::sync::Arc::new(daq_experiment::RunEngine::new(registry));

    // Create DaqServer with shared RunEngine when scripting enabled (bd-si2c)
    #[cfg(feature = "scripting")]
    let server = DaqServer::new(run_engine_instance.clone())?;
    #[cfg(not(feature = "scripting"))]
    let server = DaqServer::new()?;

    let run_engine = RunEngineServiceImpl::new(run_engine_instance);

    let health_service = HealthServiceImpl::new();

    health_service.set_serving_status("", ServingStatus::Serving);
    health_service.set_serving_status("daq.ControlService", ServingStatus::Serving);
    health_service.set_serving_status("daq.RunEngineService", ServingStatus::Serving);

    println!("DAQ gRPC server listening on {}", bind_addr);

    if !grpc_settings.auth_enabled {
        eprintln!("⚠️  gRPC auth is disabled (set grpc.auth_enabled=true to require auth)");
    }

    let cors = build_cors_layer(&grpc_settings)?;
    let tls_config = build_tls_config(&grpc_settings)?;
    if tls_config.is_none() {
        eprintln!(
            "⚠️  gRPC TLS is disabled (set grpc.tls_cert_path + grpc.tls_key_path to enable)"
        );
    }

    let auth_settings = grpc_settings.clone();
    let mut builder = Server::builder()
        .accept_http1(true)
        .layer(cors)
        .layer(interceptor(move |request: Request<()>| {
            validate_auth(&auth_settings, &request)?;
            Ok(request)
        }));

    if let Some(tls_config) = tls_config {
        builder = builder.tls_config(tls_config)?;
    }

    #[cfg(feature = "scripting")]
    let builder = builder.add_service(tonic_web::enable(ControlServiceServer::new(server)));

    builder
        .add_service(tonic_web::enable(HealthServer::new(health_service)))
        .add_service(tonic_web::enable(RunEngineServiceServer::new(run_engine)))
        .serve(bind_addr)
        .await?;

    Ok(())
}

use common::pipeline::{MeasurementSink, Tee};

// ... (existing imports)

/// Start the DAQ gRPC server with hardware control (bd-4x6q)
///
/// Provides HardwareService for direct device control and optionally ControlService
/// (when `scripting` feature is enabled) for script management and data streaming.
///
/// # Arguments
/// * `addr` - Socket address to listen on
/// * `registry` - Device registry for hardware access
///
/// # Example
/// ```ignore
/// use rust_daq::grpc::start_server_with_hardware;
/// use rust_daq::hardware::registry::create_mock_registry;
/// use std::sync::Arc;
/// use tokio::sync::RwLock;
///
/// let registry = create_mock_registry().await?;
/// let addr = "127.0.0.1:50051".parse()?;
/// start_server_with_hardware(addr, Arc::new(RwLock::new(registry))).await?;
/// ```
pub async fn start_server_with_hardware(
    addr: std::net::SocketAddr,
    registry: std::sync::Arc<daq_hardware::registry::DeviceRegistry>,
    health_monitor: std::sync::Arc<common::health::SystemHealthMonitor>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::grpc::hardware_service::HardwareServiceImpl;
    use crate::grpc::module_service::ModuleServiceImpl;
    use crate::grpc::ni_daq_service::NiDaqServiceImpl;
    use daq_storage::hdf5_writer::HDF5Writer;
    use daq_storage::ring_buffer::RingBuffer;
    // use crate::grpc::plugin_service::PluginServiceImpl; // Unused
    use crate::grpc::preset_service::{PresetServiceImpl, default_preset_storage_path};
    use crate::grpc::proto::hardware_service_server::HardwareServiceServer;
    use crate::grpc::proto::health::health_check_response::ServingStatus;
    use crate::grpc::proto::health::health_server::HealthServer;
    use crate::grpc::proto::health_service_server::HealthServiceServer; // Custom HealthService
    use crate::grpc::proto::module_service_server::ModuleServiceServer;
    use protocol::ni_daq::ni_daq_service_server::NiDaqServiceServer;
    // use crate::grpc::proto::plugin_service_server::PluginServiceServer; // Unused
    use crate::grpc::proto::preset_service_server::PresetServiceServer;
    use crate::grpc::proto::scan_service_server::ScanServiceServer;
    use crate::grpc::proto::storage_service_server::StorageServiceServer;
    #[allow(deprecated)] // ScanService kept for backwards compatibility until v0.8.0
    use crate::grpc::scan_service::ScanServiceImpl;
    use crate::grpc::storage_service::StorageServiceImpl;

    let grpc_settings = GrpcSettings::load()?;
    if grpc_settings.auth_enabled && grpc_settings.auth_token().is_none() {
        return Err("grpc.auth_enabled is true but grpc.auth_token is not configured".into());
    }

    let bind_addr = grpc_settings.bind_socket(addr.port());
    if bind_addr.ip() != addr.ip() {
        eprintln!(
            "⚠️  Overriding gRPC bind address {} -> {} (set grpc.bind_address to change)",
            addr.ip(),
            bind_addr.ip()
        );
    }
    use std::path::Path;

    // Create ring buffer for scan data persistence (The Mullet Strategy)
    // Use /dev/shm on Linux, /tmp on macOS for memory-mapped storage
    let ring_buffer_path = if cfg!(target_os = "linux") {
        Path::new("/dev/shm/rust_daq_scan_data.buf")
    } else {
        Path::new("/tmp/rust_daq_scan_data.buf")
    };

    let ring_buffer = match RingBuffer::create(ring_buffer_path, 100) {
        Ok(rb) => {
            println!("  - RingBuffer: {} (100 MB)", ring_buffer_path.display());
            Some(std::sync::Arc::<daq_storage::ring_buffer::RingBuffer>::new(
                rb,
            ))
        }
        Err(e) => {
            eprintln!(
                "Warning: Failed to create ring buffer: {}. Scan data will not be persisted.",
                e
            );
            None
        }
    };

    // Spawn HDF5Writer background task if ring buffer is available
    // This is the "Business in the Back" of The Mullet Strategy
    if let Some(ref rb) = ring_buffer {
        let hdf5_output_path = if cfg!(target_os = "linux") {
            Path::new("/tmp/rust_daq_scan_data.h5")
        } else {
            Path::new("/tmp/rust_daq_scan_data.h5")
        };

        match HDF5Writer::new(hdf5_output_path, rb.clone()) {
            Ok(writer) => {
                println!(
                    "  - HDF5Writer: {} (1 Hz flush)",
                    hdf5_output_path.display()
                );
                tokio::spawn(async move {
                    writer.run().await;
                });
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to create HDF5 writer: {}. Data will not be persisted to disk.",
                    e
                );
            }
        }
    }

    // Create shared RunEngine FIRST - used by both RunEngineService and ControlService/scripts (bd-si2c)
    let run_engine = std::sync::Arc::new(daq_experiment::RunEngine::new(registry.clone()));

    // Initialize control server WITHOUT internal RingBuffer logic (we wire it manually)
    // Pass shared run_engine for script execution (bd-si2c)
    #[cfg(all(feature = "storage_hdf5", feature = "scripting"))]
    let control_server = DaqServer::new(None, run_engine.clone())?;
    #[cfg(all(feature = "storage_hdf5", not(feature = "scripting")))]
    let control_server = DaqServer::new(None)?;
    #[cfg(all(not(feature = "storage_hdf5"), feature = "scripting"))]
    let control_server = DaqServer::new(run_engine.clone())?;
    #[cfg(all(not(feature = "storage_hdf5"), not(feature = "scripting")))]
    let control_server = DaqServer::new()?;

    // Setup Reliable Sink (RingBuffer Writer)
    let reliable_sink_tx = if let Some(ref rb) = ring_buffer {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Measurement>(512);
        let rb_clone = rb.clone();

        // Spawn writer task
        tokio::spawn(async move {
            while let Some(measurement) = rx.recv().await {
                if let Ok(frame) = encode_measurement_frame(&measurement)
                    && let Err(e) = rb_clone.write(&frame)
                {
                    tracing::error!(error = %e, "Failed to write measurement to ring buffer");
                }
                // Yield to allow other tasks to run
                tokio::task::yield_now().await;
            }
        });
        Some(tx)
    } else {
        None
    };

    // Wire Pipelines (bd-37tw.7 - Tee-based)
    // Connect measurement sources to Tee -> (RingBuffer, Server Broadcast)
    //
    // SAFETY (bd-jnfu.6): Collect device info and sources while holding lock,
    // then DROP lock before performing async operations to prevent deadlock/contention.
    {
        // Phase 1: Collect devices and sources (lock-free with DashMap)
        let devices_to_wire: Vec<_> = registry
            .list_devices()
            .into_iter()
            .filter_map(|info| {
                registry
                    .get_measurement_source_frame(&info.id)
                    .map(|source| (info.id.clone(), source))
            })
            .collect();

        // Phase 2: Perform async registration (no lock held)
        for (device_id, source) in devices_to_wire {
            println!("  - Wiring pipeline for device: {}", device_id);

            // 1. Create channel for Source output (Arc<Frame>)
            let frame_chan = std::env::var("DAQ_PIPELINE_FRAME_CH")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(16);
            let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(frame_chan);

            // 2. Register source output (ASYNC - safe now, no lock held)
            if let Err(e) = source.register_output(frame_tx).await {
                eprintln!("Failed to register output for {}: {}", device_id, e);
                continue;
            }

            // 3. Create channel for Measurement (Tee Input)
            let meas_chan = std::env::var("DAQ_PIPELINE_MEAS_CH")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(16);
            let (meas_tx, meas_rx) = tokio::sync::mpsc::channel(meas_chan);
            let device_id_clone = device_id.clone();

            // 4. Spawn Converter Task (Frame -> Measurement)
            tokio::spawn(async move {
                while let Some(frame) = frame_rx.recv().await {
                    let buffer = match frame.bit_depth {
                        16 => {
                            if let Some(slice) = frame.as_u16_slice() {
                                common::core::PixelBuffer::U16(slice.to_vec())
                            } else {
                                // Convert Bytes to Vec<u8> for PixelBuffer
                                common::core::PixelBuffer::U8(frame.data.to_vec())
                            }
                        }
                        // Convert Bytes to Vec<u8> for PixelBuffer
                        _ => common::core::PixelBuffer::U8(frame.data.to_vec()),
                    };

                    let measurement = common::core::Measurement::Image {
                        name: device_id_clone.clone(),
                        width: frame.width,
                        height: frame.height,
                        buffer,
                        unit: "counts".to_string(),
                        metadata: common::core::ImageMetadata::default(),
                        timestamp: chrono::Utc::now(),
                    };

                    if meas_tx.send(measurement).await.is_err() {
                        break; // Downstream closed
                    }
                }
            });

            // 5. Create Tee
            let mut tee = Tee::new((*control_server.data_sender()).clone()); // Lossy output (Server Bus)

            // 6. Connect Reliable Output (if RingBuffer is present)
            if let Some(ref rb_tx) = reliable_sink_tx {
                tee.connect_reliable(rb_tx.clone());
            }

            // 7. Start Tee (Consume Measurement Stream)
            if let Err(e) = tee.register_input(meas_rx) {
                eprintln!("Failed to register Tee input for {}: {}", device_id, e);
            }
        }
    }

    // Setup Rerun Visualization (gRPC server mode for remote GUI clients)
    #[cfg(feature = "rerun_sink")]
    {
        // Port 9876 is the default Rerun gRPC port
        // same_machine=false enables higher memory limits for remote clients
        let rerun_bind = std::env::var("RERUN_BIND").unwrap_or_else(|_| "0.0.0.0".to_string());
        let rerun_port: u16 = std::env::var("RERUN_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9876);
        match crate::rerun_sink::RerunSink::new_server(&rerun_bind, rerun_port, false) {
            Ok(rerun) => {
                println!(
                    "  - Rerun Visualization: Active (gRPC server on {}:{})",
                    rerun_bind, rerun_port
                );

                // Optional blueprint: default path or override via RERUN_BLUEPRINT
                let blueprint_choice = std::env::var("RERUN_BLUEPRINT")
                    .unwrap_or_else(|_| "crates/daq-server/blueprints/daq_default.rbl".to_string());
                let skip_blueprint = matches!(
                    blueprint_choice.to_ascii_lowercase().as_str(),
                    "none" | "off" | "skip"
                );

                if skip_blueprint {
                    println!(
                        "    - Blueprint: skipped (RERUN_BLUEPRINT={})",
                        blueprint_choice
                    );
                } else {
                    match rerun.load_blueprint_if_exists(&blueprint_choice) {
                        Ok(true) => println!("    - Blueprint: {}", blueprint_choice),
                        Ok(false) => println!(
                            "    - Blueprint: not found at {} (generate with `python crates/daq-server/blueprints/generate_blueprints.py`)",
                            blueprint_choice
                        ),
                        Err(e) => eprintln!(
                            "Warning: Failed to load blueprint {}: {}",
                            blueprint_choice, e
                        ),
                    }
                }

                rerun.monitor_broadcast(control_server.data_sender().subscribe());
            }
            Err(e) => {
                eprintln!("Warning: Failed to start Rerun visualization: {}", e);
            }
        }
    }

    // RunEngine was already created above (bd-si2c) - shared between RunEngineService and scripts
    let run_engine_server = RunEngineServiceImpl::new(run_engine.clone());

    let hardware_server = HardwareServiceImpl::new(registry.clone());
    let module_server = ModuleServiceImpl::new(registry.clone());
    let ni_daq_server = NiDaqServiceImpl::new(registry.clone());

    // Create PluginService with shared factory and registry (bd-0451)
    #[cfg(feature = "serial")]
    let plugin_server = {
        // No lock needed for registry anymore
        let factory = registry.plugin_factory();
        PluginServiceImpl::new(factory, registry.clone())
    };

    // Wire ScanService with optional data persistence
    // Note: ScanService is deprecated in favor of RunEngineService but kept for backwards compatibility
    #[allow(deprecated)]
    let scan_server = if let Some(rb) = ring_buffer.clone() {
        ScanServiceImpl::new(registry.clone()).with_ring_buffer(rb)
    } else {
        ScanServiceImpl::new(registry.clone())
    };

    let preset_server = PresetServiceImpl::new(registry, default_preset_storage_path());
    let storage_server = StorageServiceImpl::new(ring_buffer.clone());

    // Standard gRPC Health Check (grpc.health.v1)
    let standard_health_service = crate::grpc::health_service::HealthServiceImpl::new();

    // Custom System Health Monitoring    // Custom health service with monitoring
    let custom_health_service =
        crate::grpc::custom_health_service::HealthServiceImpl::new(health_monitor);

    // Register serving status for all services
    standard_health_service.set_serving_status("", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.ControlService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.HardwareService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.ModuleService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.ScanService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.PresetService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.StorageService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.RunEngineService", ServingStatus::Serving);
    standard_health_service.set_serving_status("daq.HealthService", ServingStatus::Serving); // Register custom service too
    #[cfg(feature = "serial")]
    standard_health_service.set_serving_status("daq.PluginService", ServingStatus::Serving);

    println!("DAQ gRPC server (with hardware) listening on {}", bind_addr);
    println!("  - ControlService: script management");
    println!("  - HardwareService: direct device control");
    println!("  - HealthService: system health monitoring (bd-ergo)");
    println!("  - ModuleService: experiment modules (bd-c0ai)");
    #[cfg(feature = "serial")]
    println!("  - PluginService: YAML-defined instrument plugins (bd-0451)");
    println!("  - ScanService: coordinated multi-axis scans");
    println!("  - PresetService: configuration save/load (bd-akcm)");
    println!("  - StorageService: HDF5 data storage (bd-p6im)");

    if !grpc_settings.auth_enabled {
        eprintln!("⚠️  gRPC auth is disabled (set grpc.auth_enabled=true to require auth)");
    }

    let cors = build_cors_layer(&grpc_settings)?;
    let tls_config = build_tls_config(&grpc_settings)?;
    if tls_config.is_none() {
        eprintln!(
            "⚠️  gRPC TLS is disabled (set grpc.tls_cert_path + grpc.tls_key_path to enable)"
        );
    }

    #[cfg(feature = "serial")]
    let mut server_builder = {
        let auth_settings = grpc_settings.clone();
        Server::builder()
            .accept_http1(true)
            .layer(cors.clone())
            .layer(interceptor(move |request: Request<()>| {
                validate_auth(&auth_settings, &request)?;
                Ok(request)
            }))
    };

    #[cfg(feature = "serial")]
    if let Some(tls_config) = tls_config {
        server_builder = server_builder.tls_config(tls_config)?;
    }

    #[cfg(all(feature = "serial", feature = "scripting"))]
    let server_builder =
        server_builder.add_service(tonic_web::enable(ControlServiceServer::new(control_server)));

    #[cfg(feature = "serial")]
    let server_builder = server_builder
        .add_service(tonic_web::enable(HealthServer::new(
            standard_health_service,
        )))
        .add_service(tonic_web::enable(HealthServiceServer::new(
            custom_health_service,
        )))
        .add_service(tonic_web::enable(RunEngineServiceServer::new(
            run_engine_server.clone(),
        )))
        // HardwareService needs larger message size for camera frame streaming (16 MB)
        .add_service(tonic_web::enable(
            HardwareServiceServer::new(hardware_server).max_encoding_message_size(64 * 1024 * 1024),
        ))
        .add_service(tonic_web::enable(NiDaqServiceServer::new(
            ni_daq_server.clone(),
        )))
        .add_service(tonic_web::enable(ModuleServiceServer::new(module_server)))
        .add_service(tonic_web::enable(PluginServiceServer::new(plugin_server)))
        .add_service(tonic_web::enable(ScanServiceServer::new(scan_server)))
        .add_service(tonic_web::enable(PresetServiceServer::new(preset_server)))
        .add_service(tonic_web::enable(StorageServiceServer::new(storage_server)));

    #[cfg(not(feature = "serial"))]
    let mut server_builder = {
        let auth_settings = grpc_settings.clone();
        Server::builder()
            .accept_http1(true)
            .layer(cors.clone())
            .layer(interceptor(move |request: Request<()>| {
                validate_auth(&auth_settings, &request)?;
                Ok(request)
            }))
    };

    #[cfg(not(feature = "serial"))]
    if let Some(tls_config) = tls_config {
        server_builder = server_builder.tls_config(tls_config)?;
    }

    #[cfg(all(not(feature = "serial"), feature = "scripting"))]
    let server_builder =
        server_builder.add_service(tonic_web::enable(ControlServiceServer::new(control_server)));

    #[cfg(not(feature = "serial"))]
    let server_builder = server_builder
        .add_service(tonic_web::enable(HealthServer::new(
            standard_health_service,
        )))
        .add_service(tonic_web::enable(HealthServiceServer::new(
            custom_health_service,
        )))
        .add_service(tonic_web::enable(RunEngineServiceServer::new(
            run_engine_server,
        )))
        // HardwareService needs larger message size for camera frame streaming (16 MB)
        .add_service(tonic_web::enable(
            HardwareServiceServer::new(hardware_server).max_encoding_message_size(64 * 1024 * 1024),
        ))
        .add_service(tonic_web::enable(NiDaqServiceServer::new(ni_daq_server)))
        .add_service(tonic_web::enable(ModuleServiceServer::new(module_server)))
        .add_service(tonic_web::enable(ScanServiceServer::new(scan_server)))
        .add_service(tonic_web::enable(PresetServiceServer::new(preset_server)))
        .add_service(tonic_web::enable(StorageServiceServer::new(storage_server)));

    // Start Prometheus metrics server if enabled (bd-v299)
    #[cfg(feature = "metrics")]
    let _metrics_handle = {
        let metrics_port: u16 = std::env::var("METRICS_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9091);
        match crate::grpc::metrics_service::start_metrics_server(metrics_port).await {
            Ok(handle) => {
                println!(
                    "  - Prometheus Metrics: http://0.0.0.0:{}/metrics (bd-v299)",
                    metrics_port
                );
                Some(handle)
            }
            Err(e) => {
                eprintln!("⚠️  Failed to start metrics server: {}", e);
                None
            }
        }
    };

    server_builder.serve(bind_addr).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Create a test DaqServer with a mock RunEngine (bd-si2c)
    #[cfg(feature = "scripting")]
    fn create_test_server() -> DaqServer {
        let registry = std::sync::Arc::new(daq_hardware::registry::DeviceRegistry::new());
        let run_engine = std::sync::Arc::new(daq_experiment::RunEngine::new(registry));
        DaqServer::new(run_engine).expect("failed to create test DaqServer")
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_upload_valid_script() {
        let server = create_test_server();
        let request = Request::new(UploadRequest {
            script_content: "let x = 42;".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });

        let response = server.upload_script(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!(!resp.script_id.is_empty());
        assert_eq!(resp.error_message, "");
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_upload_invalid_script() {
        let server = create_test_server();
        let request = Request::new(UploadRequest {
            script_content: "this is not valid rhai syntax {{{".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });

        let response = server.upload_script(request).await.unwrap();
        let resp = response.into_inner();

        assert!(!resp.success);
        assert!(resp.script_id.is_empty());
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_start_nonexistent_script() {
        let server = create_test_server();
        let request = Request::new(StartRequest {
            script_id: "nonexistent-id".to_string(),
            parameters: HashMap::new(),
        });

        let result = server.start_script(request).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_script_execution_lifecycle() {
        let server = create_test_server();

        // Upload script
        let upload_req = Request::new(UploadRequest {
            script_content: "let x = 1 + 1;".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });
        let upload_resp = server.upload_script(upload_req).await.unwrap().into_inner();
        assert!(upload_resp.success);

        // Start execution
        let start_req = Request::new(StartRequest {
            script_id: upload_resp.script_id,
            parameters: HashMap::new(),
        });
        let start_resp = server.start_script(start_req).await.unwrap().into_inner();
        assert!(start_resp.started);

        // Wait for completion
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Check status
        let status_req = Request::new(StatusRequest {
            execution_id: start_resp.execution_id,
        });
        let status_resp = server
            .get_script_status(status_req)
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status_resp.state, "COMPLETED");
        assert_eq!(status_resp.error_message, "");
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_measurements_basic() {
        use tokio_stream::StreamExt;

        let server = create_test_server();

        // Get sender to simulate hardware
        let data_sender = server.data_sender();

        // Start streaming with no filters
        let request = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec![],
            max_rate_hz: 0,
        });

        let response = server.stream_measurements(request).await.unwrap();
        let mut stream = response.into_inner();

        // Spawn task to send mock data
        tokio::spawn(async move {
            for i in 0..5 {
                let _ = data_sender.send(Measurement::Scalar {
                    name: "test_channel".to_string(),
                    value: i as f64,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        // Collect measurements
        let mut received = Vec::new();
        while let Some(result) = stream.next().await {
            let data_point = result.unwrap();
            received.push(data_point);
            if received.len() >= 5 {
                break;
            }
        }

        // Verify we got all 5 measurements
        assert_eq!(received.len(), 5);
        assert_eq!(received[0].channel, "test_channel");
        assert_eq!(received[0].value, 0.0);
        assert_eq!(received[4].value, 4.0);
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_measurements_channel_filter() {
        use tokio_stream::StreamExt;

        let server = create_test_server();
        let data_sender = server.data_sender();

        // Request only "channel_a" measurements
        let request = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec!["channel_a".to_string()],
            max_rate_hz: 0,
        });

        let response = server.stream_measurements(request).await.unwrap();
        let mut stream = response.into_inner();

        // Send mixed data
        tokio::spawn(async move {
            for i in 0..10 {
                let channel = if i % 2 == 0 { "channel_a" } else { "channel_b" };
                let _ = data_sender.send(Measurement::Scalar {
                    name: channel.to_string(),
                    value: i as f64,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        });

        // Collect filtered measurements
        let mut received = Vec::new();
        while let Some(result) = stream.next().await {
            let data_point = result.unwrap();
            received.push(data_point);
            if received.len() >= 5 {
                break;
            }
        }

        // Verify only channel_a was received
        assert_eq!(received.len(), 5);
        for data_point in &received {
            assert_eq!(data_point.channel, "channel_a");
        }

        // Verify values are even (0, 2, 4, 6, 8)
        assert_eq!(received[0].value, 0.0);
        assert_eq!(received[1].value, 2.0);
        assert_eq!(received[4].value, 8.0);
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_measurements_rate_limiting() {
        use std::time::Instant;
        use tokio_stream::StreamExt;

        let server = create_test_server();
        let data_sender = server.data_sender();

        // Request max 10 Hz rate
        let request = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec![],
            max_rate_hz: 10,
        });

        let response = server.stream_measurements(request).await.unwrap();
        let mut stream = response.into_inner();

        // Send data faster than rate limit
        tokio::spawn(async move {
            for i in 0..20 {
                let _ = data_sender.send(Measurement::Scalar {
                    name: "test".to_string(),
                    value: i as f64,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        });

        // Measure time to receive 5 measurements
        let start = Instant::now();
        let mut count = 0;
        while let Some(result) = stream.next().await {
            result.unwrap();
            count += 1;
            if count >= 5 {
                break;
            }
        }
        let elapsed = start.elapsed();

        // At 10 Hz, 5 measurements should take ~400-500ms
        // (first is immediate, then 4 x 100ms intervals)
        assert!(
            elapsed.as_millis() >= 400,
            "Rate limiting not working: took {:?}",
            elapsed
        );
        assert!(
            elapsed.as_millis() < 700,
            "Rate limiting too slow: took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_measurements_multiple_clients() {
        use tokio_stream::StreamExt;

        let server = create_test_server();
        let data_sender = server.data_sender();

        // Start two concurrent streams
        let request1 = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec![],
            max_rate_hz: 0,
        });
        let request2 = Request::new(crate::grpc::proto::MeasurementRequest {
            channels: vec![],
            max_rate_hz: 0,
        });

        let response1 = server.stream_measurements(request1).await.unwrap();
        let response2 = server.stream_measurements(request2).await.unwrap();

        let mut stream1 = response1.into_inner();
        let mut stream2 = response2.into_inner();

        // Send test data
        tokio::spawn(async move {
            for i in 0..3 {
                let _ = data_sender.send(Measurement::Scalar {
                    name: "shared".to_string(),
                    value: i as f64,
                    unit: "V".to_string(),
                    timestamp: Utc::now(),
                });
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        // Both clients should receive the same data
        let mut client1_data = Vec::new();
        let mut client2_data = Vec::new();

        for _ in 0..3 {
            if let Some(result) = stream1.next().await {
                client1_data.push(result.unwrap().value);
            }
            if let Some(result) = stream2.next().await {
                client2_data.push(result.unwrap().value);
            }
        }

        assert_eq!(client1_data.len(), 3);
        assert_eq!(client2_data.len(), 3);
        assert_eq!(client1_data, client2_data);
    }

    #[test]
    fn test_auth_rejects_missing_token() {
        let settings = GrpcSettings {
            auth_enabled: true,
            auth_token: Some("secret".to_string()),
            ..Default::default()
        };

        let request = Request::new(());
        let result = validate_auth(&settings, &request);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn test_auth_rejects_invalid_token() {
        let settings = GrpcSettings {
            auth_enabled: true,
            auth_token: Some("secret".to_string()),
            ..Default::default()
        };

        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer wrong".parse().unwrap());

        let result = validate_auth(&settings, &request);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn test_auth_allows_matching_token() {
        let settings = GrpcSettings {
            auth_enabled: true,
            auth_token: Some("secret".to_string()),
            ..Default::default()
        };

        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("x-api-key", "secret".parse().unwrap());

        let result = validate_auth(&settings, &request);

        assert!(result.is_ok());
    }
}
