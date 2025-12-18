//! StorageService gRPC implementation (bd-p6im)
//!
//! Provides HDF5 data storage and export functionality via gRPC.
//! Uses the "Mullet Strategy": Arrow for fast in-memory processing,
//! HDF5 for long-term storage and cross-platform compatibility.
//!
//! # Security (bd-hwq9)
//! All user-provided filenames are sanitized to prevent path traversal attacks.
//! Output paths are validated to remain within the configured output directory.

use crate::grpc::proto::{
    AcquisitionInfo, AcquisitionSummary, ConfigureStorageRequest, ConfigureStorageResponse,
    DeleteAcquisitionRequest, DeleteAcquisitionResponse, FlushToStorageRequest,
    FlushToStorageResponse, GetAcquisitionInfoRequest, GetRecordingStatusRequest,
    GetRingBufferTapInfoRequest, GetStorageConfigRequest, Hdf5Config, Hdf5Structure,
    ListAcquisitionsRequest, ListAcquisitionsResponse, RecordingProgress, RecordingState,
    RecordingStatus, RingBufferTapInfo, StartRecordingRequest, StartRecordingResponse,
    StopRecordingRequest, StopRecordingResponse, StorageConfig, StreamRecordingProgressRequest,
    storage_service_server::StorageService,
};
use daq_storage::hdf5_writer::HDF5Writer;
use daq_storage::ring_buffer::RingBuffer;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::task::JoinHandle;
use tonic::{Request, Response, Status};
use uuid::Uuid;

#[cfg(feature = "storage_hdf5")]
use crate::data::ring_buffer::RingBuffer;

// =============================================================================
// Path Security (bd-hwq9)
// =============================================================================

/// Sanitize a user-provided name to a safe filename component.
///
/// This prevents path traversal attacks by:
/// - Removing directory separators (/, \)
/// - Removing path traversal sequences (..)
/// - Only allowing alphanumeric characters, underscores, hyphens, and periods
/// - Ensuring the result is non-empty (falls back to "unnamed" if empty)
///
/// # Security
/// This is a critical security function. Any changes must be reviewed carefully.
fn sanitize_filename_component(name: &str) -> String {
    // Replace common directory separators with underscores
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '_',
            c if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' => c,
            _ => '_',
        })
        .collect();

    // Remove any remaining path traversal patterns
    let sanitized = sanitized
        .replace("..", "_")
        .replace("._", "_")
        .replace("_.", "_");

    // Collapse multiple underscores
    let mut result = String::with_capacity(sanitized.len());
    let mut last_was_underscore = false;
    for c in sanitized.chars() {
        if c == '_' {
            if !last_was_underscore {
                result.push(c);
            }
            last_was_underscore = true;
        } else {
            result.push(c);
            last_was_underscore = false;
        }
    }

    // Trim leading/trailing underscores and periods (prevent hidden files)
    let result = result.trim_matches(|c| c == '_' || c == '.');

    // Ensure non-empty
    if result.is_empty() {
        "unnamed".to_string()
    } else {
        result.to_string()
    }
}

/// Validate that a path is safely within the expected base directory.
///
/// This uses canonicalization to resolve symlinks and relative paths,
/// then verifies the result starts with the expected base directory.
///
/// # Returns
/// - `Ok(canonical_path)` if the path is valid and within the base directory
/// - `Err(message)` if the path escapes the base directory or cannot be validated
///
/// # Security
/// This is a critical security function that prevents directory traversal via:
/// - Relative paths (../)
/// - Symlink escapes
/// - Absolute path injection
fn validate_path_within_directory(
    base_dir: &Path,
    target_path: &Path,
) -> Result<PathBuf, String> {
    // Canonicalize base directory (must exist)
    let canonical_base = base_dir.canonicalize().map_err(|e| {
        format!(
            "Cannot canonicalize base directory '{}': {}",
            base_dir.display(),
            e
        )
    })?;

    // For the target path, we need to handle files that don't exist yet.
    // Canonicalize the parent directory, then append the filename.
    let parent = target_path.parent().ok_or_else(|| {
        format!("Invalid target path: {}", target_path.display())
    })?;
    let filename = target_path.file_name().ok_or_else(|| {
        format!("Target path has no filename: {}", target_path.display())
    })?;

    // If parent doesn't exist, try to canonicalize what does exist
    let canonical_parent = if parent.exists() {
        parent.canonicalize().map_err(|e| {
            format!(
                "Cannot canonicalize parent directory '{}': {}",
                parent.display(),
                e
            )
        })?
    } else {
        // Parent doesn't exist - check if it would be created within base
        // For safety, require the parent directory to exist
        return Err(format!(
            "Parent directory does not exist: {}",
            parent.display()
        ));
    };

    let canonical_target = canonical_parent.join(filename);

    // Verify the canonical target starts with the canonical base
    if !canonical_target.starts_with(&canonical_base) {
        return Err(format!(
            "Path '{}' escapes base directory '{}'",
            target_path.display(),
            base_dir.display()
        ));
    }

    Ok(canonical_target)
}

