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
use scripting::ScriptEngine; // Trait import
// use common::error::DaqError; // Unused
#[cfg(feature = "scripting")]
use experiment::RunEngine;
use protocol::daq::{UploadRequest, UploadResponse};
#[cfg(feature = "scripting")]
use scripting::RhaiEngine;
#[cfg(feature = "scripting")]
use scripting::ScriptPlanRunner;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "storage_hdf5")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::System;
#[cfg(feature = "storage_hdf5")]
use tokio::sync::mpsc;
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::body::BoxBody;
use tonic::service::interceptor::interceptor;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use uuid::Uuid;

use http::{HeaderValue, header::HeaderName};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};

use crate::config::{GrpcSettings, ServerConfig};

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

#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    exp: Option<usize>,
    iss: Option<String>,
    aud: Option<String>,
    sub: Option<String>,
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
    let mut cors = CorsLayer::new()
        .allow_headers(Any)
        .allow_methods(Any)
        .expose_headers([
            HeaderName::from_static("grpc-status"),
            HeaderName::from_static("grpc-message"),
            HeaderName::from_static("grpc-status-details-bin"),
            HeaderName::from_static("grpc-encoding"),
            HeaderName::from_static("grpc-accept-encoding"),
            HeaderName::from_static("x-grpc-web"),
            HeaderName::from_static("content-type"),
        ]);

    if settings.allowed_origins.iter().any(|o| o == "*") {
        cors = cors.allow_origin(AllowOrigin::any());
    } else if settings.allowed_origins.is_empty() {
        eprintln!("⚠️  grpc.allowed_origins is empty; gRPC-web requests will be blocked by CORS");
        cors = cors.allow_origin(AllowOrigin::list(Vec::<HeaderValue>::new()));
    } else {
        let origins = settings
            .allowed_origins
            .iter()
            .map(|origin| HeaderValue::from_str(origin))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Invalid origin in grpc.allowed_origins: {e}"))?;
        cors = cors.allow_origin(AllowOrigin::list(origins));
    }

    Ok(cors)
}

#[expect(
    clippy::result_large_err,
    reason = "tonic::Status (176 bytes) is the standard gRPC error type"
)]
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
    let Some(token) = candidate else {
        return Err(Status::unauthenticated("missing authorization token"));
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
    #[cfg(feature = "scripting")]
    /// Persistent journal for script crash recovery (bd-izdj.7)
    script_journal: Arc<crate::script_journal::ScriptJournal>,
    #[cfg(feature = "scripting")]
    /// Token used to reject new script starts during shutdown (bd-1afe.12 race fix).
    shutdown_token: CancellationToken,
    start_time: SystemTime,

    /// Broadcast channel for distributing hardware measurements to multiple consumers.
    /// Receivers can be cloned for gRPC clients, storage writers, etc.
    data_tx: Arc<broadcast::Sender<Measurement>>,

    /// Optional ring buffer for persistent storage (only when storage features enabled)
    #[cfg(feature = "storage_hdf5")]
    _ring_buffer: Option<Arc<storage::ring_buffer::RingBuffer>>,
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
        ring_buffer: Option<Arc<storage::ring_buffer::RingBuffer>>,
        run_engine: Arc<RunEngine>,
        shutdown_token: CancellationToken,
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
                                if let Err(err) = rb_tx.try_send(data_point)
                                    && matches!(err, mpsc::error::TrySendError::Full(_))
                                {
                                    let dropped = drop_counter.fetch_add(1, Ordering::Relaxed) + 1;
                                    if dropped.is_multiple_of(100) {
                                        tracing::warn!(
                                            dropped = dropped,
                                            "Dropped {} measurements while ring buffer writer was saturated",
                                            dropped
                                        );
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                // Throttle lag warnings to every 100 events (bd-jnfu.15)
                                total_lagged += skipped;
                                if total_lagged.is_multiple_of(100) || skipped > 50 {
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
                let mut last_frame_numbers = HashMap::<String, u64>::new();
                while let Some(mut measurement) = rb_rx.recv().await {
                    if let Some((source, fault)) = annotate_measurement_data_integrity(
                        &mut measurement,
                        &mut last_frame_numbers,
                    ) {
                        log_data_integrity_fault(&source, fault);
                    }

                    match encode_measurement_frame(&measurement) {
                        Ok(frame) => {
                            let rb = rb.clone();
                            // Offload blocking mmap write to avoid stalling Tokio
                            // workers under contention (bd-tvp6).
                            if let Err(e) = tokio::task::spawn_blocking(move || rb.write(&frame))
                                .await
                                .expect("ring buffer write task panicked")
                            {
                                tracing::error!(error = %e, "Failed to write measurement to ring buffer");
                            }
                        }
                        Err(e) => tracing::error!(error = %e, "Failed to encode measurement frame"),
                    }
                }
            });
        }

        // Initialize script journal for crash recovery (bd-izdj.7)
        #[cfg(feature = "scripting")]
        let script_journal = Arc::new(
            crate::script_journal::ScriptJournal::default_dir()
                .map_err(|e| format!("failed to initialize script journal: {e}"))?,
        );

        // Check for interrupted scripts from previous daemon runs
        #[cfg(feature = "scripting")]
        {
            match script_journal.find_interrupted() {
                Ok(interrupted) if !interrupted.is_empty() => {
                    tracing::warn!(
                        count = interrupted.len(),
                        "Found {} interrupted script(s) from previous daemon run",
                        interrupted.len()
                    );
                    for entry in &interrupted {
                        tracing::info!(
                            execution_id = %entry.execution_id,
                            script_id = %entry.script_id,
                            checkpoint = ?entry.checkpoint_name,
                            "Interrupted script marked for review"
                        );
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Failed to scan for interrupted scripts: {}", e);
                }
            }
        }

        // SAFETY (bd-1afe.12): Spawn reaper task to abort running scripts on shutdown.
        // When the shutdown token fires, all running script tasks are aborted before
        // hardware shutdown begins. This prevents scripts from commanding disconnected hardware.
        let running_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        {
            let tasks_map = running_tasks.clone();
            let token = shutdown_token.clone();
            tokio::spawn(async move {
                token.cancelled().await;
                tracing::info!("Shutdown token received: aborting all running scripts");
                let tasks = tasks_map.read().await;
                for (id, handle) in tasks.iter() {
                    tracing::debug!(execution_id = %id, "Aborting script execution");
                    handle.abort();
                }
            });
        }

        Ok(Self {
            #[cfg(feature = "scripting")]
            script_engine: Arc::new(RwLock::new(RhaiEngine::with_hardware().map_err(|e| {
                format!("failed to initialize RhaiEngine with hardware bindings: {e}")
            })?)),
            #[cfg(feature = "scripting")]
            scripts: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "scripting")]
            script_metadata: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "scripting")]
            executions: Arc::new(RwLock::new(HashMap::new())),
            #[cfg(feature = "scripting")]
            running_tasks,
            #[cfg(feature = "scripting")]
            run_engine,
            #[cfg(feature = "scripting")]
            script_journal,
            #[cfg(feature = "scripting")]
            shutdown_token,
            start_time: SystemTime::now(),
            data_tx,
            #[cfg(feature = "storage_hdf5")]
            _ring_buffer: ring_buffer,
        })
    }

    /// Create a new DAQ server instance without storage features.
    /// Requires `run_engine` when scripting feature is enabled (bd-si2c).
    #[cfg(all(not(feature = "storage_hdf5"), feature = "scripting"))]
    pub fn new(
        run_engine: Arc<RunEngine>,
        shutdown_token: CancellationToken,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Create broadcast channel for data distribution
        let (data_tx, _rx) = broadcast::channel(1000);
        let data_tx = Arc::new(data_tx);

        let script_journal = Arc::new(
            crate::script_journal::ScriptJournal::default_dir()
                .map_err(|e| format!("failed to initialize script journal: {}", e))?,
        );

        // Check for interrupted scripts from previous daemon runs
        if let Ok(interrupted) = script_journal.find_interrupted()
            && !interrupted.is_empty()
        {
            tracing::warn!(
                count = interrupted.len(),
                "Found {} interrupted script(s) from previous daemon run",
                interrupted.len()
            );
        }

        // SAFETY (bd-1afe.12): Spawn reaper task to abort running scripts on shutdown.
        let running_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>> =
            Arc::new(RwLock::new(HashMap::new()));
        {
            let tasks_map = running_tasks.clone();
            let token = shutdown_token.clone();
            tokio::spawn(async move {
                token.cancelled().await;
                tracing::info!("Shutdown token received: aborting all running scripts");
                let tasks = tasks_map.read().await;
                for (id, handle) in tasks.iter() {
                    tracing::debug!(execution_id = %id, "Aborting script execution");
                    handle.abort();
                }
            });
        }

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
            running_tasks,
            run_engine,
            script_journal,
            shutdown_token,
            start_time: SystemTime::now(),
            data_tx,
        })
    }

    /// Create a new DAQ server instance with storage but no scripting support.
    #[cfg(all(feature = "storage_hdf5", not(feature = "scripting")))]
    pub fn new(
        ring_buffer: Option<Arc<storage::ring_buffer::RingBuffer>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Create broadcast channel for data distribution (capacity 1000 in-flight messages)
        let (data_tx, _rx) = broadcast::channel(1000);
        let data_tx = Arc::new(data_tx);

        Ok(Self {
            start_time: SystemTime::now(),
            data_tx,
            _ring_buffer: ring_buffer,
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

fn spectrum_payload_from_parts(
    name: String,
    wavelengths: Vec<f64>,
    intensities: Vec<f64>,
    wavelength_unit: Option<String>,
    intensity_unit: Option<String>,
    metadata: Option<serde_json::Value>,
    timestamp_ns: u64,
) -> crate::grpc::proto::SpectrumPayload {
    let metadata_object = metadata.as_ref().and_then(serde_json::Value::as_object);
    let metadata_json = metadata
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_default();

    let merged = metadata_bool(metadata_object, &["merged"]).unwrap_or_else(|| {
        metadata_string(metadata_object, &["kind"]).is_some_and(|kind| kind == "echelle_merged")
            || name.contains("merged")
    });
    let order_index = metadata_i32(metadata_object, &["order_index", "relative_index"])
        .unwrap_or(if merged { -1 } else { 0 });

    crate::grpc::proto::SpectrumPayload {
        wavelengths,
        intensities,
        ivar: metadata_f64_list(metadata_object, &["ivar"]).unwrap_or_default(),
        wavelength_unit: wavelength_unit.unwrap_or_default(),
        order_index,
        merged,
        calibration_profile_id: metadata_string(
            metadata_object,
            &["calibration_profile_id", "profile_id"],
        )
        .unwrap_or_default(),
        quality_flags: metadata_string_list(metadata_object, &["quality_flags"])
            .unwrap_or_default(),
        snr_estimate: metadata_f64(metadata_object, &["snr_estimate"]).unwrap_or_default(),
        name,
        timestamp_ns,
        intensity_unit: intensity_unit.unwrap_or_default(),
        metadata_json,
        device_id: metadata_string(metadata_object, &["device_id"]).unwrap_or_default(),
    }
}

fn metadata_string(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    })
}

fn metadata_bool(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<bool> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_bool)
    })
}

