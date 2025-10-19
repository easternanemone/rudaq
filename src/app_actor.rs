//! Actor-based DAQ application state management
//!
//! This module implements the actor pattern to replace Arc<Mutex<DaqAppInner>>.
//! All state mutations happen in a single async task that processes commands
//! via message-passing, eliminating lock contention and improving scalability.

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

/// Actor that manages all DAQ state
pub struct DaqManagerActor<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::DataPoint>,
{
    settings: Arc<Settings>,
    instrument_registry: Arc<InstrumentRegistry<M>>,
    processor_registry: Arc<ProcessorRegistry>,
    instruments: HashMap<String, InstrumentHandle>,
    data_distributor: Arc<Mutex<DataDistributor<Arc<Measurement>>>>,
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
    /// Creates a new DaqManagerActor
    pub fn new(
        settings: Arc<Settings>,
        instrument_registry: Arc<InstrumentRegistry<M>>,
        processor_registry: Arc<ProcessorRegistry>,
        log_buffer: LogBuffer,
        runtime: Arc<Runtime>,
    ) -> Result<Self> {
        let data_distributor = Arc::new(Mutex::new(DataDistributor::new(
            settings.application.broadcast_channel_capacity
        )));
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

    /// Runs the actor event loop, processing commands until shutdown
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
                    let result = self.start_recording();
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
                    let result = self.save_session(&path, gui_state);
                    let _ = response.send(result);
                }

                DaqCommand::LoadSession { path, response } => {
                    let result = self.load_session(&path);
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
                    let receiver = {
                        let mut dist = self.data_distributor.lock().await;
                        dist.subscribe()
                    };
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

    /// Spawns an instrument to run on the Tokio runtime
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
                                // Convert M::Data to daq_core::DataPoint using Into trait
                                let daq_dp: daq_core::DataPoint = dp.into();
                                let mut measurements = vec![Arc::new(Measurement::Scalar(daq_dp))];

                                // Process through measurement processor chain
                                for processor in &mut processors {
                                    measurements = processor.process_measurements(&measurements);
                                }

                                // Broadcast processed measurements
                                for measurement in measurements {
                                    let mut dist = data_distributor.lock().await;
                                    if let Err(e) = dist.broadcast(measurement).await {
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

    /// Stops a running instrument
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

    /// Sends a command to a running instrument
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

    /// Starts the data recording process
    fn start_recording(&mut self) -> Result<()> {
        if self.writer_task.is_some() {
            return Err(anyhow!("Recording is already in progress."));
        }

        let settings = self.settings.clone();
        let metadata = self.metadata.clone();
        let mut rx = {
            let mut dist = self.data_distributor.blocking_lock();
            dist.subscribe()
        };
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

    /// Stops the data recording process
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

    /// Saves the current application state to a session file
    fn save_session(&self, path: &Path, gui_state: session::GuiState) -> Result<()> {
        let active_instruments: std::collections::HashSet<String> = self.instruments.keys().cloned().collect();

        let session = Session {
            active_instruments,
            storage_settings: self.settings.storage.clone(),
            gui_state,
        };

        session::save_session(&session, path)
    }

    /// Loads application state from a session file
    fn load_session(&mut self, path: &Path) -> Result<session::GuiState> {
        let session = session::load_session(path)?;
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

    /// Shuts down the application, stopping all instruments
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