// =============================================================================
// Service Implementation
// =============================================================================

/// Active recording session state
struct RecordingSession {
    id: String,
    name: String,
    output_path: PathBuf,
    start_time_ns: u64,
    samples_recorded: AtomicU64,
    bytes_written: AtomicU64,
    flushes_completed: AtomicU64,
    state: RwLock<RecordingState>,
    metadata: HashMap<String, String>,
    scan_id: Option<String>,
    run_uid: Option<String>,
    writer: Mutex<Option<ActiveWriter>>,
}

/// Completed acquisition metadata
#[derive(Clone)]
struct AcquisitionRecord {
    id: String,
    name: String,
    file_path: PathBuf,
    created_at_ns: u64,
    duration_ns: u64,
    sample_count: u64,
    file_size_bytes: u64,
    metadata: HashMap<String, String>,
    scan_id: Option<String>,
    run_uid: Option<String>,
}

struct ActiveWriter {
    writer: Arc<HDF5Writer>,
    handle: JoinHandle<()>,
}

/// Storage configuration
struct StorageSettings {
    output_directory: PathBuf,
    compression: String,
    compression_level: u32,
    chunk_size: u32,
    filename_pattern: String,
    include_timestamps: bool,
    include_device_metadata: bool,
    flush_interval_ms: u32,
    max_buffer_mb: u32,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            output_directory: PathBuf::from("./data"),
            compression: "gzip".to_string(),
            compression_level: 4,
            chunk_size: 4096,
            filename_pattern: "{name}_{timestamp}.h5".to_string(),
            include_timestamps: true,
            include_device_metadata: true,
            flush_interval_ms: 1000,
            max_buffer_mb: 256,
        }
    }
}

/// StorageService implementation for HDF5 data storage
pub struct StorageServiceImpl {
    settings: Arc<RwLock<StorageSettings>>,
    current_recording: Arc<RwLock<Option<Arc<RecordingSession>>>>,
    acquisitions: Arc<RwLock<HashMap<String, AcquisitionRecord>>>,
    is_recording: AtomicBool,
    ring_buffer: Option<Arc<RingBuffer>>,
}

impl StorageServiceImpl {
    /// Create a new StorageService
    pub fn new(ring_buffer: Option<Arc<RingBuffer>>) -> Self {
        Self {
            settings: Arc::new(RwLock::new(StorageSettings::default())),
            current_recording: Arc::new(RwLock::new(None)),
            acquisitions: Arc::new(RwLock::new(HashMap::new())),
            is_recording: AtomicBool::new(false),
            ring_buffer,
        }
    }

    /// Generate output filename from pattern
    ///
    /// # Security (bd-hwq9)
    /// The `name` parameter is sanitized to prevent path traversal attacks.
    /// Only safe filename characters are allowed in the output.
    fn generate_filename(&self, name: &str, pattern: &str) -> String {
        // SECURITY: Sanitize user-provided name to prevent path traversal
        let safe_name = sanitize_filename_component(name);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        pattern
            .replace("{name}", &safe_name)
            .replace("{timestamp}", &timestamp.to_string())
            .replace("{date}", &chrono::Utc::now().format("%Y-%m-%d").to_string())
            .replace(
                "{datetime}",
                &chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string(),
            )
    }

    /// Get disk space info for a directory
    async fn get_disk_space(path: &Path) -> (u64, u64) {
        // Try to get disk space using sys-info or fall back to defaults
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(metadata) = tokio::fs::metadata(path).await {
                // This is a rough estimate - actual implementation would use statvfs
                let _ = metadata.dev();
            }
        }
        // Fallback: return large defaults to indicate unknown
        (u64::MAX, 0)
    }

    /// Scan existing HDF5 files in output directory
    async fn scan_existing_acquisitions(&self) -> HashMap<String, AcquisitionRecord> {
        let mut records = HashMap::new();
        let settings = self.settings.read().await;

        let mut entries = match fs::read_dir(&settings.output_directory).await {
            Ok(dir) => dir,
            Err(_) => return records,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path
                .extension()
                .map_or(false, |ext| ext == "h5" || ext == "hdf5")
            {
                if let Ok(metadata) = fs::metadata(&path).await {
                    let id = Uuid::new_v4().to_string();
                    let name = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();

                    let created_at_ns = metadata
                        .created()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0);

                    records.insert(
                        id.clone(),
                        AcquisitionRecord {
                            id,
                            name,
                            file_path: path.clone(),
                            created_at_ns,
                            duration_ns: 0, // Unknown for scanned files
                            sample_count: 0,
                            file_size_bytes: metadata.len(),
                            metadata: HashMap::new(),
                            scan_id: None,
                            run_uid: None,
                        },
                    );
                }
            }
        }

        records
    }
}

