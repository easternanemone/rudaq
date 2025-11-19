//! The core application state and logic (Actor-based implementation)
//!
//! This module uses the actor pattern to eliminate Arc<Mutex<>> lock contention.
//! All state is owned by DaqManagerActor, and GUI/session code communicates via
//! message-passing through mpsc channels.

use crate::core_v3::Measurement;
use crate::{
    app_actor::DaqManagerActor, config::Settings, data::registry::ProcessorRegistry,
    instrument::InstrumentRegistry, log_capture::LogBuffer, measurement::Measure,
    messages::DaqCommand, session,
};
use anyhow::{Context, Result};
use log::info;
use std::path::Path;
use std::sync::Arc;
use tokio::{runtime::Runtime, sync::mpsc};

/// The main application struct (actor-based implementation)
///
/// **DEPRECATION NOTICE**: This struct is a compatibility layer for tests and will be removed
/// in Phase 3. New code should communicate directly with DaqManagerActor via command channels.
/// The blocking `with_inner()` method causes GUI freezes and is being phased out (bd-cd89).
#[derive(Clone)]
pub struct DaqApp<M = crate::measurement::InstrumentMeasurement>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    pub command_tx: mpsc::Sender<DaqCommand>,
    pub runtime: Arc<Runtime>,
    // Immutable shared state for GUI access
    pub settings: Settings,
    pub log_buffer: LogBuffer,
    pub instrument_registry: Arc<InstrumentRegistry<M>>,
    pub instrument_registry_v2: Arc<crate::instrument::InstrumentRegistryV2>,
    pub _phantom: std::marker::PhantomData<M>,
}