fn metadata_i32(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<i32> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
    })
}

fn metadata_f64(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<f64> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_f64)
    })
}

fn metadata_string_list(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<Vec<String>> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
    })
}

fn metadata_f64_list(
    metadata: Option<&serde_json::Map<String, serde_json::Value>>,
    keys: &[&str],
) -> Option<Vec<f64>> {
    keys.iter().find_map(|key| {
        metadata
            .and_then(|object| object.get(*key))
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_f64)
                    .collect()
            })
    })
}

fn encode_measurement_frame(measurement: &Measurement) -> Result<Vec<u8>, bincode::Error> {
    let payload = bincode::serialize(measurement)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    #[expect(
        clippy::cast_possible_truncation,
        reason = "serialized measurement payload is well under u32::MAX bytes"
    )]
    let payload_len = payload.len() as u32;
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ImageSequenceFault {
    previous_frame_number: u64,
    current_frame_number: u64,
    missing_frames: u64,
}

fn build_image_measurement(device_id: &str, frame: &common::data::Frame) -> Measurement {
    let buffer = match frame.bit_depth {
        16 => {
            if let Some(slice) = frame.as_u16_slice() {
                common::core::PixelBuffer::U16(slice.to_vec())
            } else {
                common::core::PixelBuffer::U8(frame.data.to_vec())
            }
        }
        _ => common::core::PixelBuffer::U8(frame.data.to_vec()),
    };

    let frame_metadata = frame.metadata.as_deref();
    let roi_origin = if frame.roi_x == 0 && frame.roi_y == 0 {
        None
    } else {
        Some((frame.roi_x, frame.roi_y))
    };

    Measurement::Image {
        name: device_id.to_string(),
        width: frame.width,
        height: frame.height,
        buffer,
        unit: "counts".to_string(),
        metadata: common::core::ImageMetadata {
            exposure_ms: frame.exposure_ms,
            gain: None,
            binning: frame_metadata
                .and_then(|meta| meta.binning)
                .map(|(x, y)| (u32::from(x), u32::from(y))),
            temperature_c: frame_metadata.and_then(|meta| meta.temperature_c),
            hardware_timestamp_us: None,
            readout_ms: None,
            roi_origin,
            frame_number: Some(frame.frame_number),
            sequence_gap_from_previous: None,
            summing_count: frame_metadata.and_then(|meta| meta.summing_count),
        },
        timestamp: chrono::Utc::now(),
    }
}

fn load_echelle_profile_for_device(
    registry: &hardware::registry::DeviceRegistry,
    device_id: &str,
) -> Option<Arc<echelle::EchelleCalibrationProfile>> {
    let profile_path = registry.get_driver_config_string(device_id, "echelle_profile_path")?;
    match echelle::EchelleCalibrationProfile::load_from_path(std::path::Path::new(&profile_path)) {
        Ok(profile) => Some(Arc::new(profile)),
        Err(error) => {
            tracing::warn!(
                device_id = %device_id,
                profile_path = %profile_path,
                error = %error,
                "Failed to load echelle profile; server-side extraction disabled for device"
            );
            None
        }
    }
}

fn detect_image_sequence_fault(
    previous_frame_number: Option<u64>,
    current_frame_number: u64,
) -> Option<ImageSequenceFault> {
    let previous_frame_number = previous_frame_number?;
    if current_frame_number > previous_frame_number.saturating_add(1) {
        return Some(ImageSequenceFault {
            previous_frame_number,
            current_frame_number,
            missing_frames: current_frame_number - previous_frame_number - 1,
        });
    }
    None
}

fn annotate_measurement_data_integrity(
    measurement: &mut Measurement,
    last_frame_numbers: &mut HashMap<String, u64>,
) -> Option<(String, ImageSequenceFault)> {
    let Measurement::Image { name, metadata, .. } = measurement else {
        return None;
    };
    let current_frame_number = metadata.frame_number?;
    let previous_frame_number = last_frame_numbers.get(name).copied();
    let fault = detect_image_sequence_fault(previous_frame_number, current_frame_number);

    if let Some(fault) = fault {
        metadata.sequence_gap_from_previous = Some(fault.missing_frames);
    }

    let next_frame_number = previous_frame_number.map_or(current_frame_number, |previous| {
        previous.max(current_frame_number)
    });
    last_frame_numbers.insert(name.clone(), next_frame_number);

    fault.map(|fault| (name.clone(), fault))
}