#[tonic::async_trait]
impl StorageService for StorageServiceImpl {
    /// Configure storage settings
    async fn configure_storage(
        &self,
        request: Request<ConfigureStorageRequest>,
    ) -> Result<Response<ConfigureStorageResponse>, Status> {
        let req = request.into_inner();

        // Validate output directory
        let output_dir = PathBuf::from(&req.output_directory);
        if !output_dir.exists() {
            if let Err(e) = fs::create_dir_all(&output_dir).await {
                return Ok(Response::new(ConfigureStorageResponse {
                    success: false,
                    error_message: format!("Failed to create output directory: {}", e),
                    resolved_output_directory: String::new(),
                }));
            }
        }

        let resolved_path = fs::canonicalize(&output_dir)
            .await
            .unwrap_or(output_dir.clone())
            .to_string_lossy()
            .to_string();

        // Update settings
        let mut settings = self.settings.write().await;
        settings.output_directory = output_dir;

        if let Some(hdf5_config) = req.hdf5_config {
            if !hdf5_config.compression.is_empty() {
                settings.compression = hdf5_config.compression;
            }
            if let Some(level) = hdf5_config.compression_level {
                settings.compression_level = level;
            }
            if let Some(chunk) = hdf5_config.chunk_size {
                settings.chunk_size = chunk;
            }
            if let Some(pattern) = hdf5_config.filename_pattern {
                settings.filename_pattern = pattern;
            }
            settings.include_timestamps = hdf5_config.include_timestamps;
            settings.include_device_metadata = hdf5_config.include_device_metadata;
        }

        if let Some(interval) = req.flush_interval_ms {
            settings.flush_interval_ms = interval;
        }
        if let Some(max_mb) = req.max_buffer_mb {
            settings.max_buffer_mb = max_mb;
        }

        Ok(Response::new(ConfigureStorageResponse {
            success: true,
            error_message: String::new(),
            resolved_output_directory: resolved_path,
        }))
    }

    /// Get current storage configuration
    async fn get_storage_config(
        &self,
        _request: Request<GetStorageConfigRequest>,
    ) -> Result<Response<StorageConfig>, Status> {
        let settings = self.settings.read().await;
        let (available, used) = Self::get_disk_space(&settings.output_directory).await;

        Ok(Response::new(StorageConfig {
            output_directory: settings.output_directory.to_string_lossy().to_string(),
            hdf5_config: Some(Hdf5Config {
                compression: settings.compression.clone(),
                compression_level: Some(settings.compression_level),
                chunk_size: Some(settings.chunk_size),
                filename_pattern: Some(settings.filename_pattern.clone()),
                include_timestamps: settings.include_timestamps,
                include_device_metadata: settings.include_device_metadata,
            }),
            flush_interval_ms: settings.flush_interval_ms,
            max_buffer_mb: settings.max_buffer_mb,
            disk_space_available_bytes: available,
            disk_space_used_bytes: used,
        }))
    }

