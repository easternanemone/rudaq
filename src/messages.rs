//! Message types for actor-based communication
//!
//! This module defines the command and response types used for message-passing
//! between the GUI and the DaqManagerActor. This replaces the Arc<Mutex<DaqAppInner>>
//! pattern with non-blocking async message passing.

use crate::{
    core::InstrumentCommand,
    session::GuiState,
};
use anyhow::Result;
use daq_core::Measurement;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Errors that can occur during instrument spawning
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("Configuration invalid: {0}")]
    InvalidConfig(String),
    #[error("Failed to connect: {0}")]
    ConnectionFailed(String),
    #[error("Instrument already running: {0}")]
    AlreadyRunning(String),
}

/// Commands that can be sent to the DaqManagerActor
#[derive(Debug)]
pub enum DaqCommand {
    /// Spawn a new instrument
    SpawnInstrument {
        id: String,
        response: oneshot::Sender<Result<(), SpawnError>>,
    },

    /// Stop a running instrument
    StopInstrument {
        id: String,
        response: oneshot::Sender<()>,
    },

    /// Send a command to a running instrument
    SendInstrumentCommand {
        id: String,
        command: InstrumentCommand,
        response: oneshot::Sender<Result<()>>,
    },

    /// Start recording data to storage
    StartRecording {
        response: oneshot::Sender<Result<()>>,
    },

    /// Stop recording data
    StopRecording {
        response: oneshot::Sender<()>,
    },

    /// Save current session to file
    SaveSession {
        path: PathBuf,
        gui_state: GuiState,
        response: oneshot::Sender<Result<()>>,
    },

    /// Load session from file
    LoadSession {
        path: PathBuf,
        response: oneshot::Sender<Result<GuiState>>,
    },

    /// Get list of running instrument IDs
    GetInstrumentList {
        response: oneshot::Sender<Vec<String>>,
    },

    /// Get list of available channel names
    GetAvailableChannels {
        response: oneshot::Sender<Vec<String>>,
    },

    /// Get current storage format
    GetStorageFormat {
        response: oneshot::Sender<String>,
    },

    /// Set storage format
    SetStorageFormat {
        format: String,
        response: oneshot::Sender<()>,
    },

    /// Subscribe to data broadcast channel
    SubscribeToData {
        response: oneshot::Sender<mpsc::Receiver<Arc<Measurement>>>,
    },

    /// Shutdown the DAQ system
    Shutdown {
        response: oneshot::Sender<()>,
    },
}

impl DaqCommand {
    /// Helper to create a SpawnInstrument command
    pub fn spawn_instrument(id: String) -> (Self, oneshot::Receiver<Result<(), SpawnError>>) {
        let (tx, rx) = oneshot::channel();
        (Self::SpawnInstrument { id, response: tx }, rx)
    }

    /// Helper to create a StopInstrument command
    pub fn stop_instrument(id: String) -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Self::StopInstrument { id, response: tx }, rx)
    }

    /// Helper to create a SendInstrumentCommand command
    pub fn send_instrument_command(
        id: String,
        command: InstrumentCommand,
    ) -> (Self, oneshot::Receiver<Result<()>>) {
        let (tx, rx) = oneshot::channel();
        (
            Self::SendInstrumentCommand {
                id,
                command,
                response: tx,
            },
            rx,
        )
    }

    /// Helper to create a StartRecording command
    pub fn start_recording() -> (Self, oneshot::Receiver<Result<()>>) {
        let (tx, rx) = oneshot::channel();
        (Self::StartRecording { response: tx }, rx)
    }

    /// Helper to create a StopRecording command
    pub fn stop_recording() -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Self::StopRecording { response: tx }, rx)
    }

    /// Helper to create a SaveSession command
    pub fn save_session(path: PathBuf, gui_state: GuiState) -> (Self, oneshot::Receiver<Result<()>>) {
        let (tx, rx) = oneshot::channel();
        (
            Self::SaveSession {
                path,
                gui_state,
                response: tx,
            },
            rx,
        )
    }

    /// Helper to create a LoadSession command
    pub fn load_session(path: PathBuf) -> (Self, oneshot::Receiver<Result<GuiState>>) {
        let (tx, rx) = oneshot::channel();
        (Self::LoadSession { path, response: tx }, rx)
    }

    /// Helper to create a GetInstrumentList command
    pub fn get_instrument_list() -> (Self, oneshot::Receiver<Vec<String>>) {
        let (tx, rx) = oneshot::channel();
        (Self::GetInstrumentList { response: tx }, rx)
    }

    /// Helper to create a GetAvailableChannels command
    pub fn get_available_channels() -> (Self, oneshot::Receiver<Vec<String>>) {
        let (tx, rx) = oneshot::channel();
        (Self::GetAvailableChannels { response: tx }, rx)
    }

    /// Helper to create a GetStorageFormat command
    pub fn get_storage_format() -> (Self, oneshot::Receiver<String>) {
        let (tx, rx) = oneshot::channel();
        (Self::GetStorageFormat { response: tx }, rx)
    }

    /// Helper to create a SetStorageFormat command
    pub fn set_storage_format(format: String) -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Self::SetStorageFormat { format, response: tx }, rx)
    }

    /// Helper to create a SubscribeToData command
    pub fn subscribe_to_data() -> (Self, oneshot::Receiver<mpsc::Receiver<Arc<Measurement>>>) {
        let (tx, rx) = oneshot::channel();
        (Self::SubscribeToData { response: tx }, rx)
    }

    /// Helper to create a Shutdown command
    pub fn shutdown() -> (Self, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Self::Shutdown { response: tx }, rx)
    }
}
