//! Device-specific control panel widgets.
//!
//! This module provides specialized control panels for different device types,
//! including lasers, power meters, rotators, stages, and analog outputs.

mod generic_panel;
mod maitai_panel;
mod power_meter_panel;
mod rotator_panel;
mod stage_panel;

pub use generic_panel::GenericDevicePanel;
pub use maitai_panel::MaiTaiControlPanel;
pub use power_meter_panel::PowerMeterControlPanel;
pub use rotator_panel::RotatorControlPanel;
pub use stage_panel::StageControlPanel;

use egui::Ui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use client::DaqClient;
use protocol::daq::DeviceInfo;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Debug)]
    enum TestAction {
        Read(Result<f64, String>),
        Write(Result<(), String>),
    }

    #[test]
    fn test_device_panel_state_new() {
        let state: DevicePanelState<TestAction> = DevicePanelState::new();
        assert_eq!(state.actions_in_flight, 0);
        assert_eq!(state.error, None);
        assert_eq!(state.status, None);
        assert_eq!(state.device_id, None);
        assert!(!state.initial_fetch_done);
        assert!(state.auto_refresh);
        assert_eq!(state.last_refresh, None);
    }

    #[test]
    fn test_device_panel_state_default() {
        let state: DevicePanelState<TestAction> = DevicePanelState::default();
        assert_eq!(state.actions_in_flight, 0);
        assert!(state.auto_refresh);
    }

    #[test]
    fn test_action_started_increments_counter() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        assert_eq!(state.actions_in_flight, 0);

        state.action_started();
        assert_eq!(state.actions_in_flight, 1);

        state.action_started();
        assert_eq!(state.actions_in_flight, 2);
    }

    #[test]
    fn test_action_completed_decrements_counter() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        state.action_started();
        state.action_started();
        assert_eq!(state.actions_in_flight, 2);

        state.action_completed();
        assert_eq!(state.actions_in_flight, 1);

        state.action_completed();
        assert_eq!(state.actions_in_flight, 0);
    }

    #[test]
    fn test_action_completed_saturates_at_zero() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        assert_eq!(state.actions_in_flight, 0);

        state.action_completed();
        assert_eq!(state.actions_in_flight, 0);

        state.action_completed();
        assert_eq!(state.actions_in_flight, 0);
    }

    #[test]
    fn test_is_busy() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        assert!(!state.is_busy());

        state.action_started();
        assert!(state.is_busy());

        state.action_completed();
        assert!(!state.is_busy());
    }

    #[test]
    fn test_set_error_clears_status() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        state.set_status("All good");
        assert_eq!(state.status, Some("All good".to_string()));
        assert_eq!(state.error, None);

        state.set_error("Something went wrong");
        assert_eq!(state.error, Some("Something went wrong".to_string()));
        assert_eq!(state.status, None);
    }

    #[test]
    fn test_set_status_clears_error() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        state.set_error("Error occurred");
        assert_eq!(state.error, Some("Error occurred".to_string()));
        assert_eq!(state.status, None);

        state.set_status("Success");
        assert_eq!(state.status, Some("Success".to_string()));
        assert_eq!(state.error, None);
    }

    #[test]
    fn test_should_refresh_when_never_refreshed() {
        let state: DevicePanelState<TestAction> = DevicePanelState::new();
        assert!(state.should_refresh(Duration::from_secs(1)));
    }

    #[test]
    fn test_should_refresh_respects_auto_refresh_flag() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        state.auto_refresh = false;
        assert!(!state.should_refresh(Duration::from_secs(1)));

        state.auto_refresh = true;
        assert!(state.should_refresh(Duration::from_secs(1)));
    }

    #[test]
    fn test_should_refresh_blocks_when_busy() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        state.action_started();
        assert!(!state.should_refresh(Duration::from_secs(1)));

        state.action_completed();
        assert!(state.should_refresh(Duration::from_secs(1)));
    }

    #[test]
    fn test_should_refresh_respects_interval() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        state.mark_refreshed();

        // Immediately after refresh, should not refresh again
        assert!(!state.should_refresh(Duration::from_secs(10)));

        // After sleeping past the interval, should refresh
        std::thread::sleep(Duration::from_millis(50));
        assert!(state.should_refresh(Duration::from_millis(10)));
    }

    #[test]
    fn test_mark_refreshed_updates_timestamp() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        assert_eq!(state.last_refresh, None);

        state.mark_refreshed();
        assert!(state.last_refresh.is_some());

        let first_refresh = state.last_refresh.unwrap();
        std::thread::sleep(Duration::from_millis(10));

        state.mark_refreshed();
        let second_refresh = state.last_refresh.unwrap();

        assert!(second_refresh > first_refresh);
    }

    #[test]
    fn test_multiple_actions_in_flight() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();

        state.action_started();
        state.action_started();
        state.action_started();
        assert_eq!(state.actions_in_flight, 3);
        assert!(state.is_busy());
        assert!(!state.should_refresh(Duration::from_secs(1)));

        state.action_completed();
        assert_eq!(state.actions_in_flight, 2);
        assert!(state.is_busy());

        state.action_completed();
        state.action_completed();
        assert_eq!(state.actions_in_flight, 0);
        assert!(!state.is_busy());
    }
}