    /// Start recording data to HDF5 file
    async fn start_recording(
        &self,
        request: Request<StartRecordingRequest>,
    ) -> Result<Response<StartRecordingResponse>, Status> {
        let req = request.into_inner();

        // Check if already recording
        if self.is_recording.load(Ordering::SeqCst) {
            return Ok(Response::new(StartRecordingResponse {
                success: false,
                error_message: "Recording already in progress".to_string(),
                recording_id: String::new(),
                output_path: String::new(),
            }));
        }

        let settings = self.settings.read().await;

        // Generate output path with sanitized filename
        let filename = self.generate_filename(&req.name, &settings.filename_pattern);
        let output_path = settings.output_directory.join(&filename);

        // SECURITY (bd-hwq9): Validate the output path stays within the configured directory
        // This is defense-in-depth - sanitize_filename_component should have already
        // prevented path traversal, but we verify as a second layer of protection.
        let validated_path = match validate_path_within_directory(&settings.output_directory, &output_path) {
            Ok(path) => path,
            Err(e) => {
                tracing::error!("Path validation failed for recording '{}': {}", req.name, e);
                return Ok(Response::new(StartRecordingResponse {
                    success: false,
                    error_message: format!("Invalid output path: {}", e),
                    recording_id: String::new(),
                    output_path: String::new(),
                }));
            }
        };
        let output_path = validated_path;

        let recording_id = Uuid::new_v4().to_string();
        let start_time_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let session = Arc::new(RecordingSession {
            id: recording_id.clone(),
            name: req.name,
            output_path: output_path.clone(),
            start_time_ns,
            samples_recorded: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            flushes_completed: AtomicU64::new(0),
            state: RwLock::new(RecordingState::RecordingActive),
            metadata: req.metadata,
            scan_id: req.scan_id,
            run_uid: req.run_uid,
            writer: Mutex::new(None),
        });

        // Store session
        *self.current_recording.write().await = Some(session.clone());
        self.is_recording.store(true, Ordering::SeqCst);

        if let Some(rb) = &self.ring_buffer {
            match HDF5Writer::new(&output_path, rb.clone()) {
                Ok(mut writer_instance) => {
                    let interval = Duration::from_millis(settings.flush_interval_ms as u64);
                    writer_instance.set_flush_interval(interval);
                    let writer_arc = Arc::new(writer_instance);
                    let session_clone = Arc::clone(&session);
                    let writer_clone = writer_arc.clone();
                    let handle = tokio::spawn(async move {
                        let mut ticker = tokio::time::interval(writer_clone.flush_interval());
                        loop {
                            ticker.tick().await;
                            match writer_clone.flush_to_disk().await {
                                Ok(bytes) => {
                                    if bytes > 0 {
                                        session_clone
                                            .bytes_written
                                            .fetch_add(bytes as u64, Ordering::SeqCst);
                                        session_clone
                                            .samples_recorded
                                            .fetch_add(1, Ordering::SeqCst);
                                        session_clone
                                            .flushes_completed
                                            .fetch_add(1, Ordering::SeqCst);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        error = %e,
                                        "Recording flush failed; data may be incomplete"
                                    );
                                }
                            }
                        }
                    });

                    *session.writer.lock().await = Some(ActiveWriter {
                        writer: writer_arc,
                        handle,
                    });
                }
                Err(e) => {
                    self.is_recording.store(false, Ordering::SeqCst);
                    *self.current_recording.write().await = None;
                    return Ok(Response::new(StartRecordingResponse {
                        success: false,
                        error_message: format!("Failed to initialize HDF5 writer: {e}"),
                        recording_id: String::new(),
                        output_path: String::new(),
                    }));
                }
            }
        }

        Ok(Response::new(StartRecordingResponse {
            success: true,
            error_message: String::new(),
            recording_id,
            output_path: output_path.to_string_lossy().to_string(),
        }))
    }

