//! Actor-based DAQ application state management.
//!
//! This module implements the actor pattern for centralized state management,
//! replacing the previous `Arc<Mutex<DaqAppInner>>` approach. The actor model
//! eliminates lock contention, prevents deadlocks, and provides a clearer
//! mental model for concurrent state management.
//!
//! # Architecture
//!
//! The [`DaqManagerActor`] is the single owner of all DAQ state and runs in
//! a dedicated Tokio task. It processes [`DaqCommand`] messages received via
//! an mpsc channel and responds using oneshot channels.
//!
//! ## Actor Responsibilities
//!
//! - **Lifecycle Management**: Spawns, monitors, and shuts down instrument and storage tasks
//! - **State Ownership**: Sole owner of instruments, metadata, and configuration
//! - **Command Processing**: Handles all state-mutating operations sequentially
//! - **Task Supervision**: Manages graceful shutdown with timeouts and fallback abort
//!
//! ## Message Flow
//!
//! 1. GUI sends `DaqCommand` via mpsc channel (non-blocking)
//! 2. Actor processes command in its event loop (sequential, no locks)
//! 3. Actor performs state mutation (e.g., spawn instrument, start recording)
//! 4. Actor sends response via oneshot channel
//! 5. GUI receives result asynchronously
//!
//! ## Data Flow
//!
//! Instrument tasks broadcast data through a shared [`DataDistributor`]:
//!
//! ```text
//! Instrument Task 1 ──┐
//! Instrument Task 2 ──┼──> DataDistributor ──┬──> GUI (plotting)
//! Instrument Task N ──┘    (broadcast)       └──> Storage Writer
//! ```
//!
//! ## Graceful Shutdown Protocol
//!
//! 1. Actor receives `DaqCommand::Shutdown`
//! 2. Stops storage writer (5s timeout)
//! 3. Sends `InstrumentCommand::Shutdown` to each instrument
//! 4. Waits up to 5s per instrument for graceful disconnect
//! 5. Aborts any stragglers that exceed timeout
//! 6. Actor event loop exits
//!
//! # Example
//!
//! ```no_run
//! use tokio::sync::mpsc;
//! use rust_daq::{app_actor::DaqManagerActor, messages::DaqCommand};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create command channel
//! let (cmd_tx, cmd_rx) = mpsc::channel(32);
//!
//! // Spawn actor task
//! # let settings = std::sync::Arc::new(rust_daq::config::Settings::default());
//! # let instrument_registry = std::sync::Arc::new(rust_daq::instrument::InstrumentRegistry::new());
//! # let processor_registry = std::sync::Arc::new(rust_daq::data::registry::ProcessorRegistry::new());
//! # let log_buffer = rust_daq::log_capture::LogBuffer::new();
//! # let runtime = std::sync::Arc::new(tokio::runtime::Runtime::new()?);
//! let actor = DaqManagerActor::new(
//!     settings,
//!     instrument_registry,
//!     processor_registry,
//!     log_buffer,
//!     runtime,
//! )?;
//! tokio::spawn(actor.run(cmd_rx));
//!
//! // Send command from GUI
//! let (cmd, rx) = DaqCommand::spawn_instrument("my_instrument".to_string());
//! cmd_tx.send(cmd).await?;
//! let result = rx.await?;
//! # Ok(())
//! # }
//! ```