impl<M> DaqApp<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    /// Creates a new `DaqApp` with actor-based state management
    pub fn new(
        settings: Arc<Settings>,
        instrument_registry: Arc<InstrumentRegistry<M>>,
        processor_registry: Arc<ProcessorRegistry>,
        module_registry: Arc<crate::modules::ModuleRegistry<M>>,
        log_buffer: LogBuffer,
    ) -> Result<Self> {
        Self::new_with_v2(
            settings,
            instrument_registry,
            Arc::new(crate::instrument::InstrumentRegistryV2::new()),
            processor_registry,
            module_registry,
            log_buffer,
        )
    }

    /// Creates a new `DaqApp` with explicit V2 instrument registry (advanced use).
    pub fn new_with_v2(
        settings: Arc<Settings>,
        instrument_registry: Arc<InstrumentRegistry<M>>,
        instrument_registry_v2: Arc<crate::instrument::InstrumentRegistryV2>,
        processor_registry: Arc<ProcessorRegistry>,
        module_registry: Arc<crate::modules::ModuleRegistry<M>>,
        log_buffer: LogBuffer,
    ) -> Result<Self> {
        let runtime = Arc::new(Runtime::new().context("Failed to create Tokio runtime")?);

        // Settings are shared via Arc for callers but stored by value internally for
        // backwards compatibility with older APIs.
        let settings = settings.as_ref().clone();

        // Create the actor
        let actor = DaqManagerActor::new(
            settings.clone(),
            instrument_registry.clone(),
            instrument_registry_v2.clone(),
            processor_registry,
            module_registry,
            runtime.clone(),
        )?;

        // Create command channel
        let (command_tx, command_rx) = mpsc::channel(settings.application.command_channel_capacity);

        // Spawn instruments from config before starting actor
        let instrument_ids: Vec<String> = settings.instruments.keys().cloned().collect();

        // Spawn the actor task
        let runtime_clone = runtime.clone();
        runtime_clone.spawn(async move {
            actor.run(command_rx).await;
        });

        // Spawn configured instruments
        for id in instrument_ids {
            let (cmd, rx) = DaqCommand::spawn_instrument(id.clone());
            if command_tx.blocking_send(cmd).is_ok() {
                if let Ok(result) = rx.blocking_recv() {
                    if let Err(e) = result {
                        log::error!("Failed to spawn instrument '{}': {}", id, e);
                    }
                }
            }
        }

        Ok(Self {
            command_tx,
            runtime,
            settings,
            log_buffer,
            instrument_registry,
            instrument_registry_v2,
            _phantom: std::marker::PhantomData,
        })
    }

    /// Returns a clone of the application's Tokio runtime handle
    pub fn get_runtime(&self) -> Arc<Runtime> {
        self.runtime.clone()
    }

    /// Shuts down the application
    pub fn shutdown(&self) -> Result<()> {
        let (cmd, rx) = DaqCommand::shutdown();
        self.command_tx
            .blocking_send(cmd)
            .map_err(|_| anyhow::anyhow!("Failed to send shutdown command"))?;
        rx.blocking_recv()
            .map_err(|_| anyhow::anyhow!("Failed to receive shutdown response"))?
            .map_err(|e| anyhow::anyhow!("Shutdown error: {}", e))?;
        info!("Application shutdown complete");
        Ok(())
    }

    /// Saves the current application state to a session file
    pub fn save_session(&self, path: &Path, gui_state: session::GuiState) -> Result<()> {
        let (cmd, rx) = DaqCommand::save_session(path.to_path_buf(), gui_state);
        self.command_tx
            .blocking_send(cmd)
            .map_err(|_| anyhow::anyhow!("Failed to send save session command"))?;
        rx.blocking_recv()
            .map_err(|_| anyhow::anyhow!("Failed to receive save session response"))?
    }

    /// Loads application state from a session file
    pub fn load_session(&self, path: &Path) -> Result<session::GuiState> {
        let (cmd, rx) = DaqCommand::load_session(path.to_path_buf());
        self.command_tx
            .blocking_send(cmd)
            .map_err(|_| anyhow::anyhow!("Failed to send load session command"))?;
        rx.blocking_recv()
            .map_err(|_| anyhow::anyhow!("Failed to receive load session response"))?
    }

    /// Helper method to access actor state (for backwards compatibility with tests)
    ///
    /// **DEPRECATED**: This method uses blocking operations that freeze the GUI.
    /// Use async message-passing with DaqCommand instead. Will be removed in Phase 3 (bd-51).
    ///
    /// This provides a similar interface to the old with_inner() pattern but uses
    /// message-passing under the hood. Note that this is less efficient than direct
    /// async methods and should only be used for compatibility during migration.
    #[deprecated(
        since = "0.2.0",
        note = "Use async DaqCommand message-passing instead. Causes GUI freezes."
    )]
    pub fn with_inner<F, R>(&self, f: F) -> R
    where
        M: 'static,
        F: FnOnce(&mut DaqAppCompat<M>) -> R,
    {
        let mut compat = DaqAppCompat {
            command_tx: self.command_tx.clone(),
            settings: Arc::new(self.settings.clone()),
            log_buffer: self.log_buffer.clone(),
            instrument_registry: self.instrument_registry.clone(),
            data_sender: DaqDataSender {
                command_tx: self.command_tx.clone(),
            },
            instruments: DaqInstruments {
                command_tx: self.command_tx.clone(),
            },
            _phantom: std::marker::PhantomData,
        };
        f(&mut compat)
    }
}

/// Compatibility shim for code that uses with_inner()
///
/// This struct provides the same methods as DaqAppInner but routes them through
/// the actor via message-passing. This allows existing test code to work while
/// we migrate to async methods.
pub struct DaqAppCompat<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    command_tx: mpsc::Sender<DaqCommand>,
    pub settings: Arc<Settings>,
    pub log_buffer: LogBuffer,
    pub instrument_registry: Arc<InstrumentRegistry<M>>,
    pub data_sender: DaqDataSender,
    pub instruments: DaqInstruments,
    _phantom: std::marker::PhantomData<M>,
}