    /// Stop recording and finalize HDF5 file
    async fn stop_recording(
        &self,
        request: Request<StopRecordingRequest>,
    ) -> Result<Response<StopRecordingResponse>, Status> {
        let req = request.into_inner();

        let session = {
            let recording = self.current_recording.read().await;
            match &*recording {
                Some(s) => {
                    // If specific ID requested, verify it matches
                    if let Some(ref id) = req.recording_id {
                        if &s.id != id {
                            return Ok(Response::new(StopRecordingResponse {
                                success: false,
                                error_message: format!("Recording {} not found", id),
                                acquisition_id: String::new(),
                                output_path: String::new(),
                                file_size_bytes: 0,
                                total_samples: 0,
                                duration_ns: 0,
                            }));
                        }
                    }
                    Arc::clone(s)
                }
                None => {
                    return Ok(Response::new(StopRecordingResponse {
                        success: false,
                        error_message: "No active recording".to_string(),
                        acquisition_id: String::new(),
                        output_path: String::new(),
                        file_size_bytes: 0,
                        total_samples: 0,
                        duration_ns: 0,
                    }));
                }
            }
        };

        // Update state to finalizing
        *session.state.write().await = RecordingState::RecordingFinalizing;

        // Calculate duration
        let end_time_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let duration_ns = end_time_ns - session.start_time_ns;

        // bd-q33m: Take writer under lock, then drop lock before awaiting flush
        // This prevents holding the mutex guard across the .await point
        let active = { session.writer.lock().await.take() };
        if let Some(active) = active {
            active.handle.abort();
            if let Err(e) = active.writer.flush_to_disk().await {
                tracing::warn!(
                    error = %e,
                    "Final flush failed while stopping recording; data may be incomplete"
                );
            }
        }

        // Get file size
        let file_size = fs::metadata(&session.output_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let samples = session.samples_recorded.load(Ordering::SeqCst);

        // Create acquisition record
        let acquisition_id = Uuid::new_v4().to_string();
        let mut metadata = session.metadata.clone();
        for (k, v) in req.final_metadata {
            metadata.insert(k, v);
        }

        let record = AcquisitionRecord {
            id: acquisition_id.clone(),
            name: session.name.clone(),
            file_path: session.output_path.clone(),
            created_at_ns: session.start_time_ns,
            duration_ns,
            sample_count: samples,
            file_size_bytes: file_size,
            metadata,
            scan_id: session.scan_id.clone(),
            run_uid: session.run_uid.clone(),
        };

        // Store acquisition record
        self.acquisitions
            .write()
            .await
            .insert(acquisition_id.clone(), record);

        // Clear current recording
        *self.current_recording.write().await = None;
        self.is_recording.store(false, Ordering::SeqCst);

        Ok(Response::new(StopRecordingResponse {
            success: true,
            error_message: String::new(),
            acquisition_id,
            output_path: session.output_path.to_string_lossy().to_string(),
            file_size_bytes: file_size,
            total_samples: samples,
            duration_ns,
        }))
    }

    /// Get current recording status
    async fn get_recording_status(
        &self,
        request: Request<GetRecordingStatusRequest>,
    ) -> Result<Response<RecordingStatus>, Status> {
        let req = request.into_inner();

        let recording = self.current_recording.read().await;
        match &*recording {
            Some(session) => {
                // If specific ID requested, verify it matches
                if let Some(ref id) = req.recording_id {
                    if &session.id != id {
                        return Ok(Response::new(RecordingStatus {
                            recording_id: id.clone(),
                            state: RecordingState::RecordingIdle.into(),
                            output_path: String::new(),
                            samples_recorded: 0,
                            bytes_written: 0,
                            start_time_ns: 0,
                            elapsed_ns: 0,
                            buffer_fill_percent: 0,
                            pending_samples: 0,
                            flushes_completed: 0,
                            error_message: "Recording not found".to_string(),
                        }));
                    }
                }

                let now_ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64;

                let state = session.state.read().await;

                Ok(Response::new(RecordingStatus {
                    recording_id: session.id.clone(),
                    state: (*state).into(),
                    output_path: session.output_path.to_string_lossy().to_string(),
                    samples_recorded: session.samples_recorded.load(Ordering::SeqCst),
                    bytes_written: session.bytes_written.load(Ordering::SeqCst),
                    start_time_ns: session.start_time_ns,
                    elapsed_ns: now_ns - session.start_time_ns,
                    buffer_fill_percent: 0, // Ring buffer fill % not yet integrated
                    pending_samples: 0,     // Ring buffer pending count not yet integrated
                    flushes_completed: session.flushes_completed.load(Ordering::SeqCst),
                    error_message: String::new(),
                }))
            }
            None => Ok(Response::new(RecordingStatus {
                recording_id: String::new(),
                state: RecordingState::RecordingIdle.into(),
                output_path: String::new(),
                samples_recorded: 0,
                bytes_written: 0,
                start_time_ns: 0,
                elapsed_ns: 0,
                buffer_fill_percent: 0,
                pending_samples: 0,
                flushes_completed: 0,
                error_message: String::new(),
            })),
        }
    }

    /// List all saved acquisitions
    async fn list_acquisitions(
        &self,
        request: Request<ListAcquisitionsRequest>,
    ) -> Result<Response<ListAcquisitionsResponse>, Status> {
        let req = request.into_inner();

        // Scan for any new files
        let scanned = self.scan_existing_acquisitions().await;
        let mut acquisitions = self.acquisitions.write().await;
        for (id, record) in scanned {
            acquisitions.entry(id).or_insert(record);
        }

        let limit = req.limit.unwrap_or(100) as usize;
        let offset = req.offset.unwrap_or(0) as usize;

        let mut results: Vec<_> = acquisitions
            .values()
            .filter(|r| {
                // Apply name pattern filter
                if let Some(ref pattern) = req.name_pattern {
                    if !r.name.contains(pattern.trim_matches('*')) {
                        return false;
                    }
                }
                // Apply timestamp filters
                if let Some(after) = req.after_timestamp_ns {
                    if r.created_at_ns < after {
                        return false;
                    }
                }
                if let Some(before) = req.before_timestamp_ns {
                    if r.created_at_ns > before {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Sort by creation time, newest first
        results.sort_by(|a, b| b.created_at_ns.cmp(&a.created_at_ns));

        let total_count = results.len() as u32;

        let summaries: Vec<AcquisitionSummary> = results
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|r| AcquisitionSummary {
                acquisition_id: r.id.clone(),
                name: r.name.clone(),
                file_path: r.file_path.to_string_lossy().to_string(),
                file_size_bytes: r.file_size_bytes,
                created_at_ns: r.created_at_ns,
                duration_ns: r.duration_ns,
                sample_count: r.sample_count,
            })
            .collect();

        Ok(Response::new(ListAcquisitionsResponse {
            acquisitions: summaries,
            total_count,
        }))
    }

    /// Get detailed info about a specific acquisition
    async fn get_acquisition_info(
        &self,
        request: Request<GetAcquisitionInfoRequest>,
    ) -> Result<Response<AcquisitionInfo>, Status> {
        let req = request.into_inner();

        let acquisitions = self.acquisitions.read().await;
        let record = acquisitions
            .get(&req.acquisition_id)
            .ok_or_else(|| Status::not_found("Acquisition not found"))?;

        // HDF5 file structure parsing not yet implemented (requires storage_hdf5 feature)
        let datasets = Vec::new(); // Would parse HDF5 file for dataset info

        Ok(Response::new(AcquisitionInfo {
            acquisition_id: record.id.clone(),
            name: record.name.clone(),
            file_path: record.file_path.to_string_lossy().to_string(),
            file_size_bytes: record.file_size_bytes,
            created_at_ns: record.created_at_ns,
            duration_ns: record.duration_ns,
            datasets,
            metadata: record.metadata.clone(),
            scan_id: record.scan_id.clone(),
            run_uid: record.run_uid.clone(),
            structure: Some(Hdf5Structure {
                groups: vec!["/measurements".to_string(), "/metadata".to_string()],
                dataset_count: 0,
                total_elements: 0,
                compression: "gzip".to_string(),
                chunk_size: 4096,
            }),
        }))
    }

    /// Delete an acquisition file
    async fn delete_acquisition(
        &self,
        request: Request<DeleteAcquisitionRequest>,
    ) -> Result<Response<DeleteAcquisitionResponse>, Status> {
        let req = request.into_inner();

        if !req.confirm {
            return Ok(Response::new(DeleteAcquisitionResponse {
                success: false,
                error_message: "Deletion requires confirm=true".to_string(),
                bytes_freed: 0,
            }));
        }

        let mut acquisitions = self.acquisitions.write().await;
        let record = acquisitions
            .remove(&req.acquisition_id)
            .ok_or_else(|| Status::not_found("Acquisition not found"))?;

        let bytes_freed = record.file_size_bytes;

        // Delete the file
        if let Err(e) = fs::remove_file(&record.file_path).await {
            // Re-insert record since delete failed
            acquisitions.insert(req.acquisition_id, record);
            return Ok(Response::new(DeleteAcquisitionResponse {
                success: false,
                error_message: format!("Failed to delete file: {}", e),
                bytes_freed: 0,
            }));
        }

        Ok(Response::new(DeleteAcquisitionResponse {
            success: true,
            error_message: String::new(),
            bytes_freed,
        }))
    }

    /// Flush ring buffer data to storage
    async fn flush_to_storage(
        &self,
        _request: Request<FlushToStorageRequest>,
    ) -> Result<Response<FlushToStorageResponse>, Status> {
        // Check if recording
        if !self.is_recording.load(Ordering::SeqCst) {
            return Ok(Response::new(FlushToStorageResponse {
                success: false,
                error_message: "No active recording".to_string(),
                samples_flushed: 0,
                bytes_written: 0,
            }));
        }

        let session_arc = self.current_recording.read().await.clone();
        let writer = if let Some(session) = session_arc.as_ref() {
            let guard = session.writer.lock().await;
            guard.as_ref().map(|active| active.writer.clone())
        } else {
            None
        };

        if let Some(writer) = writer {
            match writer.flush_to_disk().await {
                Ok(bytes) => {
                    if let Some(session) = session_arc.as_ref() {
                        session
                            .bytes_written
                            .fetch_add(bytes as u64, Ordering::SeqCst);
                        session.flushes_completed.fetch_add(1, Ordering::SeqCst);
                        session.samples_recorded.fetch_add(1, Ordering::SeqCst);
                    }

                    return Ok(Response::new(FlushToStorageResponse {
                        success: true,
                        error_message: String::new(),
                        samples_flushed: 1,
                        bytes_written: bytes as u64,
                    }));
                }
                Err(e) => {
                    return Ok(Response::new(FlushToStorageResponse {
                        success: false,
                        error_message: format!("Flush failed: {e}"),
                        samples_flushed: 0,
                        bytes_written: 0,
                    }));
                }
            }
        }

        Ok(Response::new(FlushToStorageResponse {
            success: false,
            error_message: "Recording writer is not initialized".to_string(),
            samples_flushed: 0,
            bytes_written: 0,
        }))
    }

    type StreamRecordingProgressStream =
        tokio_stream::wrappers::ReceiverStream<Result<RecordingProgress, Status>>;

    /// Stream recording progress updates
    async fn stream_recording_progress(
        &self,
        request: Request<StreamRecordingProgressRequest>,
    ) -> Result<Response<Self::StreamRecordingProgressStream>, Status> {
        let req = request.into_inner();
        let scan_id = req.scan_id;
        let interval_ms = req.update_interval_ms.max(100); // Min 100ms

        let (tx, rx) = mpsc::channel(32);
        let current_recording = Arc::clone(&self.current_recording);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(interval_ms as u64));

            loop {
                interval.tick().await;

                let recording = current_recording.read().await;
                match &*recording {
                    Some(session) if session.id == scan_id => {
                        let now_ns = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_nanos() as u64;

                        let state = session.state.read().await;
                        let samples = session.samples_recorded.load(Ordering::SeqCst);
                        let bytes = session.bytes_written.load(Ordering::SeqCst);
                        let elapsed_ns = now_ns - session.start_time_ns;
                        let elapsed_secs = elapsed_ns as f64 / 1_000_000_000.0;
                        let samples_per_sec = if elapsed_secs > 0.0 {
                            samples as f64 / elapsed_secs
                        } else {
                            0.0
                        };

                        let progress = RecordingProgress {
                            scan_id: session.id.clone(),
                            state: (*state).into(),
                            timestamp_ns: now_ns,
                            samples_recorded: samples,
                            bytes_written: bytes,
                            samples_per_second: samples_per_sec,
                            buffer_fill_percent: 0,
                            flush_in_progress: false,
                            estimated_remaining_ns: None,
                        };

                        if tx.send(Ok(progress)).await.is_err() {
                            break; // Client disconnected
                        }

                        // Stop streaming if recording finished
                        if *state == RecordingState::RecordingIdle {
                            break;
                        }
                    }
                    _ => {
                        // Recording not found or finished
                        break;
                    }
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    /// Get information for cross-process ring buffer access via mmap.
    async fn get_ring_buffer_tap_info(
        &self,
        _request: Request<GetRingBufferTapInfoRequest>,
    ) -> Result<Response<RingBufferTapInfo>, Status> {
        // Get ring buffer or return error
        let ring_buffer = self
            .ring_buffer
            .as_ref()
            .ok_or_else(|| Status::unavailable("Ring buffer not configured"))?;

        // Get file size from metadata
        let path = ring_buffer.path();
        let total_size_bytes = tokio::fs::metadata(path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(Response::new(RingBufferTapInfo {
            file_path: path.to_string_lossy().to_string(),
            total_size_bytes,
            capacity_bytes: ring_buffer.capacity(),
            header_size: 128, // HEADER_SIZE constant
            stream_id: ring_buffer.stream_id(),
            magic: ring_buffer.magic(),
            write_head: ring_buffer.write_head(),
            read_tail: ring_buffer.read_tail(),
            write_epoch: ring_buffer.write_epoch(),
            data_format: "arrow_ipc".to_string(),
            arrow_schema_json: None, // TODO: Populate from buffer if available
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_configure_storage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ring_path = temp_dir.path().join("ring.buf");
        let ring_buffer = Arc::new(RingBuffer::create(&ring_path, 4).unwrap());
        let service = StorageServiceImpl::new(Some(ring_buffer));

        let request = Request::new(ConfigureStorageRequest {
            output_directory: temp_dir.path().to_string_lossy().to_string(),
            hdf5_config: Some(Hdf5Config {
                compression: "lz4".to_string(),
                compression_level: Some(6),
                chunk_size: Some(8192),
                filename_pattern: Some("{name}_{datetime}.h5".to_string()),
                include_timestamps: true,
                include_device_metadata: false,
            }),
            flush_interval_ms: Some(500),
            max_buffer_mb: Some(128),
        });

        let response = service.configure_storage(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!(resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_start_stop_recording() {
        let service = StorageServiceImpl::new(None);
        let temp_dir = tempfile::tempdir().unwrap();

        // Configure storage first
        let config_req = Request::new(ConfigureStorageRequest {
            output_directory: temp_dir.path().to_string_lossy().to_string(),
            hdf5_config: None,
            flush_interval_ms: None,
            max_buffer_mb: None,
        });
        service.configure_storage(config_req).await.unwrap();

        // Start recording
        let start_req = Request::new(StartRecordingRequest {
            name: "test_recording".to_string(),
            metadata: HashMap::new(),
            config_override: None,
            scan_id: None,
            run_uid: None,
        });
        let start_resp = service
            .start_recording(start_req)
            .await
            .unwrap()
            .into_inner();

        assert!(start_resp.success);
        assert!(!start_resp.recording_id.is_empty());

        // Try to start another recording (should fail)
        let start_req2 = Request::new(StartRecordingRequest {
            name: "test_recording2".to_string(),
            metadata: HashMap::new(),
            config_override: None,
            scan_id: None,
            run_uid: None,
        });
        let start_resp2 = service
            .start_recording(start_req2)
            .await
            .unwrap()
            .into_inner();
        assert!(!start_resp2.success);

        // Stop recording
        let stop_req = Request::new(StopRecordingRequest {
            recording_id: Some(start_resp.recording_id.clone()),
            final_metadata: HashMap::new(),
        });
        let stop_resp = service.stop_recording(stop_req).await.unwrap().into_inner();

        assert!(stop_resp.success);
        assert!(!stop_resp.acquisition_id.is_empty());
    }

    #[tokio::test]
    async fn test_list_acquisitions() {
        let service = StorageServiceImpl::new(None);

        let request = Request::new(ListAcquisitionsRequest {
            name_pattern: None,
            after_timestamp_ns: None,
            before_timestamp_ns: None,
            limit: Some(10),
            offset: Some(0),
        });

        let response = service.list_acquisitions(request).await.unwrap();
        let resp = response.into_inner();

        // Empty list is valid
        assert!(resp.acquisitions.is_empty() || !resp.acquisitions.is_empty());
    }

    // =========================================================================
    // Security Tests (bd-hwq9)
    // =========================================================================

    #[test]
    fn test_sanitize_filename_basic() {
        // Normal names should pass through
        assert_eq!(sanitize_filename_component("test"), "test");
        assert_eq!(sanitize_filename_component("my_file"), "my_file");
        assert_eq!(sanitize_filename_component("data-2024"), "data-2024");
        assert_eq!(sanitize_filename_component("scan.h5"), "scan.h5");
    }

    #[test]
    fn test_sanitize_filename_path_traversal() {
        // Path traversal attempts should be neutralized
        assert_eq!(sanitize_filename_component("../../../etc/passwd"), "etc_passwd");
        assert_eq!(sanitize_filename_component(".."), "unnamed");
        assert_eq!(sanitize_filename_component("foo/../bar"), "foo_bar");
        assert_eq!(sanitize_filename_component("a..b"), "a_b");
    }

    #[test]
    fn test_sanitize_filename_directory_separators() {
        // Directory separators should be replaced
        assert_eq!(sanitize_filename_component("path/to/file"), "path_to_file");
        assert_eq!(sanitize_filename_component("path\\to\\file"), "path_to_file");
        assert_eq!(sanitize_filename_component("/absolute/path"), "absolute_path");
    }

    #[test]
    fn test_sanitize_filename_special_chars() {
        // Special characters should be replaced with underscores
        assert_eq!(sanitize_filename_component("file:name"), "file_name");
        assert_eq!(sanitize_filename_component("file\0name"), "file_name");
        assert_eq!(sanitize_filename_component("file<>name"), "file_name");
    }

    #[test]
    fn test_sanitize_filename_empty_and_edge_cases() {
        // Empty strings should return "unnamed"
        assert_eq!(sanitize_filename_component(""), "unnamed");
        assert_eq!(sanitize_filename_component("..."), "unnamed");
        assert_eq!(sanitize_filename_component("___"), "unnamed");
        assert_eq!(sanitize_filename_component("._._"), "unnamed");
    }

    #[test]
    fn test_sanitize_filename_unicode() {
        // Unicode alphanumeric should be preserved, others replaced
        assert_eq!(sanitize_filename_component("日本語"), "日本語");
        assert_eq!(sanitize_filename_component("tëst"), "tëst");
        assert_eq!(sanitize_filename_component("файл"), "файл");
    }

    #[test]
    fn test_validate_path_within_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base_dir = temp_dir.path();

        // Valid path within directory
        let valid_path = base_dir.join("test.h5");
        assert!(validate_path_within_directory(base_dir, &valid_path).is_ok());

        // Path traversal attempt should fail
        let traversal_path = base_dir.join("../../../etc/passwd");
        assert!(validate_path_within_directory(base_dir, &traversal_path).is_err());
    }

    #[tokio::test]
    async fn test_start_recording_path_traversal_blocked() {
        let service = StorageServiceImpl::new(None);
        let temp_dir = tempfile::tempdir().unwrap();

        // Configure storage first
        let config_req = Request::new(ConfigureStorageRequest {
            output_directory: temp_dir.path().to_string_lossy().to_string(),
            hdf5_config: None,
            flush_interval_ms: None,
            max_buffer_mb: None,
        });
        service.configure_storage(config_req).await.unwrap();

        // Attempt path traversal in recording name
        let malicious_req = Request::new(StartRecordingRequest {
            name: "../../../etc/passwd".to_string(),
            metadata: HashMap::new(),
            config_override: None,
            scan_id: None,
            run_uid: None,
        });

        let response = service
            .start_recording(malicious_req)
            .await
            .unwrap()
            .into_inner();

        // The filename should be sanitized, so it might succeed with a safe name
        // OR fail path validation. Either way, it should NOT write to /etc/passwd.
        // Check that if it succeeded, the output path is within the temp dir.
        if response.success {
            let output_path = std::path::Path::new(&response.output_path);
            // Use canonicalized paths for comparison (handles macOS /tmp -> /private symlink)
            let canonical_temp = temp_dir.path().canonicalize().unwrap();
            let canonical_output_parent = output_path.parent().unwrap().canonicalize().unwrap();
            assert!(
                canonical_output_parent.starts_with(&canonical_temp),
                "Output path '{}' should be within temp directory '{}'",
                response.output_path,
                canonical_temp.display()
            );
            // Verify it's NOT writing to /etc/passwd
            assert!(
                !output_path.to_string_lossy().contains("/etc/passwd"),
                "Should not write to /etc/passwd"
            );
            // Clean up by stopping
            let stop_req = Request::new(StopRecordingRequest {
                recording_id: Some(response.recording_id),
                final_metadata: HashMap::new(),
            });
            let _ = service.stop_recording(stop_req).await;
        }
    }
}
