//! Device-specific control panel widgets.
//!
//! This module provides specialized control panels for different device types,
//! including lasers, power meters, rotators, stages, and analog outputs.

mod andor_panel;
mod dover_panel;
mod generic_panel;
mod maitai_panel;
pub mod parameter_widget;
mod power_meter_panel;
mod rotator_panel;
mod spectrograph_panel;
mod stage_panel;

pub use andor_panel::AndorCameraPanel;
pub use dover_panel::DoverStagePanel;
pub use generic_panel::GenericDevicePanel;
pub use maitai_panel::MaiTaiControlPanel;
pub use power_meter_panel::PowerMeterControlPanel;
pub use rotator_panel::RotatorControlPanel;
pub use spectrograph_panel::SpectrographPanel;
pub use stage_panel::StageControlPanel;

use crate::layout;
use crate::runtime::Runtime;
use egui::Ui;
use egui_extras::{Column, Size, StripBuilder, TableBuilder};
use tokio::sync::mpsc;

use client::DaqClient;
use protocol::daq::{DeviceInfo, ParameterDescriptor};

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

    /// Queue a follow-up refresh after a command completes.
    #[allow(unused_variables)]
    fn queue_refresh_if_needed(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
        device_id: &str,
    ) {
    }

    /// Return the device type this widget handles
    #[allow(unused)]
    fn device_type(&self) -> &'static str;
}

/// Resolve a runtime parameter name from current descriptors and aliases.
///
/// Resolution order:
/// 1. `preferred_name` exact match (if provided)
/// 2. alias exact matches
/// 3. `preferred_name` case-insensitive match
/// 4. alias case-insensitive matches
pub(crate) fn resolve_parameter_name(
    descriptors: &[ParameterDescriptor],
    preferred_name: Option<&str>,
    aliases: &[&str],
) -> Option<String> {
    let mut candidates: Vec<&str> = Vec::new();

    if let Some(preferred) = preferred_name
        && !preferred.is_empty()
    {
        candidates.push(preferred);
    }
    for alias in aliases {
        if !candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(alias))
        {
            candidates.push(alias);
        }
    }

    for candidate in &candidates {
        if let Some(desc) = descriptors.iter().find(|desc| desc.name == **candidate) {
            return Some(desc.name.clone());
        }
    }

    for candidate in &candidates {
        if let Some(desc) = descriptors
            .iter()
            .find(|desc| desc.name.eq_ignore_ascii_case(candidate))
        {
            return Some(desc.name.clone());
        }
    }

    None
}

/// Extract numeric bounds from top-level and metadata fields.
pub(crate) fn parameter_numeric_range(desc: &ParameterDescriptor) -> Option<(f64, f64)> {
    let min = desc
        .min_value
        .or_else(|| desc.metadata.as_ref().and_then(|m| m.min_value));
    let max = desc
        .max_value
        .or_else(|| desc.metadata.as_ref().and_then(|m| m.max_value));

    match (min, max) {
        (Some(min), Some(max)) if min <= max => Some((min, max)),
        _ => None,
    }
}

/// Collect enum options from both descriptor-level and metadata-level fields.
pub(crate) fn parameter_enum_values(desc: &ParameterDescriptor) -> Vec<String> {
    let mut values: Vec<String> = Vec::new();

    for value in &desc.enum_values {
        if !values.iter().any(|v| v.eq_ignore_ascii_case(value)) {
            values.push(value.clone());
        }
    }

    if let Some(meta) = desc.metadata.as_ref() {
        for value in &meta.enum_values {
            if !values.iter().any(|v| v.eq_ignore_ascii_case(value)) {
                values.push(value.clone());
            }
        }
    }

    values
}

pub(crate) fn scoped_widget_id(device_id: &str, key: &str) -> String {
    format!("{device_id}::{key}")
}

const ACTION_BUTTON_MIN_WIDTH: f32 = 88.0;
const ACTION_BUTTON_MIN_HEIGHT: f32 = 28.0;

pub(crate) fn panel_value_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).monospace().strong()
}

pub(crate) fn panel_hint_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).weak()
}

