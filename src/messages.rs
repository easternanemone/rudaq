//! Message types for actor-based communication.
//!
//! This module defines the command and response types used for message-passing
//! between the GUI and the `DaqManagerActor` (see [`crate::app_actor`]).
//!
//! # Architecture
//!
//! The message protocol replaces the previous `Arc<Mutex<DaqAppInner>>` pattern
//! with non-blocking async message passing. Commands are sent via an mpsc channel,
//! and responses are returned via oneshot channels embedded in each command variant.
//!
//! # Message Flow
//!
//! ```text
//! GUI Thread                         Actor Task
//! ----------                         ----------
//! 1. Create command with oneshot
//! 2. Send via mpsc channel    ------>
//!                                    3. Receive command
//!                                    4. Process (mutate state)
//!                                    5. Send response
//! 6. Await oneshot receiver   <------
//! 7. Handle result
//! ```
//!
//! # Channel Types
//!
//! - **mpsc (Multi-Producer, Single-Consumer)**: GUI → Actor command channel
//!   - Capacity: 32 (configurable)
//!   - Non-blocking sends from GUI
//!   - Sequential processing in actor
//!
//! - **oneshot (Single-Producer, Single-Consumer)**: Actor → GUI response
//!   - One-time use per command
//!   - Type-safe responses
//!   - Zero-copy when possible
//!
//! # Helper Methods
//!
//! Each command variant has a helper method that creates the command and
//! returns the oneshot receiver:
//!
//! ```rust
//! use rust_daq::messages::DaqCommand;
//!
//! let (cmd, rx) = DaqCommand::spawn_instrument("my_instrument".to_string());
//! // cmd_tx.send(cmd).await?;
//! // let result = rx.await?;
//! ```
//!
//! This pattern ensures the GUI always gets a receiver to await the response.

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

/// Commands that can be sent to the `DaqManagerActor` (see [`crate::app_actor`]).
///
/// Each variant includes a `oneshot::Sender` for the response, implementing
/// the request-response pattern over async channels. Use the helper methods
/// like [`spawn_instrument`](Self::spawn_instrument) to create commands with
/// receivers.
#[derive(Debug)]
pub enum DaqCommand {
    /// Spawns a new instrument task from configuration.
    ///
    /// The actor will:
    /// 1. Look up instrument configuration in settings
    /// 2. Create instrument instance from registry
    /// 3. Build processor chain (if configured)
    /// 4. Connect to hardware
    /// 5. Spawn Tokio task with event loop
    ///
    /// # Response
    ///
    /// - `Ok(())`: Instrument spawned and connected successfully
    /// - `Err(SpawnError)`: Configuration invalid, connection failed, or already running
    SpawnInstrument {
        /// Instrument ID from configuration
        id: String,
        /// Response channel for spawn result
        response: oneshot::Sender<Result<(), SpawnError>>,
    },

    /// Stops a running instrument task gracefully.
    ///
    /// The actor sends `InstrumentCommand::Shutdown` to the instrument task
    /// and waits up to 5 seconds for graceful disconnect. If the task doesn't
    /// respond in time, it is aborted forcefully.
    ///
    /// # Response
    ///
    /// Always succeeds (sent after shutdown attempt completes).
    StopInstrument {
        /// Instrument ID to stop
        id: String,
        /// Response channel (acknowledges shutdown attempt)
        response: oneshot::Sender<()>,
    },

    /// Sends a command to a running instrument's event loop.
    ///
    /// Commands are forwarded to the instrument task via its mpsc channel.
    /// The instrument processes commands in its `tokio::select!` loop alongside
    /// data acquisition.
    ///
    /// # Response
    ///
    /// - `Ok(())`: Command sent successfully
    /// - `Err`: Instrument not running or channel full/closed
    SendInstrumentCommand {
        /// Target instrument ID
        id: String,
        /// Command to forward (SetParameter, Trigger, etc.)
        command: InstrumentCommand,
        /// Response channel for send result
        response: oneshot::Sender<Result<()>>,
    },

