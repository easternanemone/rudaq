//! GUI module for rust-daq.
//!
//! Architecture (validated by Codex):
//! - UI Thread: egui immediate-mode rendering, never blocks
//! - Backend Thread: tokio runtime with gRPC client, manages streams
//! - Communication: Channel-based message passing (watch + mpsc)
//!
//! # Example
//!
//! ```ignore
//! use rust_daq::gui::{channels::create_channels, backend::spawn_backend};
//!
//! let (ui_channels, backend_handle) = create_channels();
//! let _backend_thread = spawn_backend(backend_handle);
//!
//! // UI uses ui_channels for communication
//! ui_channels.send_command(BackendCommand::Connect { address: "localhost:50051".into() });
//! ```

pub mod backend;
pub mod platform;
pub mod channels;
pub mod types;
pub mod widgets;
pub mod app;

// Re-export commonly used types
pub use backend::spawn_backend;
pub use channels::{create_channels, UiChannels};
pub use types::{
    BackendCommand, BackendEvent, BackendMetrics, ConnectionStatus, DeviceInfo, DeviceState,
    DeviceStateSnapshot, ParameterDescriptor, ParameterType, PlotPoint,
};
pub use widgets::{parameter_group, parameter_widget, ParameterEditState, WidgetResult};