pub(crate) fn action_button(text: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(text.into()).min_size(egui::vec2(
        ACTION_BUTTON_MIN_WIDTH,
        ACTION_BUTTON_MIN_HEIGHT,
    ))
}

pub(crate) fn filled_action_button(
    text: impl Into<String>,
    fill: egui::Color32,
) -> egui::Button<'static> {
    action_button(
        egui::RichText::new(text.into())
            .color(egui::Color32::WHITE)
            .strong(),
    )
    .fill(fill)
}

pub(crate) fn device_info_rows(
    device: &DeviceInfo,
    extra_rows: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let mut rows = vec![
        ("Device ID".to_string(), device.id.clone()),
        ("Driver".to_string(), device.driver_type.clone()),
        ("Name".to_string(), device.name.clone()),
    ];
    rows.extend(extra_rows);
    rows
}

pub(crate) fn show_panel_header(
    ui: &mut Ui,
    title: &str,
    badge: Option<(&str, egui::Color32)>,
    is_busy: bool,
    is_refreshing: bool,
) {
    ui.horizontal(|ui| {
        ui.heading(title);
        if let Some((badge_text, badge_color)) = badge {
            ui.colored_label(badge_color, badge_text);
        }
        if is_busy {
            ui.spinner();
            ui.label(egui::RichText::new("Applying changes").weak());
        } else if is_refreshing {
            ui.spinner();
            ui.label(egui::RichText::new("Refreshing").weak());
        }
    });
}

pub(crate) fn show_panel_section<R>(
    ui: &mut Ui,
    title: &str,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let mut result = None;
    layout::section_frame(ui).show(ui, |ui| {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(6.0);
        result = Some(add_contents(ui));
    });
    result.expect("panel section should produce a result")
}

pub(crate) fn show_panel_columns_with_state<T>(
    ui: &mut Ui,
    state: &mut T,
    left: impl FnOnce(&mut Ui, &mut T),
    right: impl FnOnce(&mut Ui, &mut T),
) {
    if ui.available_width() < 520.0 {
        left(ui, state);
        ui.add_space(layout::SECTION_SPACING / 2.0);
        right(ui, state);
        return;
    }

    let mut left = Some(left);
    let mut right = Some(right);
    StripBuilder::new(ui)
        .size(Size::remainder())
        .size(Size::exact(layout::SECTION_SPACING))
        .size(Size::remainder())
        .horizontal(|mut strip| {
            strip.cell(|ui| {
                left.take().expect("left panel column should render once")(ui, state);
            });
            strip.cell(|ui| {
                ui.add_space(0.0);
            });
            strip.cell(|ui| {
                right.take().expect("right panel column should render once")(ui, state);
            });
        });
}

pub(crate) fn show_key_value_grid(ui: &mut Ui, grid_id: String, rows: &[(String, String)]) {
    TableBuilder::new(ui)
        .id_salt(grid_id)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(160.0).at_least(120.0).clip(true))
        .column(Column::remainder().clip(true))
        .body(|mut body| {
            for (label, value) in rows {
                body.row(24.0, |mut row| {
                    row.col(|ui| {
                        ui.label(panel_hint_text(label));
                    });
                    row.col(|ui| {
                        ui.label(value);
                    });
                });
            }
        });
}

pub(crate) fn show_device_info_section(ui: &mut Ui, table_id: String, rows: &[(String, String)]) {
    show_panel_section(ui, "Device Info", |ui| {
        show_key_value_grid(ui, table_id, rows);
    });
}

pub(crate) fn request_panel_repaint(ui: &Ui, active: bool) {
    if active {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

pub(crate) fn show_panel_messages(ui: &mut Ui, error: Option<&str>, status: Option<&str>) {
    if let Some(err) = error {
        ui.colored_label(layout::colors::ERROR, err);
    }
    if let Some(status) = status {
        ui.colored_label(layout::colors::SUCCESS, status);
    }
}

pub(crate) fn parse_f64_input(input: &str, field_name: &str) -> Result<f64, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }

    trimmed
        .parse::<f64>()
        .map_err(|_| format!("Invalid {field_name}: expected a number"))
}