    /// Starts recording data to disk by spawning a storage writer task.
    ///
    /// The storage writer:
    /// - Subscribes to the `DataDistributor` broadcast channel
    /// - Creates a writer based on current storage format
    /// - Writes all measurements asynchronously
    /// - Continues until `StopRecording` is sent
    ///
    /// # Response
    ///
    /// - `Ok(())`: Recording started successfully
    /// - `Err`: Already recording or unsupported format
    StartRecording {
        /// Response channel for start result
        response: oneshot::Sender<Result<()>>,
    },

    /// Stops the storage writer task gracefully.
    ///
    /// The actor sends a shutdown signal and waits up to 5 seconds for the
    /// writer to flush all buffered data to disk.
    ///
    /// # Response
    ///
    /// Always succeeds (sent after shutdown attempt completes).
    StopRecording {
        /// Response channel (acknowledges shutdown attempt)
        response: oneshot::Sender<()>,
    },

    /// Saves the current application state to a session file.
    ///
    /// Session files are JSON/TOML and contain:
    /// - Active instrument IDs
    /// - Storage configuration
    /// - GUI state (window layout, plots)
    ///
    /// # Response
    ///
    /// - `Ok(())`: Session saved successfully
    /// - `Err`: File I/O error or serialization failure
    SaveSession {
        /// Path to save session file
        path: PathBuf,
        /// Current GUI state to persist
        gui_state: GuiState,
        /// Response channel for save result
        response: oneshot::Sender<Result<()>>,
    },

    /// Loads application state from a session file.
    ///
    /// The actor will:
    /// 1. Stop all currently running instruments
    /// 2. Parse session file
    /// 3. Spawn instruments from session
    /// 4. Apply storage settings
    /// 5. Return GUI state for caller to restore
    ///
    /// # Response
    ///
    /// - `Ok(GuiState)`: Session loaded, instruments spawned, GUI state returned
    /// - `Err`: File I/O error, parse error, or invalid configuration
    LoadSession {
        /// Path to session file
        path: PathBuf,
        /// Response channel for GUI state
        response: oneshot::Sender<Result<GuiState>>,
    },

    /// Gets the list of currently running instrument IDs.
    ///
    /// This is a read-only query that doesn't mutate actor state.
    ///
    /// # Response
    ///
    /// Vector of instrument IDs (empty if no instruments running).
    GetInstrumentList {
        /// Response channel for instrument list
        response: oneshot::Sender<Vec<String>>,
    },

    /// Gets the list of available channel names from the instrument registry.
    ///
    /// This returns all configured instruments, not just running ones.
    ///
    /// # Response
    ///
    /// Vector of instrument IDs from configuration.
    GetAvailableChannels {
        /// Response channel for channel list
        response: oneshot::Sender<Vec<String>>,
    },

    /// Gets the current storage format (csv, hdf5, arrow).
    ///
    /// # Response
    ///
    /// Storage format string.
    GetStorageFormat {
        /// Response channel for format string
        response: oneshot::Sender<String>,
    },

    /// Sets the storage format for future recordings.
    ///
    /// Does not affect active recordings. Takes effect on next `StartRecording`.
    ///
    /// # Response
    ///
    /// Always succeeds.
    SetStorageFormat {
        /// New format (csv, hdf5, arrow)
        format: String,
        /// Response channel (acknowledges format change)
        response: oneshot::Sender<()>,
    },

    /// Subscribes to the data broadcast channel.
    ///
    /// Returns a new receiver for the `DataDistributor` broadcast channel.
    /// Multiple subscribers can receive the same data stream independently.
    ///
    /// # Response
    ///
    /// Receiver for `Arc<Measurement>` broadcast.
    SubscribeToData {
        /// Response channel for data receiver
        response: oneshot::Sender<mpsc::Receiver<Arc<Measurement>>>,
    },

    /// Initiates graceful shutdown of the entire DAQ system.
    ///
    /// Shutdown sequence:
    /// 1. Stop recording (if active)
    /// 2. Stop all instruments (5s timeout each)
    /// 3. Actor event loop exits
    ///
    /// After sending this command, the actor will stop processing further
    /// commands.
    ///
    /// # Response
    ///
    /// Always succeeds (sent after shutdown sequence completes).
    Shutdown {
        /// Response channel (acknowledges shutdown completion)
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
