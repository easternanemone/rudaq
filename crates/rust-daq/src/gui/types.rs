//! UI-specific data types (lightweight DTOs separate from proto types).
//!
//! These types are optimized for UI rendering and avoid the overhead of
//! proto serialization. The backend converts proto messages to these types
//! before sending to the UI thread.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// Device information for UI display.
#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub driver_type: String,
    pub capabilities: Vec<String>,
    pub is_connected: bool,
}

/// Current state of a device for UI display.
#[derive(Debug, Clone, Default)]
pub struct DeviceState {
    /// Device ID
    pub device_id: String,
    /// Key-value pairs of device state fields
    pub fields: HashMap<String, String>,
    /// Version for optimistic concurrency
    pub version: u64,
    /// Last update timestamp
    pub updated_at: Option<Instant>,
}

/// Snapshot of all device states.
#[derive(Debug, Clone, Default)]
pub struct DeviceStateSnapshot {
    pub devices: HashMap<String, DeviceState>,
    pub is_connected: bool,
    pub last_error: Option<String>,
}

/// Parameter data type for widget selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterType {
    Float,
    Int,
    Bool,
    String,
    Enum,
}

impl Default for ParameterType {
    fn default() -> Self {
        Self::String
    }
}

impl From<&str> for ParameterType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "float" | "f64" | "double" => Self::Float,
            "int" | "i32" | "i64" | "integer" => Self::Int,
            "bool" | "boolean" => Self::Bool,
            "enum" => Self::Enum,
            _ => Self::String,
        }
    }
}

/// Parameter descriptor for auto-generating control widgets.
#[derive(Debug, Clone, Default)]
pub struct ParameterDescriptor {
    pub device_id: String,
    pub name: String,
    pub description: String,
    pub dtype: ParameterType,
    pub units: String,
    pub readable: bool,
    pub writable: bool,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub enum_values: Vec<String>,
    /// Current value (fetched separately or from state stream)
    pub current_value: Option<String>,
}

/// A single data point for plotting.
#[derive(Debug, Clone)]
pub struct PlotPoint {
    pub device_id: String,
    pub parameter_name: String,
    pub timestamp_ms: f64,
    pub value: f64,
    pub units: String,
}

/// Backend performance metrics for UI display.
#[derive(Debug, Clone, Default)]
pub struct BackendMetrics {
    /// UI frame time in milliseconds
    pub ui_frame_ms: f32,
    /// Number of frames dropped due to channel full
    pub frames_dropped: u64,
    /// Current frame channel depth
    pub frame_channel_depth: usize,
    /// Current plot channel depth
    pub plot_channel_depth: usize,
    /// gRPC round-trip time in milliseconds
    pub grpc_rtt_ms: f32,
    /// Number of stream restarts due to errors
    pub stream_restarts: u64,
    /// Connection status
    pub is_connected: bool,
}

/// Connection status for the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
    Failed { reason: String },
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::Disconnected
    }
}

/// Commands sent from UI to backend.
#[derive(Debug, Clone)]
pub enum BackendCommand {
    /// Connect to daemon at given address
    Connect { address: String },
    /// Disconnect from daemon
    Disconnect,
    /// Refresh device list
    RefreshDevices,
    /// Fetch parameters for a device
    FetchParameters { device_id: String },
    /// Move a device to absolute position
    MoveAbsolute { device_id: String, position: f64 },
    /// Move a device by relative distance
    MoveRelative { device_id: String, distance: f64 },
    /// Read a scalar value from a device
    ReadValue { device_id: String },
    /// Set a parameter on a device
    SetParameter {
        device_id: String,
        name: String,
        value: String,
    },
    /// Start streaming device state
    StartStateStream { device_ids: Vec<String> },
    /// Stop streaming device state
    StopStateStream,
    /// Shutdown the backend
    Shutdown,
}

/// Events sent from backend to UI (for one-shot responses).
#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// Device list refreshed
    DevicesRefreshed { devices: Vec<DeviceInfo> },
    /// Parameters fetched for a device
    ParametersFetched {
        device_id: String,
        parameters: Vec<ParameterDescriptor>,
    },
    /// Value read from device
    ValueRead {
        device_id: String,
        value: f64,
        units: String,
    },
    /// Device state updated (from streaming)
    DeviceStateUpdated {
        device_id: String,
        fields: HashMap<String, String>,
        version: u64,
        is_snapshot: bool,
    },
    /// State stream started
    StateStreamStarted,
    /// State stream stopped
    StateStreamStopped,
    /// Operation failed
    Error { message: String },
    /// Connection status changed
    ConnectionChanged { status: ConnectionStatus },
    /// Image data received
    ImageReceived {
        device_id: String,
        /// Dimensions: [width, height]
        size: [usize; 2],
        /// Raw pixel data (e.g., RGB or Grayscale)
        data: Vec<u8>,
    },
}