impl<M> DaqAppCompat<M>
where
    M: Measure + 'static,
    M::Data: Into<daq_core::Measurement>,
{
    /// Spawns an instrument
    pub fn spawn_instrument(&mut self, id: &str) -> Result<()> {
        let (cmd, rx) = DaqCommand::spawn_instrument(id.to_string());
        self.command_tx
            .blocking_send(cmd)
            .map_err(|_| anyhow::anyhow!("Failed to send spawn command"))?;
        rx.blocking_recv()
            .map_err(|_| anyhow::anyhow!("Failed to receive spawn response"))?
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    /// Stops an instrument
    pub fn stop_instrument(&mut self, id: &str) {
        let (cmd, rx) = DaqCommand::stop_instrument(id.to_string());
        let _ = self.command_tx.blocking_send(cmd);
        let _ = rx.blocking_recv();
    }

    /// Sends a command to an instrument
    pub fn send_instrument_command(
        &self,
        id: &str,
        command: crate::core::InstrumentCommand,
    ) -> Result<()> {
        let (cmd, rx) = DaqCommand::send_instrument_command(id.to_string(), command);
        self.command_tx
            .blocking_send(cmd)
            .map_err(|_| anyhow::anyhow!("Failed to send instrument command"))?;
        rx.blocking_recv()
            .map_err(|_| anyhow::anyhow!("Failed to receive instrument command response"))?
    }

    /// Gets the data broadcast sender for subscribing
    pub fn data_sender(&self) -> DaqDataSender {
        DaqDataSender {
            command_tx: self.command_tx.clone(),
        }
    }

    /// Gets the list of running instruments
    pub fn instruments(&self) -> DaqInstruments {
        DaqInstruments {
            command_tx: self.command_tx.clone(),
        }
    }

    /// Gets the storage format
    pub fn storage_format(&self) -> DaqStorageFormat {
        DaqStorageFormat {
            command_tx: self.command_tx.clone(),
        }
    }

    /// Sets the storage format
    pub fn set_storage_format(&mut self, format: String) {
        let (cmd, rx) = DaqCommand::set_storage_format(format);
        let _ = self.command_tx.blocking_send(cmd);
        let _ = rx.blocking_recv();
    }

    /// Starts recording
    pub fn start_recording(&mut self) -> Result<()> {
        let (cmd, rx) = DaqCommand::start_recording();
        self.command_tx
            .blocking_send(cmd)
            .map_err(|_| anyhow::anyhow!("Failed to send start recording command"))?;
        rx.blocking_recv()
            .map_err(|_| anyhow::anyhow!("Failed to receive start recording response"))?
    }

    /// Stops recording
    pub fn stop_recording(&mut self) {
        let (cmd, rx) = DaqCommand::stop_recording();
        let _ = self.command_tx.blocking_send(cmd);
        let _ = rx.blocking_recv();
    }

    /// Gets the list of available channels from the instrument registry
    pub fn get_available_channels(&self) -> Vec<String> {
        self.instrument_registry.list().collect()
    }
}

/// Helper struct for data_sender compatibility
pub struct DaqDataSender {
    command_tx: mpsc::Sender<DaqCommand>,
}

impl DaqDataSender {
    pub fn subscribe(&self) -> mpsc::Receiver<Arc<Measurement>> {
        let (cmd, rx) = DaqCommand::subscribe_to_data();
        self.command_tx.blocking_send(cmd).ok();
        rx.blocking_recv().unwrap_or_else(|_| {
            // Fallback: create a dummy receiver
            let (tx, rx) = mpsc::channel(1);
            drop(tx);
            rx
        })
    }
}

/// Helper struct for instruments compatibility
pub struct DaqInstruments {
    command_tx: mpsc::Sender<DaqCommand>,
}

impl DaqInstruments {
    pub fn len(&self) -> usize {
        let (cmd, rx) = DaqCommand::get_instrument_list();
        self.command_tx.blocking_send(cmd).ok();
        rx.blocking_recv().map(|list| list.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn keys(&self) -> impl Iterator<Item = String> {
        let (cmd, rx) = DaqCommand::get_instrument_list();
        self.command_tx.blocking_send(cmd).ok();
        rx.blocking_recv().unwrap_or_default().into_iter()
    }

    pub fn contains_key(&self, key: &str) -> bool {
        let (cmd, rx) = DaqCommand::get_instrument_list();
        self.command_tx.blocking_send(cmd).ok();
        rx.blocking_recv()
            .map(|list| list.contains(&key.to_string()))
            .unwrap_or(false)
    }
}

/// Helper struct for storage_format compatibility
pub struct DaqStorageFormat {
    command_tx: mpsc::Sender<DaqCommand>,
}

impl DaqStorageFormat {
    pub fn clone(&self) -> String {
        let (cmd, rx) = DaqCommand::get_storage_format();
        self.command_tx.blocking_send(cmd).ok();
        rx.blocking_recv().unwrap_or_else(|_| "csv".to_string())
    }
}
