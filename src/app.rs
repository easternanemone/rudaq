//! The core application state and logic.
use crate::{
    config::Settings,
    core::{DataPoint, DataProcessor, InstrumentHandle},
    data::registry::ProcessorRegistry,
    instrument::InstrumentRegistry,
    log_capture::LogBuffer,
    metadata::Metadata,
    session::{self, Session},
};
use anyhow::{anyhow, Context, Result};
use log::{error, info};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::{runtime::Runtime, sync::broadcast, task::JoinHandle};

/// The main application struct that holds all state.
#[derive(Clone)]
pub struct DaqApp {
    inner: Arc<Mutex<DaqAppInner>>,
}

/// Inner state of the DAQ application, protected by a Mutex.
pub struct DaqAppInner {
    pub settings: Arc<Settings>,
    pub instrument_registry: Arc<InstrumentRegistry>,
    pub processor_registry: Arc<ProcessorRegistry>,
    pub instruments: HashMap<String, InstrumentHandle>,
    pub data_sender: broadcast::Sender<DataPoint>,
    pub log_buffer: LogBuffer,
    pub metadata: Metadata,
    pub writer_task: Option<JoinHandle<Result<()>>>,
    pub storage_format: String,
    runtime: Arc<Runtime>,
    shutdown_flag: bool,
}

impl DaqApp {
    /// Creates a new `DaqApp`.
    pub fn new(
        settings: Arc<Settings>,
        instrument_registry: Arc<InstrumentRegistry>,
        processor_registry: Arc<ProcessorRegistry>,
        log_buffer: LogBuffer,
    ) -> Result<Self> {
        let runtime = Arc::new(Runtime::new().context("Failed to create Tokio runtime")?);
        let (data_sender, _) = broadcast::channel(1024);
        let storage_format = settings.storage.default_format.clone();

        let mut inner = DaqAppInner {
            settings: settings.clone(),
            instrument_registry,
            processor_registry,
            instruments: HashMap::new(),
            data_sender,
            log_buffer,
            metadata: Metadata::default(),
            writer_task: None,
            storage_format,
            runtime,
            shutdown_flag: false,
        };

        for id in settings.instruments.keys() {
            if let Err(e) = inner.spawn_instrument(id) {
                error!("Failed to spawn instrument '{}': {}", id, e);
            }
        }

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    /// Provides safe access to the inner application state.
    pub fn with_inner<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut DaqAppInner) -> R,
    {
        let mut inner = self.inner.lock().unwrap();
        f(&mut inner)
    }

    /// Returns a clone of the application's Tokio runtime handle.
    pub fn get_runtime(&self) -> Arc<Runtime> {
        self.with_inner(|inner| inner.runtime.clone())
    }

    /// Shuts down the application, stopping all instruments and the Tokio runtime.
    pub fn shutdown(&self) {
        self.with_inner(|inner| {
            if inner.shutdown_flag {
                return;
            }
            info!("Shutting down application runtime...");
            inner.shutdown_flag = true;
            // Stop all instruments
            for (id, handle) in inner.instruments.drain() {
                info!("Stopping instrument: {}", id);
                handle.task.abort();
            }
        });
    }

    /// Saves the current application state to a session file.
    pub fn save_session(&self, path: &Path, gui_state: session::GuiState) -> Result<()> {
        let session = Session::from_app(self, gui_state);
        session::save_session(&session, path)
    }

    /// Loads application state from a session file.
    pub fn load_session(&self, path: &Path) -> Result<session::GuiState> {
        let session = session::load_session(path)?;
        let gui_state = session.gui_state.clone();
        session.apply_to_app(self);
        Ok(gui_state)
    }
}

