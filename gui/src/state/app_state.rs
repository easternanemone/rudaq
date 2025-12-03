//! Application state structures
//!
//! Contains the main AppState and related types for managing application state.

use crate::services::DaqClient;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Global toast ID counter for unique toast identifiers
pub static TOAST_ID_COUNTER: AtomicI32 = AtomicI32::new(1);

/// Type alias for the shared state wrapped in Arc<Mutex<>>
pub type SharedState = Arc<Mutex<AppState>>;

/// Per-device streaming state
#[expect(
    dead_code,
    reason = "is_streaming used for future per-device stream management"
)]
pub struct DeviceStreamState {
    pub stream_handle: Option<tokio::task::JoinHandle<()>>,
    pub is_streaming: bool,
}

impl DeviceStreamState {
    pub fn new() -> Self {
        Self {
            stream_handle: None,
            is_streaming: false,
        }
    }
}

impl Default for DeviceStreamState {
    fn default() -> Self {
        Self::new()
    }
}

/// Application state shared between UI and background tasks
pub struct AppState {
    /// gRPC client connection
    pub client: Option<DaqClient>,

    /// Handle to the device state subscription stream
    pub state_stream_handle: Option<tokio::task::JoinHandle<()>>,

    /// Set of selected device IDs (multi-select)
    pub selected_device_ids: HashSet<String>,

    /// Per-device stream state (for readers)
    pub device_streams: HashMap<String, DeviceStreamState>,

    /// Per-device position state (for movables)
    #[expect(dead_code, reason = "Used by rebuild_selected_panels (future use)")]
    pub device_positions: HashMap<String, f64>,

    #[expect(dead_code, reason = "Used by rebuild_selected_panels (future use)")]
    pub device_moving: HashMap<String, bool>,

    /// Current scan ID (if a scan is active)
    pub current_scan_id: Option<String>,

    /// Handle to the scan progress stream task
    pub scan_progress_handle: Option<tokio::task::JoinHandle<()>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            client: None,
            state_stream_handle: None,
            selected_device_ids: HashSet::new(),
            device_streams: HashMap::new(),
            device_positions: HashMap::new(),
            device_moving: HashMap::new(),
            current_scan_id: None,
            scan_progress_handle: None,
        }
    }

    /// Get a clone of the client if connected
    pub fn get_client(&self) -> Option<DaqClient> {
        self.client.clone()
    }

    /// Check if connected to the daemon
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    /// Clear all state on disconnect
    pub fn clear_on_disconnect(&mut self) {
        // Abort state stream
        if let Some(handle) = self.state_stream_handle.take() {
            handle.abort();
        }

        // Abort all device streams
        for (_, stream_state) in self.device_streams.iter_mut() {
            if let Some(handle) = stream_state.stream_handle.take() {
                handle.abort();
            }
        }
        self.device_streams.clear();

        // Abort scan progress stream
        if let Some(handle) = self.scan_progress_handle.take() {
            handle.abort();
        }

        // Clear selection and state
        self.selected_device_ids.clear();
        self.device_positions.clear();
        self.device_moving.clear();
        self.current_scan_id = None;
        self.client = None;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate the next unique toast ID
pub fn next_toast_id() -> i32 {
    TOAST_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}
