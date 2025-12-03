//! UI Adapter - Facade pattern for Slint UI manipulation
//!
//! This module provides a high-level API for UI updates, encapsulating
//! all VecModel instantiation and Slint-specific details. This removes
//! tight coupling between handlers and Slint internals.

use crate::ui::{
    DeviceInfo, MainWindow, ModuleEvent, ModuleInstance, ModuleParameter, ModuleRole,
    ModuleTypeInfo, PresetInfo, SelectedCamera, SelectedMovable, SelectedReadable, SharedString,
    ToastMessage, VecModel, Weak,
};
use slint::Model;
use std::rc::Rc;

/// High-level facade for UI updates
///
/// Wraps `Weak<MainWindow>` and exposes semantic operations for updating
/// the UI state. All VecModel creation and Slint-specific logic is
/// encapsulated here.
#[derive(Clone)]
pub struct UiAdapter {
    window: Weak<MainWindow>,
}

impl UiAdapter {
    /// Create a new UiAdapter wrapping a weak reference to the MainWindow
    pub fn new(window: Weak<MainWindow>) -> Self {
        Self { window }
    }

    /// Get the underlying weak reference (for handler registration)
    pub fn weak(&self) -> Weak<MainWindow> {
        self.window.clone()
    }

    // =========================================================================
    // Model Initialization
    // =========================================================================

    /// Initialize or reset all models to empty/default state
    ///
    /// Call this on startup and after disconnect to clear all UI state.
    pub fn reset_all_models(&self) {
        let _ = self.window.upgrade_in_event_loop(|ui| {
            // Device models
            ui.set_devices(Rc::new(VecModel::<DeviceInfo>::default()).into());

            // Selected panel models
            ui.set_selected_movables(Rc::new(VecModel::<SelectedMovable>::default()).into());
            ui.set_selected_readables(Rc::new(VecModel::<SelectedReadable>::default()).into());
            ui.set_selected_cameras(Rc::new(VecModel::<SelectedCamera>::default()).into());

            // Module models
            ui.set_module_types(Rc::new(VecModel::<ModuleTypeInfo>::default()).into());
            ui.set_module_instances(Rc::new(VecModel::<ModuleInstance>::default()).into());
            ui.set_current_module_roles(Rc::new(VecModel::<ModuleRole>::default()).into());
            ui.set_current_module_parameters(
                Rc::new(VecModel::<ModuleParameter>::default()).into(),
            );
            ui.set_current_module_events(Rc::new(VecModel::<ModuleEvent>::default()).into());
            ui.set_available_device_ids(Rc::new(VecModel::<SharedString>::default()).into());

            // Preset models
            ui.set_presets(Rc::new(VecModel::<PresetInfo>::default()).into());

            // Toast model
            ui.set_toasts(Rc::new(VecModel::<ToastMessage>::default()).into());
        });
    }

    /// Clear only the selected panel models (movables, readables, cameras)
    pub fn clear_selected_panels(&self) {
        let _ = self.window.upgrade_in_event_loop(|ui| {
            ui.set_selected_movables(Rc::new(VecModel::<SelectedMovable>::default()).into());
            ui.set_selected_readables(Rc::new(VecModel::<SelectedReadable>::default()).into());
            ui.set_selected_cameras(Rc::new(VecModel::<SelectedCamera>::default()).into());
        });
    }

    // =========================================================================
    // Connection State
    // =========================================================================

    /// Update connection status indicators
    pub fn set_connection_status(&self, state: &str, status: &str, error: &str) {
        let state = SharedString::from(state);
        let status = SharedString::from(status);
        let error = SharedString::from(error);

        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_connection_state(state);
            ui.set_connection_status(status);
            ui.set_connection_error(error);
        });
    }

    /// Set connection state to connected with device count
    pub fn set_connected(&self, device_count: usize) {
        let status = format!("Connected ({} devices)", device_count);

        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_connected(true);
            ui.set_connection_state(SharedString::from("connected"));
            ui.set_connection_status(SharedString::from(&status));
            ui.set_connection_error(SharedString::from(""));
        });
    }

    /// Set connection state to disconnected
    pub fn set_disconnected(&self) {
        let _ = self.window.upgrade_in_event_loop(|ui| {
            ui.set_connected(false);
            ui.set_connection_state(SharedString::from("disconnected"));
            ui.set_connection_status(SharedString::from("Disconnected"));
            ui.set_connection_error(SharedString::from(""));
        });
    }

    /// Set connection state to connecting
    pub fn set_connecting(&self) {
        let _ = self.window.upgrade_in_event_loop(|ui| {
            ui.set_connection_state(SharedString::from("connecting"));
            ui.set_connection_status(SharedString::from("Connecting..."));
            ui.set_connection_error(SharedString::from(""));
        });
    }

    /// Set connection state to error
    pub fn set_connection_error(&self, error: &str) {
        let error = SharedString::from(error);

        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.set_connected(false);
            ui.set_connection_state(SharedString::from("error"));
            ui.set_connection_status(SharedString::from("Connection Failed"));
            ui.set_connection_error(error);
        });
    }

    // =========================================================================
    // Device Management
    // =========================================================================

    /// Update the list of available devices
    pub fn update_devices(&self, devices: Vec<DeviceInfo>) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            let model = Rc::new(VecModel::from(devices));
            ui.set_devices(model.into());
        });
    }

    /// Update the list of available device IDs (for module assignment dropdowns)
    pub fn update_available_device_ids(&self, device_ids: Vec<SharedString>) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            let model = Rc::new(VecModel::from(device_ids));
            ui.set_available_device_ids(model.into());
        });
    }

    /// Update a specific device's position in the movables panel
    pub fn update_device_position(&self, device_id: String, position: f32) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            let movables = ui.get_selected_movables();
            for i in 0..movables.row_count() {
                if let Some(mut m) = movables.row_data(i) {
                    if m.device_id.as_str() == device_id {
                        m.position = position;
                        if let Some(vm) = movables
                            .as_any()
                            .downcast_ref::<VecModel<SelectedMovable>>()
                        {
                            vm.set_row_data(i, m);
                        }
                        break;
                    }
                }
            }
        });
    }

    /// Update a specific device's reading in the readables panel
    pub fn update_device_reading(&self, device_id: String, value: f32) {
        let _ = self.window.upgrade_in_event_loop(move |ui| {
            let readables = ui.get_selected_readables();
            for i in 0..readables.row_count() {
                if let Some(mut r) = readables.row_data(i) {
                    if r.device_id.as_str() == device_id {
                        r.value = value;
                        if let Some(vm) = readables
                            .as_any()
                            .downcast_ref::<VecModel<SelectedReadable>>()
                        {
                            vm.set_row_data(i, r);
                        }
                        break;
                    }
                }
            }
        });
    }

    // =========================================================================
    // Toast Notifications
    // =========================================================================

    /// Show a toast notification
    pub fn show_toast(&self, toast_type: &str, title: &str, message: &str) {
        let toast_type = SharedString::from(toast_type);
        let title = SharedString::from(title);
        let message = SharedString::from(message);

        let _ = self.window.upgrade_in_event_loop(move |ui| {
            ui.invoke_show_toast(toast_type, title, message);
        });
    }

    /// Show an error toast
    pub fn show_error(&self, title: &str, message: &str) {
        self.show_toast("error", title, message);
    }

    /// Show a success toast
    pub fn show_success(&self, title: &str, message: &str) {
        self.show_toast("success", title, message);
    }
}

#[cfg(test)]
mod tests {
    // UiAdapter tests would require a mock MainWindow, which is complex
    // due to Slint's generated code. Integration tests are more practical.
}