fn log_data_integrity_fault(source: &str, fault: ImageSequenceFault) {
    tracing::warn!(
        target: "data_integrity",
        source,
        previous_frame_number = fault.previous_frame_number,
        current_frame_number = fault.current_frame_number,
        missing_frames = fault.missing_frames,
        "DataIntegrityFault: {} dropped {} frame(s) (prev={}, current={})",
        source,
        fault.missing_frames,
        fault.previous_frame_number,
        fault.current_frame_number
    );
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
            .finish_non_exhaustive()
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
    #[expect(
        clippy::cast_possible_truncation,
        reason = "script content size fits in protobuf u32 field"
    )]
    async fn upload_script(
        &self,
        request: Request<UploadRequest>,
    ) -> Result<Response<UploadResponse>, Status> {
        let req = request.into_inner();
        let script_id = Uuid::new_v4().to_string();

        let script_size = req.script_content.len();

        // SECURITY AUDIT (bd-qa36.8.2): Log all script uploads for audit trail.
        tracing::info!(
            script_id = %script_id,
            script_name = %req.name,
            script_size = script_size,
            "AUDIT: Script upload received"
        );

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
                error_message: format!("Validation failed: {e}"),
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
    #[expect(
        clippy::cast_possible_truncation,
        reason = "script execution timestamp nanos fit in protobuf u64 field"
    )]
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

        // Persist to journal for crash recovery (bd-izdj.7)
        if let Err(e) = self
            .script_journal
            .start_run(&execution_id, &req.script_id, script)
        {
            tracing::warn!(execution_id = %execution_id, "Failed to journal script start: {}", e);
        }

        // SAFETY (bd-1afe.12): Reject new scripts if shutdown is in progress.
        // Without this check, a script could start after the reaper has already fired.
        if self.shutdown_token.is_cancelled() {
            return Err(Status::unavailable("Server is shutting down"));
        }

        // Execute script via ScriptPlanRunner using shared RunEngine (bd-si2c)
        // This ensures all yielded plans emit Documents and coordinate with gRPC services
        let script_clone = script.clone();
        let run_engine_clone = self.run_engine.clone();
        let executions_clone = self.executions.clone();
        let exec_id_clone = execution_id.clone();
        let running_tasks_clone = self.running_tasks.clone();
        let exec_id_for_cleanup = execution_id.clone();
        let journal_clone = self.script_journal.clone();

        // SAFETY (bd-1afe.12): Acquire write lock BEFORE spawning the task.
        // This closes the race window where the reaper could fire between
        // tokio::spawn and running_tasks.insert, missing the new task.
        let mut tasks = self.running_tasks.write().await;
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
                        // Journal completion/failure (bd-izdj.7)
                        if report.success {
                            if let Err(e) = journal_clone.complete_run(&exec_id_clone) {
                                tracing::warn!("Failed to journal script completion: {}", e);
                            }
                        } else if let Some(ref err_msg) = report.error
                            && let Err(e) = journal_clone.fail_run(&exec_id_clone, err_msg)
                        {
                            tracing::warn!("Failed to journal script failure: {}", e);
                        }
                    }
                    Err(e) => {
                        exec.state = "ERROR".to_string();
                        exec.error = Some(e.to_string());
                        // Journal error (bd-izdj.7)
                        if let Err(je) = journal_clone.fail_run(&exec_id_clone, &e.to_string()) {
                            tracing::warn!("Failed to journal script error: {}", je);
                        }
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
        tasks.insert(execution_id.clone(), handle);
        drop(tasks);

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
            #[expect(
                clippy::cast_possible_truncation,
                reason = "Unix epoch nanos will not exceed u64::MAX until year 2554"
            )]
            let end_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            exec.end_time = Some(end_ns);
        }

        // Persist stop to journal so restart doesn't show "interrupted" (bd-izdj)
        if let Err(e) = self
            .script_journal
            .fail_run(&req.execution_id, "stopped by user")
        {
            tracing::warn!(
                execution_id = %req.execution_id,
                "Failed to journal script stop: {}",
                e
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
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "precision loss acceptable for metrics/display"
                )]
                let used_memory_mb = used_memory_kb as f64 / 1024.0;

                // Determine engine state based on CPU activity
                // This is a simple heuristic: if CPU > 1%, we consider it RUNNING
                let current_state = if cpu_usage_percent > 1.0 {
                    "RUNNING".to_string()
                } else {
                    "IDLE".to_string()
                };

                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "Unix epoch nanos will not exceed u64::MAX until year 2554"
                )]
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
    type StreamSpectraStream =
        tokio_stream::wrappers::ReceiverStream<Result<crate::grpc::proto::SpectrumPayload, Status>>;

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
                    1.0 / f64::from(max_rate_hz),
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
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "value is non-negative at this point"
                        )]
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        (name.clone(), *value, ts_ns)
                    }
                    Measurement::Vector {
                        name,
                        values,
                        timestamp,
                        ..
                    } => {
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "timestamp nanos are non-negative"
                        )]
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        #[expect(
                            clippy::cast_precision_loss,
                            reason = "precision loss acceptable for metrics/display"
                        )]
                        let len_f64 = values.len() as f64;
                        // For vectors, we can emit the length or first value
                        (format!("{name}_len"), len_f64, ts_ns)
                    }
                    Measurement::Image {
                        name,
                        width,
                        height,
                        timestamp,
                        ..
                    } => {
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "timestamp nanos are non-negative"
                        )]
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        (name.clone(), f64::from(width * height), ts_ns)
                    }
                    Measurement::Spectrum {
                        name,
                        amplitudes,
                        timestamp,
                        ..
                    } => {
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "timestamp nanos are non-negative"
                        )]
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        #[expect(
                            clippy::cast_precision_loss,
                            reason = "precision loss acceptable for metrics/display"
                        )]
                        let len_f64 = amplitudes.len() as f64;
                        (format!("{name}_spectrum"), len_f64, ts_ns)
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

    /// Stream full spectrum measurements (additive path; preserves StreamMeasurements behavior)
    async fn stream_spectra(
        &self,
        request: Request<crate::grpc::proto::SpectrumStreamRequest>,
    ) -> Result<Response<Self::StreamSpectraStream>, Status> {
        let req = request.into_inner();
        let (tx, rx) = tokio::sync::mpsc::channel(32);

        let mut data_rx = self.data_tx.subscribe();
        let channels = req.channels;
        let max_rate_hz = req.max_rate_hz;

        tokio::spawn(async move {
            let mut rate_limiter = if max_rate_hz > 0 {
                Some(tokio::time::interval(std::time::Duration::from_secs_f64(
                    1.0 / f64::from(max_rate_hz),
                )))
            } else {
                None
            };

            let mut last_lag_warning = std::time::Instant::now();
            let mut total_skipped: u64 = 0;

            loop {
                let data_point = match data_rx.recv().await {
                    Ok(dp) => dp,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        total_skipped += skipped;
                        if last_lag_warning.elapsed() > std::time::Duration::from_secs(1) {
                            tracing::debug!(
                                skipped = total_skipped,
                                "gRPC client lagged behind spectrum stream, skipped measurements"
                            );
                            total_skipped = 0;
                            last_lag_warning = std::time::Instant::now();
                        }
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };

                let Measurement::Spectrum {
                    name,
                    frequencies,
                    amplitudes,
                    frequency_unit,
                    amplitude_unit,
                    metadata,
                    timestamp,
                } = data_point
                else {
                    continue;
                };

                let spectrum_device_id = metadata_string(
                    metadata.as_ref().and_then(serde_json::Value::as_object),
                    &["device_id"],
                );
                if !channels.is_empty()
                    && !channels.contains(&name)
                    && spectrum_device_id
                        .as_ref()
                        .is_none_or(|device_id| !channels.contains(device_id))
                {
                    continue;
                }

                if let Some(ref mut limiter) = rate_limiter {
                    limiter.tick().await;
                }

                #[expect(clippy::cast_sign_loss, reason = "value is non-negative at this point")]
                let timestamp_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                let proto_spectrum = spectrum_payload_from_parts(
                    name,
                    frequencies,
                    amplitudes,
                    frequency_unit,
                    amplitude_unit,
                    metadata,
                    timestamp_ns,
                );

                if tx.send(Ok(proto_spectrum)).await.is_err() {
                    break;
                }

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
    #[expect(
        clippy::vec_init_then_push,
        reason = "conditional cfg-gated pushes can't use vec![] macro"
    )]
    async fn get_daemon_info(
        &self,
        _request: Request<DaemonInfoRequest>,
    ) -> Result<Response<DaemonInfoResponse>, Status> {
        #[expect(
            unused_mut,
            reason = "features.push() only compiles under cfg'd features"
        )]
        let mut features = Vec::new();

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

/// Build a gRPC `Server` with the standard middleware stack (CORS, auth, tracing, audit).
///
/// Extracted as a macro because Tower's builder returns deeply nested generic types
/// that cannot be named in a function signature without `type_alias_impl_trait`.
macro_rules! build_grpc_server {
    ($grpc_settings:expr, $cors:expr, $web_ui_path:expr) => {{
        use crate::grpc::audit_log::AuditLogLayer;
        use crate::grpc::request_tracing::RequestTracingLayer;
        let auth_settings = $grpc_settings.clone();
        Server::builder()
            .accept_http1(true)
            // HTTP/2 flow control optimization (bd-rgnx.11): larger window sizes reduce
            // flow control overhead for high-bandwidth camera frame streaming.
            // 2 MB stream window, 4 MB connection window (defaults are 64 KB / 64 KB).
            .initial_stream_window_size(2 * 1024 * 1024)
            .initial_connection_window_size(4 * 1024 * 1024)
            // Static file serving layer (bd-j8k9): must be outermost to intercept
            // non-gRPC requests before CORS/auth processing
            .layer(WebUiLayer::new($web_ui_path))
            .layer($cors)
            .layer(interceptor(move |request: Request<()>| {
                validate_auth(&auth_settings, &request)?;
                Ok(request)
            }))
            .layer(RequestTracingLayer::new())
            .layer(AuditLogLayer::new())
    }};
}

/// Start the DAQ gRPC server
///
/// Provides RunEngineService and optionally ControlService (when `scripting` feature is enabled).
/// ControlService includes script execution, stream_measurements, and stream_status methods.
pub async fn start_server(addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    use crate::grpc::health_service::HealthServiceImpl;
    use crate::grpc::proto::health::health_check_response::ServingStatus;
    use crate::grpc::proto::health::health_server::HealthServer;

    let config = ServerConfig::load()?;
    let grpc_settings = config.grpc;

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
    let registry = std::sync::Arc::new(hardware::registry::DeviceRegistry::new());
    let run_engine_instance = std::sync::Arc::new(experiment::RunEngine::new(registry));

    // Create DaqServer with shared RunEngine when scripting enabled (bd-si2c)
    // start_server() has no external shutdown coordination, so create a standalone token
    #[cfg(feature = "scripting")]
    let standalone_token = CancellationToken::new();
    #[cfg(all(feature = "scripting", feature = "storage_hdf5"))]
    let server = DaqServer::new(None, run_engine_instance.clone(), standalone_token)?;
    #[cfg(all(feature = "scripting", not(feature = "storage_hdf5")))]
    let server = DaqServer::new(run_engine_instance.clone(), standalone_token)?;
    #[cfg(all(not(feature = "scripting"), feature = "storage_hdf5"))]
    let server = DaqServer::new(None)?;
    #[cfg(all(not(feature = "scripting"), not(feature = "storage_hdf5")))]
    let server = DaqServer::new()?;

    let run_engine = RunEngineServiceImpl::new(
        run_engine_instance,
        #[cfg(feature = "metrics")]
        None, // No DaqMetrics in legacy start_server path
    );

    let health_service = HealthServiceImpl::new();

    health_service.set_serving_status("", ServingStatus::Serving);
    health_service.set_serving_status("daq.ControlService", ServingStatus::Serving);
    health_service.set_serving_status("daq.RunEngineService", ServingStatus::Serving);

    println!("DAQ gRPC server listening on {bind_addr}");

    if !grpc_settings.auth_enabled {
        // SECURITY (bd-qa36.8.2): Script upload/execution endpoints accept arbitrary
        // Rhai code. Without auth, any network client can execute scripts with daemon
        // privileges. This warning is intentionally loud.
        eprintln!("⚠️  gRPC auth is disabled (set grpc.auth_enabled=true to require auth)");
        eprintln!("🚨 SECURITY: Script upload/execution endpoints are unauthenticated!");
        eprintln!("   Any client can upload and execute Rhai scripts with daemon privileges.");
        eprintln!("   Set grpc.auth_enabled=true and grpc.auth_token in production.");
    }

    let cors = build_cors_layer(&grpc_settings)?;
    let tls_config = build_tls_config(&grpc_settings)?;
    if tls_config.is_none() {
        eprintln!(
            "⚠️  gRPC TLS is disabled (set grpc.tls_cert_path + grpc.tls_key_path to enable)"
        );
    }

    let mut builder = build_grpc_server!(grpc_settings, cors, grpc_settings.web_ui_path.as_ref());

    if let Some(tls_config) = tls_config {
        builder = builder.tls_config(tls_config)?;
    }

    #[cfg(feature = "scripting")]
    let builder = builder.add_service(tonic_web::enable(
        ControlServiceServer::new(server).max_encoding_message_size(64 * 1024 * 1024),
    ));

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
/// use server::grpc::start_server_with_hardware;
/// use hardware::registry::create_mock_registry;
/// use std::sync::Arc;
/// use tokio::sync::RwLock;
///
/// let registry = create_mock_registry().await?;
/// let addr = "127.0.0.1:50051".parse()?;
/// start_server_with_hardware(addr, Arc::new(RwLock::new(registry))).await?;
/// ```
pub async fn start_server_with_hardware(
    addr: std::net::SocketAddr,
    registry: std::sync::Arc<hardware::registry::DeviceRegistry>,
    health_monitor: std::sync::Arc<common::health::SystemHealthMonitor>,
    shutdown_token: CancellationToken,
    #[cfg(feature = "db-surreal")] _db: Option<db::DaqDb>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::grpc::hardware_service::HardwareServiceImpl;
    use crate::grpc::module_service::ModuleServiceImpl;
    use crate::grpc::ni_daq_service::NiDaqServiceImpl;
    use storage::ring_buffer::RingBuffer;
    // use crate::grpc::plugin_service::PluginServiceImpl; // Unused
    use crate::grpc::preset_service::{PresetServiceImpl, default_preset_storage_path};
    use crate::grpc::proto::hardware_service_server::HardwareServiceServer;
    use crate::grpc::proto::health::health_check_response::ServingStatus;
    use crate::grpc::proto::health::health_server::HealthServer;
    use crate::grpc::proto::health_service_server::HealthServiceServer; // Custom HealthService
    use crate::grpc::proto::module_service_server::ModuleServiceServer;
    use protocol::ni_daq::ni_daq_service_server::NiDaqServiceServer;
    use tonic::codec::CompressionEncoding;
    // use crate::grpc::proto::plugin_service_server::PluginServiceServer; // Unused
    use crate::grpc::proto::preset_service_server::PresetServiceServer;
    use crate::grpc::proto::scan_service_server::ScanServiceServer;
    use crate::grpc::proto::storage_service_server::StorageServiceServer;
    // LEGACY: ScanService import, remove at v1.0. See deprecation-plan.md 1.2.
    #[expect(
        deprecated,
        reason = "LEGACY: ScanService kept for backwards compatibility, remove at v1.0"
    )]
    use crate::grpc::scan_service::ScanServiceImpl;
    use crate::grpc::storage_service::StorageServiceImpl;

    let config = ServerConfig::load()?;
    let grpc_settings = config.grpc;
    let storage_settings = config.storage;

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

    // Create ring buffer for scan data persistence (The Mullet Strategy)
    // Use /dev/shm on Linux, /tmp on macOS for memory-mapped storage
    let ring_buffer_path = storage_settings.ring_buffer_path.clone();
    let ring_buffer_size = storage_settings.ring_buffer_size_mb as u64;

    #[expect(
        clippy::cast_possible_truncation,
        reason = "ring buffer size in MB fits in usize on 64-bit targets"
    )]
    let ring_buffer = match tokio::task::spawn_blocking(move || {
        RingBuffer::create(&ring_buffer_path, ring_buffer_size as usize)
    })
    .await
    {
        Ok(Ok(rb)) => {
            println!(
                "  - RingBuffer: {} ({} MB)",
                storage_settings.ring_buffer_path.display(),
                ring_buffer_size
            );
            Some(std::sync::Arc::<storage::ring_buffer::RingBuffer>::new(rb))
        }
        Ok(Err(e)) => {
            eprintln!(
                "Warning: Failed to create ring buffer: {e}. Scan data will not be persisted."
            );
            None
        }
        Err(e) => {
            eprintln!("Warning: Ring buffer creation task panicked or was cancelled: {e}");
            None
        }
    };

    if let Some(ref rb) = ring_buffer {
        rb.set_tap_channel_capacity(storage_settings.tap_channel_size);
    }

    // NOTE: HDF5Writer is NOT spawned here at startup. Frames are only written
    // to disk when explicitly requested via StorageService::start_recording().
    // The ring buffer acts as a circular in-memory buffer that StorageService
    // taps into on demand.

    // Create shared RunEngine FIRST - used by both RunEngineService and ControlService/scripts (bd-si2c)
    let run_engine = std::sync::Arc::new(experiment::RunEngine::new(registry.clone()));

    // Initialize control server WITHOUT internal RingBuffer logic (we wire it manually)
    // Pass shared run_engine for script execution (bd-si2c)
    #[cfg(all(feature = "storage_hdf5", feature = "scripting"))]
    let control_server = DaqServer::new(None, run_engine.clone(), shutdown_token.clone())?;
    #[cfg(all(feature = "storage_hdf5", not(feature = "scripting")))]
    let control_server = DaqServer::new(None)?;
    #[cfg(all(not(feature = "storage_hdf5"), feature = "scripting"))]
    let control_server = DaqServer::new(run_engine.clone(), shutdown_token.clone())?;
    #[cfg(all(not(feature = "storage_hdf5"), not(feature = "scripting")))]
    let control_server = DaqServer::new()?;

    // Setup Reliable Sink (RingBuffer Writer)
    let reliable_sink_tx = if let Some(ref rb) = ring_buffer {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Measurement>(512);
        let rb_clone = rb.clone();

        // Spawn writer task
        tokio::spawn(async move {
            let mut last_frame_numbers = HashMap::<String, u64>::new();
            while let Some(mut measurement) = rx.recv().await {
                if let Some((source, fault)) =
                    annotate_measurement_data_integrity(&mut measurement, &mut last_frame_numbers)
                {
                    log_data_integrity_fault(&source, fault);
                }

                if let Ok(frame) = encode_measurement_frame(&measurement) {
                    let rb = rb_clone.clone();
                    // Offload blocking mmap write to avoid stalling Tokio
                    // workers under contention (bd-tvp6).
                    if let Err(e) = tokio::task::spawn_blocking(move || rb.write(&frame))
                        .await
                        .expect("ring buffer write task panicked")
                    {
                        tracing::error!(error = %e, "Failed to write measurement to ring buffer");
                    }
                }
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
            println!("  - Wiring pipeline for device: {device_id}");

            // 1. Create channel for Source output (Arc<Frame>)
            let frame_chan = std::env::var("DAQ_PIPELINE_FRAME_CH")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(16);
            let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(frame_chan);

            // 2. Register source output (ASYNC - safe now, no lock held)
            if let Err(e) = source.register_output(frame_tx).await {
                eprintln!("Failed to register output for {device_id}: {e}");
                continue;
            }

            // 3. Create channel for Measurement (Tee Input)
            let meas_chan = std::env::var("DAQ_PIPELINE_MEAS_CH")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(16);
            let (meas_tx, meas_rx) = tokio::sync::mpsc::channel(meas_chan);
            let image_meas_tx = meas_tx.clone();
            let device_id_clone = device_id.clone();
            let extraction_feed =
                load_echelle_profile_for_device(&registry, &device_id).map(|profile| {
                    let (extract_tx, mut extract_rx) =
                        tokio::sync::watch::channel(None::<Arc<common::data::Frame>>);
                    let spectrum_meas_tx = meas_tx.clone();
                    let extraction_device_id = device_id.clone();
                    let extraction_profile = profile.clone();
                    tokio::spawn(async move {
                        let mut u16_scratch = Vec::new();
                        while extract_rx.changed().await.is_ok() {
                            let Some(frame) = extract_rx.borrow_and_update().clone() else {
                                continue;
                            };

                            match echelle::extract_preview_with_u16_scratch(
                                extraction_profile.as_ref(),
                                &frame.data,
                                frame.width,
                                frame.height,
                                frame.bit_depth,
                                frame.frame_number,
                                &mut u16_scratch,
                            ) {
                                Ok(preview) => {
                                    for measurement in preview.to_measurements_with_context(
                                        false,
                                        Some(&extraction_device_id),
                                    ) {
                                        if spectrum_meas_tx.send(measurement).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        device_id = %extraction_device_id,
                                        frame_number = frame.frame_number,
                                        error = %error,
                                        "Echelle extraction failed; raw frame streaming continues"
                                    );
                                }
                            }

                            tokio::task::yield_now().await;
                        }
                    });

                    tracing::info!(
                        device_id = %device_id,
                        "Enabled server-side echelle spectrum extraction for frame source"
                    );

                    extract_tx
                });

            // 4. Spawn Converter Task (Frame -> Measurement)
            tokio::spawn(async move {
                while let Some(frame) = frame_rx.recv().await {
                    let measurement = build_image_measurement(&device_id_clone, &frame);

                    if image_meas_tx.send(measurement).await.is_err() {
                        break; // Downstream closed
                    }
                    if let Some(extract_tx) = &extraction_feed {
                        extract_tx.send_replace(Some(frame.clone()));
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
                eprintln!("Failed to register Tee input for {device_id}: {e}");
            }
        }
    }

    // Game loop broadcast channel — created early so Rerun can subscribe (Phase 6).
    // Drivers → mpsc → game loop → broadcast → { gRPC StreamSystemState, Rerun }
    let (state_broadcast_tx, _) =
        tokio::sync::broadcast::channel::<common::state_cache::SystemStateSnapshot>(64);

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
                    .unwrap_or_else(|_| "crates/server/blueprints/daq_default.rbl".to_string());
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
                            "    - Blueprint: not found at {} (generate with `python crates/server/blueprints/generate_blueprints.py`)",
                            blueprint_choice
                        ),
                        Err(e) => eprintln!(
                            "Warning: Failed to load blueprint {}: {}",
                            blueprint_choice, e
                        ),
                    }
                }

                rerun.monitor_broadcast(control_server.data_sender().subscribe());
                rerun.monitor_system_state(state_broadcast_tx.subscribe());
            }
            Err(e) => {
                eprintln!("Warning: Failed to start Rerun visualization: {}", e);
            }
        }
    }

    // RunEngine was already created above (bd-si2c) - shared between RunEngineService and scripts
    // Wire DaqMetrics into RunEngineService for Prometheus observability (bd-2r60, bd-4de1)
    #[cfg(feature = "metrics")]
    let daq_metrics = {
        let m = Arc::new(crate::grpc::metrics_service::DaqMetrics::new());
        tracing::info!("DaqMetrics wired to RunEngineService for Prometheus export");
        Some(m)
    };
    let run_engine_server = {
        let svc = RunEngineServiceImpl::new(
            run_engine.clone(),
            #[cfg(feature = "metrics")]
            daq_metrics,
        );
        #[cfg(feature = "db-surreal")]
        let svc = svc.with_db(_db.clone());
        svc
    };

    // Spawn orphan-plan watchdog (bd-c9z1)
    // Detects plans stuck in Running/Paused with no client activity and aborts them.
    let _watchdog_handle = run_engine.spawn_watchdog();

    // Spawn webhook alerting task if configured (bd-kctc)
    #[cfg(feature = "alerting")]
    if let Some(notifier) = crate::alerting::WebhookNotifier::new(config.alerting.clone()) {
        let notifier = std::sync::Arc::new(notifier);
        let health_rx = registry.subscribe_health_changes();
        let doc_rx = run_engine.subscribe();
        let alert_cancel = shutdown_token.clone();
        tokio::spawn(crate::alerting::run_alerting_task(
            notifier,
            health_rx,
            doc_rx,
            alert_cancel,
        ));
        tracing::info!("Webhook alerting enabled");
    }

    // Spawn heartbeat logger for overnight run forensics (bd-7xqd)
    {
        let hb_config = crate::health::heartbeat_log::HeartbeatLogConfig {
            storage_path: storage_settings.output_directory.clone(),
            ..Default::default()
        };
        let hb_registry = registry.clone();
        let hb_engine = run_engine.clone();
        let hb_cancel = shutdown_token.clone();
        tokio::spawn(crate::health::heartbeat_log::run_heartbeat_log(
            hb_registry,
            hb_engine,
            hb_config,
            hb_cancel,
        ));
    }

    // Pause the engine if disk or process RSS crosses danger thresholds (bd-102j).
    tokio::spawn(
        crate::health::sys_monitor::ResourceGuard::new(
            health_monitor.clone(),
            run_engine.clone(),
            storage_settings.output_directory.clone(),
        )
        .run(),
    );

    // Game loop for system state broadcasting (Phase 4)
    //
    // Drivers → mpsc → game loop → broadcast → { gRPC StreamSystemState, Rerun }
    // The state poller reads all Readable devices at 10 Hz and feeds the game loop.
    let (state_update_tx, state_update_rx) = tokio::sync::mpsc::channel(256);

    let shutdown_gl = shutdown_token.clone();
    tokio::spawn(common::state_cache::run_game_loop(
        state_update_rx,
        state_broadcast_tx.clone(),
        common::state_cache::GameLoopConfig::default(),
        shutdown_gl,
    ));

    // State poller: reads all Readable devices and pushes updates to the game loop.
    let poller_registry = registry.clone();
    let poller_shutdown = shutdown_token.clone();
    tokio::spawn(async move {
        use common::state_cache::{NodeStateUpdate, NodeValue, now_ns};

        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! {
                () = poller_shutdown.cancelled() => break,
                _ = tick.tick() => {
                    for device_info in poller_registry.list_devices() {
                        if let Some(readable) = poller_registry.get_readable(&device_info.id)
                            && let Ok(value) = readable.read().await
                        {
                            let _ = state_update_tx.try_send(NodeStateUpdate {
                                device_id: device_info.id.clone(),
                                timestamp_ns: now_ns(),
                                value: NodeValue::Analog(value),
                                metadata: std::collections::HashMap::new(),
                            });
                        }
                    }
                }
            }
        }
    });

    let hardware_server = {
        let svc =
            HardwareServiceImpl::new(registry.clone()).with_state_broadcast(state_broadcast_tx);
        #[cfg(feature = "db-surreal")]
        let svc = svc.with_db(_db.clone());
        svc
    };
    let module_server = ModuleServiceImpl::new(registry.clone());
    let ni_daq_server = NiDaqServiceImpl::new(registry.clone());

    // Create PluginService with shared factory and registry (bd-0451)
    #[cfg(feature = "serial")]
    let plugin_server = {
        // No lock needed for registry anymore
        let factory = registry.plugin_factory();
        PluginServiceImpl::new(factory, registry.clone())
    };

    // LEGACY: ScanService wiring, deprecated in favor of RunEngineService.
    // Remove at v1.0. See docs/reference/deprecation-plan.md Section 1.2.
    #[expect(
        deprecated,
        reason = "LEGACY: ScanService kept for backwards compatibility, remove at v1.0"
    )]
    let scan_server = if let Some(rb) = ring_buffer.clone() {
        ScanServiceImpl::new(registry.clone()).with_ring_buffer(rb)
    } else {
        ScanServiceImpl::new(registry.clone())
    };

    let preset_server = PresetServiceImpl::new(registry.clone(), default_preset_storage_path());
    let storage_server = StorageServiceImpl::new(ring_buffer.clone(), storage_settings);

    // Standard gRPC Health Check (grpc.health.v1)
    let standard_health_service = crate::grpc::health_service::HealthServiceImpl::new();

    // Custom System Health Monitoring (with per-device health via bd-qa36.4.3)
    let custom_health_service =
        crate::grpc::custom_health_service::HealthServiceImpl::new(health_monitor)
            .with_registry(registry.clone());

    // Wire DB state into health responses (bd-9n9k.3)
    #[cfg(feature = "db-surreal")]
    let custom_health_service = {
        let (db_available, engine, message) = if let Some(ref db) = _db {
            let info = db.info().await;
            // Expose only engine kind ("rocksdb", "mem") — avoid leaking filesystem paths.
            let engine_kind = info
                .engine
                .split(':')
                .next()
                .unwrap_or("unknown")
                .to_string();
            // Base availability on actual DB health, not just init success.
            let healthy = info.healthy;
            (
                healthy,
                Some(engine_kind),
                Some(if healthy {
                    format!(
                        "schema v{}, {} drivers, {} instruments",
                        info.schema_version, info.driver_count, info.instrument_count
                    )
                } else {
                    "Database initialized but health check failing".to_string()
                }),
            )
        } else {
            (
                false,
                None,
                Some("Database initialization failed or not configured".to_string()),
            )
        };
        custom_health_service.with_db_state(db_available, engine, message)
    };

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
    // Register ConfigService status based on DB availability (bd-9n9k.3)
    #[cfg(feature = "db-surreal")]
    if _db.is_some() {
        standard_health_service.set_serving_status("daq.ConfigService", ServingStatus::Serving);
    } else {
        standard_health_service.set_serving_status("daq.ConfigService", ServingStatus::NotServing);
    }

    println!("DAQ gRPC server (with hardware) listening on {bind_addr}");
    println!("  - ControlService: script management");
    println!("  - HardwareService: direct device control");
    println!("  - HealthService: system health monitoring (bd-ergo)");
    println!("  - ModuleService: experiment modules (bd-c0ai)");
    #[cfg(feature = "serial")]
    println!("  - PluginService: YAML-defined instrument plugins (bd-0451)");
    println!("  - ScanService: coordinated multi-axis scans");
    println!("  - PresetService: configuration save/load (bd-akcm)");
    println!("  - StorageService: HDF5 data storage (bd-p6im)");

    println!("  - AuditLog: hardware-mutating calls → tracing target \"audit\" (bd-1afe.10)");

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
    let mut server_builder = build_grpc_server!(
        grpc_settings,
        cors.clone(),
        grpc_settings.web_ui_path.as_ref()
    );

    #[cfg(feature = "serial")]
    if let Some(tls_config) = tls_config {
        server_builder = server_builder.tls_config(tls_config)?;
    }

    #[cfg(all(feature = "serial", feature = "scripting"))]
    let server_builder = server_builder.add_service(tonic_web::enable(
        ControlServiceServer::new(control_server).max_encoding_message_size(64 * 1024 * 1024),
    ));

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
        // gRPC-level compression (bd-rgnx.11): enable gzip for frame streaming responses
        .add_service(tonic_web::enable(
            HardwareServiceServer::new(hardware_server)
                .max_encoding_message_size(64 * 1024 * 1024)
                .accept_compressed(CompressionEncoding::Gzip)
                .send_compressed(CompressionEncoding::Gzip),
        ))
        .add_service(tonic_web::enable(NiDaqServiceServer::new(
            ni_daq_server.clone(),
        )))
        .add_service(tonic_web::enable(ModuleServiceServer::new(module_server)))
        .add_service(tonic_web::enable(PluginServiceServer::new(plugin_server)))
        .add_service(tonic_web::enable(ScanServiceServer::new(scan_server)))
        .add_service(tonic_web::enable(PresetServiceServer::new(preset_server)))
        .add_service(tonic_web::enable(StorageServiceServer::new(storage_server)));

    // ConfigService — SurrealDB config management (bd-itsc)
    #[cfg(all(feature = "serial", feature = "db-surreal"))]
    let server_builder = if let Some(ref db) = _db {
        server_builder.add_service(tonic_web::enable(
            crate::grpc::proto::config_service_server::ConfigServiceServer::new(
                crate::grpc::config_service::ConfigServiceImpl::new(
                    db.clone(),
                    Some(registry.clone()),
                ),
            ),
        ))
    } else {
        server_builder
    };

    #[cfg(not(feature = "serial"))]
    let mut server_builder = build_grpc_server!(
        grpc_settings,
        cors.clone(),
        grpc_settings.web_ui_path.as_ref()
    );

    #[cfg(not(feature = "serial"))]
    if let Some(tls_config) = tls_config {
        server_builder = server_builder.tls_config(tls_config)?;
    }

    #[cfg(all(not(feature = "serial"), feature = "scripting"))]
    let server_builder = server_builder.add_service(tonic_web::enable(
        ControlServiceServer::new(control_server).max_encoding_message_size(64 * 1024 * 1024),
    ));

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
        // gRPC-level compression (bd-rgnx.11): enable gzip for frame streaming responses
        .add_service(tonic_web::enable(
            HardwareServiceServer::new(hardware_server)
                .max_encoding_message_size(64 * 1024 * 1024)
                .accept_compressed(CompressionEncoding::Gzip)
                .send_compressed(CompressionEncoding::Gzip),
        ))
        .add_service(tonic_web::enable(NiDaqServiceServer::new(ni_daq_server)))
        .add_service(tonic_web::enable(ModuleServiceServer::new(module_server)))
        .add_service(tonic_web::enable(ScanServiceServer::new(scan_server)))
        .add_service(tonic_web::enable(PresetServiceServer::new(preset_server)))
        .add_service(tonic_web::enable(StorageServiceServer::new(storage_server)));

    // ConfigService — SurrealDB config management (bd-itsc)
    #[cfg(all(not(feature = "serial"), feature = "db-surreal"))]
    let server_builder = if let Some(ref db) = _db {
        server_builder.add_service(tonic_web::enable(
            crate::grpc::proto::config_service_server::ConfigServiceServer::new(
                crate::grpc::config_service::ConfigServiceImpl::new(
                    db.clone(),
                    Some(registry.clone()),
                ),
            ),
        ))
    } else {
        server_builder
    };

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

    // SAFETY (bd-1afe.12): Use serve_with_shutdown so the daemon can trigger graceful
    // server termination. When shutdown_token is cancelled, the server stops accepting
    // new connections and drains existing RPCs. The reaper task in DaqServer concurrently
    // aborts running scripts to prevent commands on disconnected hardware.
    if grpc_settings.web_ui_path.is_some() {
        println!(
            "🚀 Serving integrated WASM UI from {} on port {}",
            grpc_settings.web_ui_path.as_ref().unwrap().display(),
            bind_addr.port()
        );
    }

    server_builder
        .serve_with_shutdown(bind_addr, shutdown_token.cancelled())
        .await?;

    Ok(())
}