pub(crate) fn parse_positive_f64_input(input: &str, field_name: &str) -> Result<f64, String> {
    let value = parse_f64_input(input, field_name)?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!(
            "Invalid {field_name}: must be a positive finite number"
        ));
    }
    Ok(value)
}

pub(crate) fn parse_nonnegative_i64_input(input: &str, field_name: &str) -> Result<i64, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }

    let value = trimmed
        .parse::<i64>()
        .map_err(|_| format!("Invalid {field_name}: expected an integer"))?;
    if value < 0 {
        return Err(format!("Invalid {field_name}: must be non-negative"));
    }
    Ok(value)
}

pub(crate) fn parse_positive_step_input(input: &str, field_name: &str) -> Result<f64, String> {
    parse_positive_f64_input(input, field_name)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LatestRequestTracker {
    next_request_id: u64,
    latest_issued_request_id: u64,
}

impl LatestRequestTracker {
    pub(crate) fn issue(&mut self) -> u64 {
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.latest_issued_request_id = self.next_request_id;
        self.latest_issued_request_id
    }

    pub(crate) fn is_current(&self, request_id: u64) -> bool {
        request_id != 0 && request_id == self.latest_issued_request_id
    }
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
    /// Number of background refresh operations in flight
    pub background_tasks_in_flight: usize,
    /// Whether a follow-up refresh should be triggered after the current command completes
    pub refresh_after_command: bool,
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
    pub last_refresh: Option<crate::time::Instant>,
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
            background_tasks_in_flight: 0,
            refresh_after_command: false,
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
            && self.background_tasks_in_flight == 0
            && self
                .last_refresh
                .map(|t| t.elapsed() >= interval)
                .unwrap_or(true)
    }

    /// Mark the current time as the last refresh timestamp.
    ///
    /// Call this after initiating a refresh action to reset the interval timer.
    pub fn mark_refreshed(&mut self) {
        self.last_refresh = Some(crate::time::Instant::now());
    }

    /// Record that a background refresh has started.
    pub fn record_background_task_start(&mut self) {
        self.mark_refreshed();
        self.background_task_started();
    }

    /// Decrement the in-flight action counter (saturating at 0).
    ///
    /// Call this when an async action completes (success or failure).
    pub fn action_completed(&mut self) {
        self.actions_in_flight = self.actions_in_flight.saturating_sub(1);
    }

    /// Increment the background refresh counter.
    pub fn background_task_started(&mut self) {
        self.background_tasks_in_flight += 1;
    }

    /// Decrement the background refresh counter (saturating at 0).
    pub fn background_task_completed(&mut self) {
        self.background_tasks_in_flight = self.background_tasks_in_flight.saturating_sub(1);
    }

    /// Check whether a background refresh is still in flight.
    pub fn is_refreshing(&self) -> bool {
        self.background_tasks_in_flight > 0
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

    /// Clear the current error message.
    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// Request a follow-up refresh after the active command completes.
    pub fn request_refresh_after_command(&mut self) {
        self.refresh_after_command = true;
    }

    /// Consume the follow-up refresh request flag.
    pub fn consume_refresh_after_command(&mut self) -> bool {
        std::mem::take(&mut self.refresh_after_command)
    }

    /// Render the current status and error messages.
    pub fn render_status_and_errors(&self, ui: &mut Ui) {
        show_panel_messages(ui, self.error.as_deref(), self.status.as_deref());
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
        _Read(Result<f64, String>),
        _Write(Result<(), String>),
    }

    #[test]
    fn test_device_panel_state_new() {
        let state: DevicePanelState<TestAction> = DevicePanelState::new();
        assert_eq!(state.actions_in_flight, 0);
        assert_eq!(state.background_tasks_in_flight, 0);
        assert!(!state.refresh_after_command);
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
        assert_eq!(state.background_tasks_in_flight, 0);
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
    fn test_should_refresh_blocks_when_refresh_in_flight() {
        let mut state: DevicePanelState<TestAction> = DevicePanelState::new();
        state.background_task_started();
        assert!(!state.should_refresh(Duration::from_secs(1)));

        state.background_task_completed();
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
