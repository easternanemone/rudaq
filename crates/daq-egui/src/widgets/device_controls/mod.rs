//! Device-specific control panel widgets.
//!
//! This module provides specialized control panels for different device types,
//! including lasers, power meters, rotators, stages, and analog outputs.

mod analog_output_panel;
mod maitai_panel;
mod power_meter_panel;
mod rotator_panel;
mod stage_panel;

pub use analog_output_panel::AnalogOutputControlPanel;
pub use maitai_panel::MaiTaiControlPanel;
pub use power_meter_panel::PowerMeterControlPanel;
pub use rotator_panel::RotatorControlPanel;
pub use stage_panel::StageControlPanel;

use egui::Ui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use daq_client::DaqClient;
use daq_proto::daq::DeviceInfo;

/// Trait for device-specific control panel widgets
pub trait DeviceControlWidget {
    /// Render the control panel UI
    ///
    /// # Arguments
    /// * `ui` - egui UI context
    /// * `device` - Device info from the daemon
    /// * `client` - Optional gRPC client for making requests
    /// * `runtime` - Tokio runtime for async operations
    fn ui(
        &mut self,
        ui: &mut Ui,
        device: &DeviceInfo,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    );

    /// Return the device type this widget handles
    #[allow(unused)]
    fn device_type(&self) -> &'static str;
}

/// Common state container for device control panels.
///
/// This struct encapsulates the boilerplate state that all device panels share:
/// - Async action channels for non-blocking gRPC calls
/// - In-flight action tracking for UI enable/disable logic
/// - Error and status message display
/// - Device identification
/// - Initial fetch coordination
/// - Auto-refresh timing
///
/// # Type Parameter
///
/// * `R` - The panel-specific action result enum type
///
/// # Example
///
/// ```ignore
/// enum MyPanelAction {
///     ReadValue(Result<f64, String>),
///     WriteValue(Result<(), String>),
/// }
///
/// struct MyPanel {
///     state: DevicePanelState<MyPanelAction>,
///     // ... panel-specific state ...
/// }
///
/// impl Default for MyPanel {
///     fn default() -> Self {
///         Self {
///             state: DevicePanelState::new(),
///             // ... initialize panel-specific state ...
///         }
///     }
/// }
/// ```
pub struct DevicePanelState<R> {
    /// Channel sender for async action results
    pub action_tx: mpsc::Sender<R>,
    /// Channel receiver for async action results
    pub action_rx: mpsc::Receiver<R>,
    /// Number of user-initiated actions in flight (disables controls when > 0)
    pub actions_in_flight: usize,
    /// Error message to display in UI (red text)
    pub error: Option<String>,
    /// Status message to display in UI (green text)
    pub status: Option<String>,
    /// Device ID cached from last UI render
    pub device_id: Option<String>,
    /// Whether initial state fetch has been triggered
    pub initial_fetch_done: bool,
    /// Auto-refresh enabled flag
    pub auto_refresh: bool,
    /// Last refresh timestamp for interval timing
    pub last_refresh: Option<std::time::Instant>,
}

impl<R> DevicePanelState<R> {
    /// Create a new panel state with default values.
    ///
    /// Auto-refresh is enabled by default with no initial refresh timestamp.
    /// Channel buffer size is 16 (sufficient for typical async workflows).
    pub fn new() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            action_tx,
            action_rx,
            actions_in_flight: 0,
            error: None,
            status: None,
            device_id: None,
            initial_fetch_done: false,
            auto_refresh: true,
            last_refresh: None,
        }
    }

    /// Check if a refresh should occur based on the given interval.
    ///
    /// Returns `true` if auto-refresh is enabled, no actions are in flight,
    /// and the interval has elapsed since the last refresh.
    ///
    /// # Arguments
    ///
    /// * `interval` - The refresh interval duration
    ///
    /// # Returns
    ///
    /// `true` if a refresh should be triggered, `false` otherwise
    pub fn should_refresh(&self, interval: std::time::Duration) -> bool {
        self.auto_refresh
            && self.actions_in_flight == 0
            && self
                .last_refresh
                .map(|t| t.elapsed() >= interval)
                .unwrap_or(true)
    }

    /// Mark the current time as the last refresh timestamp.
    ///
    /// Call this after initiating a refresh action to reset the interval timer.
    pub fn mark_refreshed(&mut self) {
        self.last_refresh = Some(std::time::Instant::now());
    }

    /// Decrement the in-flight action counter (saturating at 0).
    ///
    /// Call this when an async action completes (success or failure).
    pub fn action_completed(&mut self) {
        self.actions_in_flight = self.actions_in_flight.saturating_sub(1);
    }

    /// Increment the in-flight action counter.
    ///
    /// Call this when initiating a new async action.
    pub fn action_started(&mut self) {
        self.actions_in_flight += 1;
    }

    /// Check if the panel is busy (has actions in flight).
    ///
    /// Use this to disable controls during async operations.
    pub fn is_busy(&self) -> bool {
        self.actions_in_flight > 0
    }

    /// Clear error and status messages.
    pub fn clear_messages(&mut self) {
        self.error = None;
        self.status = None;
    }

    /// Set an error message (clears status).
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
        self.status = None;
    }

    /// Set a status message (clears error).
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
        self.error = None;
    }
}

impl<R> Default for DevicePanelState<R> {
    fn default() -> Self {
        Self::new()
    }
}
