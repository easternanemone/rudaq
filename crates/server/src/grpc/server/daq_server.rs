//! `DaqServer` struct + all its impls, including the `ControlService`
//! gRPC trait implementation for script execution, measurement streaming,
//! and daemon introspection.
//!
//! Extracted from `server/mod.rs` (C1 step 4a). The struct itself is
//! re-exported from the parent as `pub use daq_server::DaqServer;` so
//! the crate-public path `crate::grpc::server::DaqServer` is preserved.
//! Private helper types (`ScriptMetadata`, `ExecutionState`) are scoped
//! to this module.

use super::*;

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

// Auth, TLS, and CORS helpers (`JwtClaims`, `build_tls_config`, `build_cors_layer`,
// `validate_auth`, `extract_bearer_token`) moved to the `auth` sibling module
// (C1 step 2). They are re-imported above and remain callable by the
// `build_grpc_server!` macro and the test module.

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
    /// let run_engine = Arc::new(RunEngine::new(registry));
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

// Measurement payload + image-pipeline helpers moved to `measurement_pipeline`
// sibling module (C1 step 2).

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
    #[allow(clippy::cast_possible_truncation)]
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
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
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
            #[allow(clippy::cast_possible_truncation)]
            // SAFETY: Unix epoch nanos will not exceed u64::MAX until year 2554
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
                #[allow(clippy::cast_precision_loss)]
                // SAFETY: precision loss acceptable for metrics/display
                let used_memory_mb = used_memory_kb as f64 / 1024.0;

                // Determine engine state based on CPU activity
                // This is a simple heuristic: if CPU > 1%, we consider it RUNNING
                let current_state = if cpu_usage_percent > 1.0 {
                    "RUNNING".to_string()
                } else {
                    "IDLE".to_string()
                };

                #[allow(clippy::cast_possible_truncation)]
                // SAFETY: value is bounded and fits in target type
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
                        #[allow(clippy::cast_sign_loss)]
                        // SAFETY: value is non-negative at this point
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        (name.clone(), *value, ts_ns)
                    }
                    Measurement::Vector {
                        name,
                        values,
                        timestamp,
                        ..
                    } => {
                        #[allow(clippy::cast_sign_loss)]
                        // SAFETY: timestamp nanos are non-negative
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        #[allow(clippy::cast_precision_loss)]
                        // SAFETY: precision loss acceptable for metrics/display
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
                        #[allow(clippy::cast_sign_loss)]
                        // SAFETY: timestamp nanos are non-negative
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        (name.clone(), f64::from(width * height), ts_ns)
                    }
                    Measurement::Spectrum {
                        name,
                        amplitudes,
                        timestamp,
                        ..
                    } => {
                        #[allow(clippy::cast_sign_loss)]
                        // SAFETY: timestamp nanos are non-negative
                        let ts_ns = timestamp.timestamp_nanos_opt().unwrap_or(0) as u64;
                        #[allow(clippy::cast_precision_loss)]
                        // SAFETY: precision loss acceptable for metrics/display
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

                #[allow(clippy::cast_sign_loss)]
                // SAFETY: value is non-negative at this point
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
    #[allow(clippy::vec_init_then_push)] // conditional cfg-gated pushes can't use vec![] macro
    async fn get_daemon_info(
        &self,
        _request: Request<DaemonInfoRequest>,
    ) -> Result<Response<DaemonInfoResponse>, Status> {
        #[allow(unused_mut)] // features.push() only compiles under cfg'd features
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