impl DaqAppInner {
    /// Spawns an instrument to run on the Tokio runtime.
    pub fn spawn_instrument(&mut self, id: &str) -> Result<()> {
        if self.instruments.contains_key(id) {
            return Err(anyhow!("Instrument '{}' is already running.", id));
        }

        let instrument_config = self
            .settings
            .instruments
            .get(id)
            .ok_or_else(|| anyhow!("Instrument config for '{}' not found.", id))?;
        let instrument_type = instrument_config
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Instrument type for '{}' not found in config.", id))?;

        let mut instrument = self
            .instrument_registry
            .create(instrument_type, id)
            .ok_or_else(|| anyhow!("Instrument type '{}' not found.", instrument_type))?;

        // Create processor chain for this instrument
        let mut processors: Vec<Box<dyn DataProcessor>> = Vec::new();
        if let Some(processor_configs) = self.settings.processors.as_ref().and_then(|p| p.get(id)) {
            for config in processor_configs {
                let processor = self
                    .processor_registry
                    .create(&config.r#type, &config.config)
                    .with_context(|| {
                        format!(
                            "Failed to create processor '{}' for instrument '{}'",
                            config.r#type, id
                        )
                    })?;
                processors.push(processor);
            }
        }

        let data_sender = self.data_sender.clone();
        let settings = self.settings.clone();
        let id_clone = id.to_string();

        // Create command channel
        let (command_tx, mut command_rx) = tokio::sync::mpsc::channel(32);

        let task: JoinHandle<Result<()>> = self.runtime.spawn(async move {
            instrument
                .connect(&settings)
                .await
                .with_context(|| format!("Failed to connect to instrument '{}'", id_clone))?;
            info!("Instrument '{}' connected.", id_clone);

            let mut stream = instrument
                .data_stream()
                .await
                .context("Failed to get data stream")?;
            loop {
                tokio::select! {
                    data_point_result = stream.recv() => {
                        match data_point_result {
                            Ok(dp) => {
                                let mut data_points = vec![dp];
                                for processor in &mut processors {
                                    data_points = processor.process(&data_points);
                                }

                                for processed_dp in data_points {
                                    if let Err(e) = data_sender.send(processed_dp) {
                                        error!("Failed to broadcast data point: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Stream receive error: {}", e);
                                break;
                            }
                        }
                    }
                    Some(command) = command_rx.recv() => {
                        if let Err(e) = instrument.handle_command(command).await {
                            error!("Failed to handle command for '{}': {}", id_clone, e);
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        log::trace!("Instrument stream for {} is idle.", id_clone);
                    }
                }
            }
            Ok(())
        });

        let handle = InstrumentHandle { task, command_tx };
        self.instruments.insert(id.to_string(), handle);
        Ok(())
    }

    /// Stops a running instrument.
    pub fn stop_instrument(&mut self, id: &str) {
        if let Some(handle) = self.instruments.remove(id) {
            handle.task.abort();
            info!("Instrument '{}' stopped.", id);
        }
    }

    /// Sends a command to a running instrument.
    pub fn send_instrument_command(&self, id: &str, command: crate::core::InstrumentCommand) -> Result<()> {
        let handle = self.instruments.get(id)
            .ok_or_else(|| anyhow!("Instrument '{}' is not running", id))?;

        handle.command_tx.try_send(command)
            .map_err(|e| anyhow!("Failed to send command to instrument '{}': {}", id, e))?;

        Ok(())
    }

    /// Returns a list of available channel names.
    pub fn get_available_channels(&self) -> Vec<String> {
        self.instrument_registry.list().collect()
    }

    /// Starts the data recording process.
    pub fn start_recording(&mut self) -> Result<()> {
        if self.writer_task.is_some() {
            return Err(anyhow!("Recording is already in progress."));
        }

        let settings = self.settings.clone();
        let metadata = self.metadata.clone();
        let mut rx = self.data_sender.subscribe();
        let storage_format_for_task = self.storage_format.clone();

        let task = self.runtime.spawn(async move {
            let mut writer: Box<dyn crate::core::StorageWriter> =
                match storage_format_for_task.as_str() {
                    "csv" => Box::new(crate::data::storage::CsvWriter::new()),
                    "hdf5" => Box::new(crate::data::storage::Hdf5Writer::new()),
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
                            Ok(dp) => {
                                if let Err(e) = writer.write(&[dp]).await {
                                    error!("Failed to write data point: {}", e);
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                log::warn!("Data writer lagged by {} messages.", n);
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                break;
                            }
                        }
                    }
                }
            }

            writer.shutdown().await?;
            Ok(())
        });

        self.writer_task = Some(task);
        info!("Started recording with format: {}", self.storage_format);
        Ok(())
    }

    /// Stops the data recording process.
    pub fn stop_recording(&mut self) {
        if let Some(task) = self.writer_task.take() {
            task.abort();
            info!("Stopped recording.");
        }
    }
}