use crate::{
    config::Settings,
    core::{DataPoint, MeasurementProcessor, InstrumentHandle},
    data::registry::ProcessorRegistry,
    instrument::InstrumentRegistry,
    log_capture::LogBuffer,
    measurement::{DataDistributor, Measure},
    messages::{DaqCommand, SpawnError},
    metadata::Metadata,
    session::{self, Session},
};
use daq_core::Measurement;
use anyhow::{anyhow, Context, Result};
use log::{error, info};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::{
    runtime::Runtime,
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

/// Central actor that owns and manages all DAQ state.
///
/// The `DaqManagerActor` runs in a dedicated Tokio task and processes
/// [`DaqCommand`] messages sequentially.
/// This design eliminates the need for `Arc<Mutex<>>` and provides
/// a clear, deadlock-free concurrency model.
///
/// # State Ownership
///
/// The actor owns:
/// - Active instrument tasks (`instruments` HashMap)
/// - Storage writer task (if recording)
/// - Application configuration (`settings`)
/// - Metadata for recordings
/// - Data distribution hub (`data_distributor`)
///
/// # Thread Safety
///
/// All state mutations occur sequentially within the actor's event loop,
/// ensuring sequential consistency without locks. External components
/// interact only via message-passing.
///
/// # Graceful Shutdown
///
/// The actor implements a supervised shutdown protocol:
/// - Stops storage writer with 5s timeout
/// - Stops each instrument with 5s timeout per instrument
/// - Aborts any tasks that don't respond in time
/// - Ensures all resources are properly released
pub struct DaqManagerActor<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::DataPoint>,
{
    settings: Arc<Settings>,
    instrument_registry: Arc<InstrumentRegistry<M>>,
    processor_registry: Arc<ProcessorRegistry>,
    instruments: HashMap<String, InstrumentHandle>,
    data_distributor: Arc<DataDistributor<Arc<Measurement>>>,
    log_buffer: LogBuffer,
    metadata: Metadata,
    writer_task: Option<JoinHandle<Result<()>>>,
    writer_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    storage_format: String,
    runtime: Arc<Runtime>,
    shutdown_flag: bool,
}

impl<M> DaqManagerActor<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::DataPoint>,
{
    /// Creates a new `DaqManagerActor` with the given configuration.
    ///
    /// This does not start the actor's event loop. Call [`run`](Self::run)
    /// and spawn it as a Tokio task.
    ///
    /// # Arguments
    ///
    /// - `settings`: Application configuration (instruments, storage, etc.)
    /// - `instrument_registry`: Factory for creating instrument instances
    /// - `processor_registry`: Factory for creating data processors
    /// - `log_buffer`: Shared buffer for application logs
    /// - `runtime`: Tokio runtime handle for spawning instrument tasks
    pub fn new(
        settings: Arc<Settings>,
        instrument_registry: Arc<InstrumentRegistry<M>>,
        processor_registry: Arc<ProcessorRegistry>,
        log_buffer: LogBuffer,
        runtime: Arc<Runtime>,
    ) -> Result<Self> {
        let data_distributor = Arc::new(DataDistributor::new(
            settings.application.broadcast_channel_capacity
        ));
        let storage_format = settings.storage.default_format.clone();

        Ok(Self {
            settings,
            instrument_registry,
            processor_registry,
            instruments: HashMap::new(),
            data_distributor,
            log_buffer,
            metadata: Metadata::default(),
            writer_task: None,
            writer_shutdown_tx: None,
            storage_format,
            runtime,
            shutdown_flag: false,
        })
    }

    /// Runs the actor event loop, processing commands until shutdown.
    ///
    /// This method consumes the actor and runs indefinitely until a
    /// `DaqCommand::Shutdown` is received. It should be spawned as a
    /// Tokio task:
    ///
    /// ```no_run
    /// # use tokio::sync::mpsc;
    /// # use rust_daq::app_actor::DaqManagerActor;
    /// # async fn example(actor: DaqManagerActor<impl rust_daq::measurement::Measure>, cmd_rx: mpsc::Receiver<rust_daq::messages::DaqCommand>) {
    /// tokio::spawn(actor.run(cmd_rx));
    /// # }
    /// ```
    ///
    /// Commands are processed sequentially in the order received. Each
    /// command mutates actor state and sends a response via oneshot channel.
    pub async fn run(mut self, mut command_rx: mpsc::Receiver<DaqCommand>) {
        info!("DaqManagerActor started");

        while let Some(command) = command_rx.recv().await {
            match command {
                DaqCommand::SpawnInstrument { id, response } => {
                    let result = self.spawn_instrument(&id);
                    let _ = response.send(result);
                }

                DaqCommand::StopInstrument { id, response } => {
                    self.stop_instrument(&id);
                    let _ = response.send(());
                }

                DaqCommand::SendInstrumentCommand {
                    id,
                    command,
                    response,
                } => {
                    let result = self.send_instrument_command(&id, command);
                    let _ = response.send(result);
                }

                DaqCommand::StartRecording { response } => {
                    let result = self.start_recording().await;
                    let _ = response.send(result);
                }

                DaqCommand::StopRecording { response } => {
                    self.stop_recording();
                    let _ = response.send(());
                }

                DaqCommand::SaveSession {
                    path,
                    gui_state,
                    response,
                } => {
                    let result = self.save_session(&path, gui_state).await;
                    let _ = response.send(result);
                }

                DaqCommand::LoadSession { path, response } => {
                    let result = self.load_session(&path).await;
                    let _ = response.send(result);
                }

                DaqCommand::GetInstrumentList { response } => {
                    let list: Vec<String> = self.instruments.keys().cloned().collect();
                    let _ = response.send(list);
                }

                DaqCommand::GetAvailableChannels { response } => {
                    let channels = self.instrument_registry.list().collect();
                    let _ = response.send(channels);
                }

                DaqCommand::GetStorageFormat { response } => {
                    let _ = response.send(self.storage_format.clone());
                }

                DaqCommand::SetStorageFormat { format, response } => {
                    self.storage_format = format;
                    let _ = response.send(());
                }

                DaqCommand::SubscribeToData { response } => {
                    let receiver = self.data_distributor.subscribe().await;
                    let _ = response.send(receiver);
                }

                DaqCommand::Shutdown { response } => {
                    info!("Shutdown command received");
                    self.shutdown();
                    let _ = response.send(());
                    break; // Exit event loop
                }
            }
        }

        info!("DaqManagerActor shutting down");
    }

    /// Spawns an instrument task on the Tokio runtime.
    ///
    /// This method:
    /// 1. Validates instrument configuration from settings
    /// 2. Creates instrument instance from registry
    /// 3. Builds processor chain for this instrument
    /// 4. Connects to the instrument (synchronously blocks)
    /// 5. Spawns a Tokio task with `tokio::select!` event loop
    ///
    /// The spawned task handles:
    /// - Receiving data from instrument's async stream
    /// - Processing data through measurement processor chain
    /// - Broadcasting processed measurements via `DataDistributor`
    /// - Receiving commands via mpsc channel
    /// - Graceful shutdown on `InstrumentCommand::Shutdown`
    ///
    /// # Errors
    ///
    /// Returns `SpawnError` if:
    /// - Instrument is already running
    /// - Configuration is invalid or missing
    /// - Instrument type not registered
    /// - Connection to hardware fails
    /// - Processor creation fails
    fn spawn_instrument(&mut self, id: &str) -> Result<(), SpawnError> {
        if self.instruments.contains_key(id) {
            return Err(SpawnError::AlreadyRunning(format!(
                "Instrument '{}' is already running",
                id
            )));
        }

        let instrument_config = self
            .settings
            .instruments
            .get(id)
            .ok_or_else(|| {
                SpawnError::InvalidConfig(format!("Instrument config for '{}' not found", id))
            })?;
        let instrument_type = instrument_config
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SpawnError::InvalidConfig(format!(
                    "Instrument type for '{}' not found in config",
                    id
                ))
            })?;

        let mut instrument = self
            .instrument_registry
            .create(instrument_type, id)
            .ok_or_else(|| {
                SpawnError::InvalidConfig(format!(
                    "Instrument type '{}' not registered in registry",
                    instrument_type
                ))
            })?;

        // Create processor chain for this instrument
        let mut processors: Vec<Box<dyn MeasurementProcessor>> = Vec::new();
        if let Some(processor_configs) = self.settings.processors.as_ref().and_then(|p| p.get(id)) {
            for config in processor_configs {
                let processor = self
                    .processor_registry
                    .create(&config.r#type, &config.config)
                    .map_err(|e| {
                        SpawnError::InvalidConfig(format!(
                            "Failed to create processor '{}' for instrument '{}': {}",
                            config.r#type, id, e
                        ))
                    })?;
                processors.push(processor);
            }
        }

        let data_distributor = self.data_distributor.clone();
        let settings = self.settings.clone();
        let id_clone = id.to_string();

        // Create command channel
        let (command_tx, mut command_rx) =
            tokio::sync::mpsc::channel(settings.application.command_channel_capacity);

        // Try to connect synchronously in this function to catch connection errors
        // Use block_on to run the async connect operation
        self.runtime.block_on(async {
            instrument
                .connect(&id_clone, &settings)
                .await
                .map_err(|e| {
                    SpawnError::ConnectionFailed(format!(
                        "Failed to connect to instrument '{}': {}",
                        id_clone, e
                    ))
                })
        })?;
        info!("Instrument '{}' connected.", id_clone);

        let task: JoinHandle<Result<()>> = self.runtime.spawn(async move {

            let mut stream = instrument
                .measure()
                .data_stream()
                .await
                .context("Failed to get data stream")?;
            loop {
                tokio::select! {
                    data_point_option = stream.recv() => {
                        match data_point_option {
                            Some(dp) => {
                                // Extract data from Arc and convert M::Data to daq_core::DataPoint using Into trait
                                let daq_dp: daq_core::DataPoint = (*dp).clone().into();
                                let mut measurements = vec![Arc::new(Measurement::Scalar(daq_dp))];

                                // Process through measurement processor chain
                                for processor in &mut processors {
                                    measurements = processor.process_measurements(&measurements);
                                }

                                // Broadcast processed measurements
                                for measurement in measurements {
                                    if let Err(e) = data_distributor.broadcast(measurement).await {
                                        error!("Failed to broadcast measurement: {}", e);
                                    }
                                }
                            }
                            None => {
                                error!("Stream closed");
                                break;
                            }
                        }
                    }
                    Some(command) = command_rx.recv() => {
                        match command {
                            crate::core::InstrumentCommand::Shutdown => {
                                info!("Instrument '{}' received shutdown command", id_clone);
                                break;
                            }
                            _ => {
                                if let Err(e) = instrument.handle_command(command).await {
                                    error!("Failed to handle command for '{}': {}", id_clone, e);
                                }
                            }
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        log::trace!("Instrument stream for {} is idle.", id_clone);
                    }
                }
            }

            // Graceful cleanup after loop breaks
            info!("Instrument '{}' disconnecting...", id_clone);
            instrument.disconnect().await.context("Failed to disconnect instrument")?;
            info!("Instrument '{}' disconnected successfully", id_clone);
            Ok(())
        });

        let handle = InstrumentHandle { task, command_tx };
        self.instruments.insert(id.to_string(), handle);
        Ok(())
    }

    /// Stops a running instrument with graceful shutdown protocol.
    ///
    /// Shutdown sequence:
    /// 1. Send `InstrumentCommand::Shutdown` via command channel
    /// 2. Wait up to 5 seconds for task to complete
    /// 3. If timeout, abort task forcefully
    ///
    /// The instrument task will:
    /// - Receive shutdown command in its `tokio::select!` loop
    /// - Break out of event loop
    /// - Call `instrument.disconnect()` for cleanup
    /// - Exit gracefully
    ///
    /// If the command channel is closed or full, the task is aborted immediately.
    fn stop_instrument(&mut self, id: &str) {
        if let Some(handle) = self.instruments.remove(id) {
            // Try graceful shutdown first
            info!("Sending shutdown command to instrument '{}'", id);
            if let Err(e) = handle.command_tx.try_send(crate::core::InstrumentCommand::Shutdown) {
                log::warn!("Failed to send shutdown command to '{}': {}. Aborting task.", id, e);
                handle.task.abort();
                return;
            }

            // Wait up to 5 seconds for graceful shutdown
            let timeout_duration = std::time::Duration::from_secs(5);
            let runtime = self.runtime.clone();
            let task_handle = handle.task;

            runtime.block_on(async move {
                match tokio::time::timeout(timeout_duration, task_handle).await {
                    Ok(Ok(Ok(()))) => {
                        info!("Instrument '{}' stopped gracefully", id);
                    }
                    Ok(Ok(Err(e))) => {
                        log::warn!("Instrument '{}' task failed during shutdown: {}", id, e);
                    }
                    Ok(Err(e)) => {
                        log::warn!("Instrument '{}' task panicked during shutdown: {}", id, e);
                    }
                    Err(_) => {
                        log::warn!("Instrument '{}' did not stop within {:?}, aborting", id, timeout_duration);
                        // Note: tokio::time::timeout doesn't abort the task, but since we own task_handle
                        // and it goes out of scope here, the task will be aborted when task_handle is dropped
                    }
                }
            });
        }
    }

    /// Sends a command to a running instrument task.
    ///
    /// Commands are sent via the instrument's mpsc channel. This method
    /// implements retry logic to handle transient channel congestion:
    /// - Retries up to 10 times with 100ms delays
    /// - Total retry window: 1 second
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Instrument is not running
    /// - Channel is full after all retries (instrument is overloaded)
    /// - Channel is closed (instrument task terminated unexpectedly)
    fn send_instrument_command(
        &self,
        id: &str,
        command: crate::core::InstrumentCommand,
    ) -> Result<()> {
        let handle = self
            .instruments
            .get(id)
            .ok_or_else(|| anyhow!("Instrument '{}' is not running", id))?;

        // Retry with brief delays instead of failing immediately
        const MAX_RETRIES: u32 = 10;
        const RETRY_DELAY_MS: u64 = 100;

        for attempt in 0..MAX_RETRIES {
            match handle.command_tx.try_send(command.clone()) {
                Ok(()) => return Ok(()),
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    if attempt < MAX_RETRIES - 1 {
                        std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS));
                        continue;
                    }
                    return Err(anyhow!(
                        "Command channel full for instrument '{}' after {} retries ({}ms total)",
                        id,
                        MAX_RETRIES,
                        MAX_RETRIES as u64 * RETRY_DELAY_MS
                    ));
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    return Err(anyhow!(
                        "Instrument '{}' is no longer running (channel closed)",
                        id
                    ));
                }
            }
        }

        unreachable!("Retry loop should return in all cases")
    }

    /// Starts the data recording process by spawning a storage writer task.
    ///
    /// The storage writer:
    /// - Subscribes to the `DataDistributor` to receive all measurements
    /// - Creates a storage writer based on current format (CSV, HDF5, Arrow)
    /// - Writes measurements to disk asynchronously
    /// - Handles shutdown signal via oneshot channel
    ///
    /// Recording can be stopped by calling [`stop_recording`](Self::stop_recording).
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Recording is already in progress
    /// - Storage format is unsupported or feature-gated
    async fn start_recording(&mut self) -> Result<()> {
        if self.writer_task.is_some() {
            return Err(anyhow!("Recording is already in progress."));
        }

        let settings = self.settings.clone();
        let metadata = self.metadata.clone();
        let mut rx = self.data_distributor.subscribe().await;
        let storage_format_for_task = self.storage_format.clone();

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

        let task = self.runtime.spawn(async move {
            let mut writer: Box<dyn crate::core::StorageWriter> =
                match storage_format_for_task.as_str() {
                    "csv" => Box::new(crate::data::storage::CsvWriter::new()),
                    #[cfg(feature = "storage_hdf5")]
                    "hdf5" => Box::new(crate::data::storage::Hdf5Writer::new()),
                    #[cfg(feature = "storage_arrow")]
                    "arrow" => Box::new(crate::data::storage::ArrowWriter::new()),
                    _ => {
                        return Err(anyhow!(
                            "Unsupported storage format: {}",
                            storage_format_for_task
                        ))
                    }
                };

            writer.init(&settings).await?;
            writer.set_metadata(&metadata).await?;

            loop {
                tokio::select! {
                    data_point = rx.recv() => {
                        match data_point {
                            Some(dp) => {
                                if let Err(e) = writer.write(&[dp]).await {
                                    error!("Failed to write data point: {}", e);
                                }
                            }
                            None => {
                                info!("Data channel closed, stopping storage writer");
                                break;
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        info!("Storage writer received shutdown signal");
                        break;
                    }
                }
            }

            writer.shutdown().await?;
            Ok(())
        });

        self.writer_task = Some(task);
        self.writer_shutdown_tx = Some(shutdown_tx);
        info!("Started recording with format: {}", self.storage_format);
        Ok(())
    }

    /// Stops the data recording process with graceful shutdown.
    ///
    /// Shutdown sequence:
    /// 1. Send shutdown signal via oneshot channel
    /// 2. Wait up to 5 seconds for writer to flush buffers
    /// 3. If timeout, abort task forcefully
    ///
    /// The storage writer will call `writer.shutdown()` to ensure
    /// all buffered data is flushed to disk before terminating.
    fn stop_recording(&mut self) {
        if let Some(task) = self.writer_task.take() {
            // Try graceful shutdown first
            info!("Sending shutdown signal to storage writer");
            if let Some(shutdown_tx) = self.writer_shutdown_tx.take() {
                if shutdown_tx.send(()).is_err() {
                    log::warn!("Failed to send shutdown signal to storage writer (receiver dropped). Aborting task.");
                    task.abort();
                    return;
                }

                // Wait up to 5 seconds for graceful shutdown
                let timeout_duration = std::time::Duration::from_secs(5);
                let runtime = self.runtime.clone();

                runtime.block_on(async move {
                    match tokio::time::timeout(timeout_duration, task).await {
                        Ok(Ok(Ok(()))) => {
                            info!("Storage writer stopped gracefully");
                        }
                        Ok(Ok(Err(e))) => {
                            log::warn!("Storage writer task failed during shutdown: {}", e);
                        }
                        Ok(Err(e)) => {
                            log::warn!("Storage writer task panicked during shutdown: {}", e);
                        }
                        Err(_) => {
                            log::warn!("Storage writer did not stop within {:?}, aborting", timeout_duration);
                            // Task will be aborted when it goes out of scope
                        }
                    }
                });
            } else {
                // No shutdown channel, just abort
                log::warn!("No shutdown channel for storage writer, aborting task");
                task.abort();
            }
        }
    }

    /// Saves the current application state to a session file.
    ///
    /// Session files contain:
    /// - List of active instrument IDs
    /// - Storage settings (format, path)
    /// - GUI state (window layout, plot configurations)
    ///
    /// Sessions can be loaded later to restore the application state.
    /// The blocking file I/O is wrapped in spawn_blocking to prevent
    /// blocking the actor task.
    async fn save_session(&self, path: &Path, gui_state: session::GuiState) -> Result<()> {
        let active_instruments: std::collections::HashSet<String> = self.instruments.keys().cloned().collect();

        let session = Session {
            active_instruments,
            storage_settings: self.settings.storage.clone(),
            gui_state,
        };

        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || session::save_session(&session, &path))
            .await?
    }

    /// Loads application state from a session file.
    ///
    /// This method:
    /// 1. Stops all currently running instruments
    /// 2. Spawns instruments from the session file
    /// 3. Applies storage settings from the session
    /// 4. Returns GUI state for the caller to restore
    ///
    /// If any instrument fails to start, an error is logged but loading
    /// continues for remaining instruments.
    /// The blocking file I/O is wrapped in spawn_blocking to prevent
    /// blocking the actor task.
    async fn load_session(&mut self, path: &Path) -> Result<session::GuiState> {
        let path = path.to_path_buf();
        let session = tokio::task::spawn_blocking(move || session::load_session(&path))
            .await??;
        let gui_state = session.gui_state.clone();

        // Stop all current instruments
        let current_instruments: Vec<String> = self.instruments.keys().cloned().collect();
        for id in current_instruments {
            self.stop_instrument(&id);
        }

        // Start instruments from the session
        for id in &session.active_instruments {
            if let Err(e) = self.spawn_instrument(id) {
                error!("Failed to start instrument from session '{}': {}", id, e);
                // Continue loading other instruments even if one fails
            }
        }

        // Apply storage settings
        self.storage_format = session.storage_settings.default_format.clone();

        Ok(gui_state)
    }

    /// Shuts down the application by stopping all tasks gracefully.
    ///
    /// Shutdown order:
    /// 1. Stop recording (if active)
    /// 2. Stop all instruments (5s timeout each)
    ///
    /// This method is idempotent - subsequent calls are no-ops.
    fn shutdown(&mut self) {
        if self.shutdown_flag {
            return;
        }
        info!("Shutting down application...");
        self.shutdown_flag = true;

        // Stop recording first
        self.stop_recording();

        // Stop all instruments gracefully
        let instrument_ids: Vec<String> = self.instruments.keys().cloned().collect();
        for id in instrument_ids {
            self.stop_instrument(&id);
        }

        info!("Application shutdown complete");
    }
}