// ── Integrated WASM UI serving (bd-j8k9) ──────────────────────────────────────
//
// Tower Layer that intercepts non-gRPC requests and serves static files from
// the WASM UI build directory. When `web_ui_path` is None, the layer is a
// transparent pass-through. This sits as the outermost layer so that non-gRPC
// requests never reach the CORS or auth middleware.

/// Wrapper that maps `io::Error` body errors to `tonic::Status` for BoxBody compatibility.
struct IoToStatusBody<B>(B);

impl<B> http_body::Body for IoToStatusBody<B>
where
    B: http_body::Body<Data = prost::bytes::Bytes, Error = std::io::Error> + Unpin,
{
    type Data = prost::bytes::Bytes;
    type Error = Status;

    fn poll_data(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<Self::Data, Self::Error>>> {
        std::pin::Pin::new(&mut self.0).poll_data(cx).map(
            |opt: Option<Result<prost::bytes::Bytes, std::io::Error>>| {
                opt.map(|res| res.map_err(|e| Status::internal(format!("file error: {e}"))))
            },
        )
    }

    fn poll_trailers(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<Option<http::HeaderMap>, Self::Error>> {
        std::pin::Pin::new(&mut self.0)
            .poll_trailers(cx)
            .map_err(|e| Status::internal(format!("file error: {e}")))
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.0.size_hint()
    }
}

/// Tower Layer that serves static WASM UI files for non-gRPC requests.
#[derive(Clone)]
struct WebUiLayer {
    serve_dir: Option<tower_http::services::ServeDir>,
}

impl WebUiLayer {
    fn new(path: Option<&std::path::PathBuf>) -> Self {
        Self {
            serve_dir: path.map(|p| {
                tower_http::services::ServeDir::new(p).append_index_html_on_directories(true)
            }),
        }
    }
}

impl<S> tower_layer::Layer<S> for WebUiLayer {
    type Service = WebUiService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        WebUiService {
            inner,
            serve_dir: self.serve_dir.clone(),
        }
    }
}

/// Tower Service that routes gRPC requests to the inner service and
/// non-gRPC requests to a static file server.
#[derive(Clone)]
struct WebUiService<S> {
    inner: S,
    serve_dir: Option<tower_http::services::ServeDir>,
}

impl<S, B> tower_service::Service<http::Request<B>> for WebUiService<S>
where
    S: tower_service::Service<http::Request<B>, Response = http::Response<BoxBody>>
        + Clone
        + Send
        + 'static,
    S::Error: Send,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = http::Response<BoxBody>;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<B>) -> Self::Future {
        // No UI path configured — pass everything through to gRPC
        if self.serve_dir.is_none() {
            return Box::pin(self.inner.call(req));
        }

        // Detect gRPC/gRPC-web requests by content-type header
        let is_grpc = req
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.starts_with("application/grpc"));

        // Also route by gRPC path prefix (package.Service/Method)
        let is_grpc_path = req.uri().path().starts_with("/daq.")
            || req.uri().path().starts_with("/protocol.")
            || req.uri().path().starts_with("/grpc.");

        if is_grpc || is_grpc_path {
            Box::pin(self.inner.call(req))
        } else {
            let mut files = self.serve_dir.clone().unwrap();
            Box::pin(async move {
                match files.call(req).await {
                    Ok(response) => {
                        // Map ServeDir's io::Error body to tonic's Status body
                        let (parts, body) = response.into_parts();
                        Ok(http::Response::from_parts(
                            parts,
                            BoxBody::new(IoToStatusBody(body)),
                        ))
                    }
                    Err(_io_err) => {
                        // ServeDir failed (e.g. permission denied) — return 500
                        Ok(http::Response::builder()
                            .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                            .body(BoxBody::default())
                            .unwrap())
                    }
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_image_measurement(source: &str, frame_number: Option<u64>) -> Measurement {
        Measurement::Image {
            name: source.to_string(),
            width: 2,
            height: 2,
            buffer: common::core::PixelBuffer::U16(vec![1, 2, 3, 4]),
            unit: "counts".to_string(),
            metadata: common::core::ImageMetadata {
                frame_number,
                ..Default::default()
            },
            timestamp: Utc::now(),
        }
    }

    /// Create a test DaqServer with a mock RunEngine (bd-si2c)
    #[cfg(feature = "scripting")]
    fn create_test_server() -> DaqServer {
        let registry = std::sync::Arc::new(hardware::registry::DeviceRegistry::new());
        let run_engine = std::sync::Arc::new(experiment::RunEngine::new(registry));
        let token = CancellationToken::new();
        #[cfg(feature = "storage_hdf5")]
        {
            DaqServer::new(None, run_engine, token).expect("failed to create test DaqServer")
        }
        #[cfg(not(feature = "storage_hdf5"))]
        {
            DaqServer::new(run_engine, token).expect("failed to create test DaqServer")
        }
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
                    value: f64::from(i),
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
                    value: f64::from(i),
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
                    value: f64::from(i),
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
            "Rate limiting not working: took {elapsed:?}"
        );
        assert!(
            elapsed.as_millis() < 700,
            "Rate limiting too slow: took {elapsed:?}"
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
                    value: f64::from(i),
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

    #[tokio::test]
    #[cfg(feature = "scripting")]
    async fn test_stream_spectra_filters_by_device_id_metadata() {
        use tokio_stream::StreamExt;

        let server = create_test_server();
        let data_sender = server.data_sender();
        let request = Request::new(crate::grpc::proto::SpectrumStreamRequest {
            channels: vec!["camera-a".to_string()],
            max_rate_hz: 0,
        });

        let response = server.stream_spectra(request).await.unwrap();
        let mut stream = response.into_inner();

        tokio::spawn(async move {
            let _ = data_sender.send(Measurement::Spectrum {
                name: "echelle_order_0".to_string(),
                frequencies: vec![500.0, 501.0],
                amplitudes: vec![1.0, 2.0],
                frequency_unit: Some("nm".to_string()),
                amplitude_unit: Some("counts".to_string()),
                metadata: Some(serde_json::json!({
                    "device_id": "camera-a",
                    "relative_index": 0
                })),
                timestamp: Utc::now(),
            });
        });

        let payload = stream
            .next()
            .await
            .expect("expected a streamed spectrum")
            .expect("expected streamed spectrum payload");

        assert_eq!(payload.name, "echelle_order_0");
        assert_eq!(payload.device_id, "camera-a");
        assert_eq!(payload.order_index, 0);
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

    #[test]
    fn test_build_image_measurement_preserves_frame_metadata() {
        let frame = common::data::Frame::from_u16(4, 2, &[1, 2, 3, 4, 5, 6, 7, 8])
            .with_frame_number(42)
            .with_exposure(12.5)
            .with_roi_offset(8, 16)
            .with_metadata(common::data::FrameMetadata {
                temperature_c: Some(-15.0),
                binning: Some((2, 4)),
                ..Default::default()
            });

        let measurement = build_image_measurement("camera-a", &frame);
        let Measurement::Image {
            name,
            width,
            height,
            buffer,
            metadata,
            ..
        } = measurement
        else {
            panic!("expected image measurement");
        };

        assert_eq!(name, "camera-a");
        assert_eq!((width, height), (4, 2));
        assert!(matches!(buffer, common::core::PixelBuffer::U16(_)));
        assert_eq!(metadata.exposure_ms, Some(12.5));
        assert_eq!(metadata.temperature_c, Some(-15.0));
        assert_eq!(metadata.binning, Some((2, 4)));
        assert_eq!(metadata.roi_origin, Some((8, 16)));
        assert_eq!(metadata.frame_number, Some(42));
        assert_eq!(metadata.sequence_gap_from_previous, None);
    }

    #[test]
    fn test_spectrum_payload_from_parts_defaults_without_metadata() {
        let payload = spectrum_payload_from_parts(
            "spectrum-a".to_string(),
            vec![500.0, 501.0],
            vec![1.0, 2.0],
            Some("nm".to_string()),
            Some("counts".to_string()),
            None,
            123,
        );

        assert_eq!(payload.name, "spectrum-a");
        assert_eq!(payload.wavelengths, vec![500.0, 501.0]);
        assert_eq!(payload.intensities, vec![1.0, 2.0]);
        assert_eq!(payload.wavelength_unit, "nm");
        assert_eq!(payload.intensity_unit, "counts");
        assert_eq!(payload.order_index, 0);
        assert!(!payload.merged);
        assert!(payload.ivar.is_empty());
        assert!(payload.quality_flags.is_empty());
        assert_eq!(payload.timestamp_ns, 123);
        assert!(payload.metadata_json.is_empty());
    }

    #[test]
    fn test_spectrum_payload_from_parts_parses_echelle_metadata() {
        let payload = spectrum_payload_from_parts(
            "echelle_merged_preview".to_string(),
            vec![500.0, 501.0],
            vec![1.0, 2.0],
            Some("nm".to_string()),
            Some("counts".to_string()),
            Some(serde_json::json!({
                "kind": "echelle_merged",
                "profile_id": "profile-42",
                "quality_flags": ["cosmic_masked", "blaze_corrected"],
                "snr_estimate": 18.5,
                "ivar": [0.25, 0.5]
            })),
            456,
        );

        assert!(payload.merged);
        assert_eq!(payload.order_index, -1);
        assert_eq!(payload.calibration_profile_id, "profile-42");
        assert_eq!(
            payload.quality_flags,
            vec!["cosmic_masked".to_string(), "blaze_corrected".to_string()]
        );
        assert_eq!(payload.snr_estimate, 18.5);
        assert_eq!(payload.ivar, vec![0.25, 0.5]);
        assert!(
            payload
                .metadata_json
                .contains("\"profile_id\":\"profile-42\"")
        );
    }

    #[test]
    fn test_annotate_measurement_data_integrity_marks_gap() {
        let mut last_frame_numbers = HashMap::new();
        let mut first = test_image_measurement("camera-a", Some(7));
        let mut second = test_image_measurement("camera-a", Some(10));

        assert!(annotate_measurement_data_integrity(&mut first, &mut last_frame_numbers).is_none());

        let (source, fault) =
            annotate_measurement_data_integrity(&mut second, &mut last_frame_numbers)
                .expect("expected sequence gap fault");
        let Measurement::Image { metadata, .. } = second else {
            panic!("expected image measurement");
        };

        assert_eq!(source, "camera-a");
        assert_eq!(
            fault,
            ImageSequenceFault {
                previous_frame_number: 7,
                current_frame_number: 10,
                missing_frames: 2,
            }
        );
        assert_eq!(metadata.sequence_gap_from_previous, Some(2));
        assert_eq!(last_frame_numbers.get("camera-a"), Some(&10));
    }

    #[test]
    fn test_annotate_measurement_data_integrity_ignores_regressions() {
        let mut last_frame_numbers = HashMap::from([("camera-a".to_string(), 10)]);
        let mut measurement = test_image_measurement("camera-a", Some(9));

        assert!(
            annotate_measurement_data_integrity(&mut measurement, &mut last_frame_numbers)
                .is_none()
        );
        let Measurement::Image { metadata, .. } = measurement else {
            panic!("expected image measurement");
        };
        assert_eq!(metadata.sequence_gap_from_previous, None);
        assert_eq!(last_frame_numbers.get("camera-a"), Some(&10));
    }
}
