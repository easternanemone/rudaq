//! Automation command processing and state snapshot updates.
//!
//! Bridges the `window.daqGui` JavaScript API to `DaqApp` internals.
//! Commands are drained from the shared queue each frame, and the current
//! state is written back for JS to read via `getStatus()`.

use super::*;

#[cfg(target_arch = "wasm32")]
use crate::automation::{AutomationCommand, AutomationState, ParameterInfo};

#[cfg(target_arch = "wasm32")]
use crate::panels::image_viewer::SpectrumViewMode;

impl DaqApp {
    /// Drain the automation command queue and execute each command.
    ///
    /// Called early in `update()` so that commands take effect before rendering.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn process_automation_commands(&mut self, ctx: &egui::Context) {
        let commands: Vec<AutomationCommand> =
            self.automation_commands.borrow_mut().drain(..).collect();

        for cmd in commands {
            match cmd {
                AutomationCommand::Connect(url) => {
                    self.wasm_connection.url_input = url;
                    self.wasm_connect(ctx);
                }
                AutomationCommand::Disconnect => {
                    self.wasm_disconnect();
                    self.wasm_connection.status = "Disconnected".to_string();
                }
                AutomationCommand::SelectCamera(device_id) => {
                    if let Some(client) = self.client.as_mut() {
                        self.image_viewer_panel
                            .set_device(&device_id, client, &self.runtime);
                    }
                }
                AutomationCommand::StartStream => {
                    if let Some(device_id) = self.image_viewer_panel.device_id.clone() {
                        if let Some(client) = self.client.as_mut() {
                            self.image_viewer_panel
                                .start_stream(&device_id, client, &self.runtime);
                        }
                    }
                }
                AutomationCommand::StopStream => {
                    self.image_viewer_panel
                        .stop_stream(self.client.as_mut(), &self.runtime);
                }
                AutomationCommand::SetViewMode(mode) => {
                    self.image_viewer_panel.spectrum_view_mode = match mode.as_str() {
                        "1D" | "spectrum" | "Spectrum" => SpectrumViewMode::Spectrum,
                        "2D" | "echellogram" | "Echellogram" => SpectrumViewMode::Echellogram,
                        "split" | "Split" => SpectrumViewMode::Split,
                        _ => {
                            tracing::warn!(mode = %mode, "Unknown automation view mode");
                            continue;
                        }
                    };
                }
                AutomationCommand::SetParameter { name, value } => {
                    if let Some(device_id) = self.image_viewer_panel.device_id.clone() {
                        self.image_viewer_panel
                            .pending_param_updates
                            .push((device_id, name, value));
                    }
                }
                AutomationCommand::ActivateProfile(path) => {
                    self.image_viewer_panel.request_remote_profile_load(path);
                }
                AutomationCommand::SelectOrder(index) => {
                    self.image_viewer_panel.echelle_selected_order_plot = index;
                }
                AutomationCommand::RefreshCameras => {
                    if let Some(client) = self.client.as_mut() {
                        self.image_viewer_panel
                            .refresh_cameras(client, &self.runtime);
                    }
                }
            }
        }
    }

    /// Snapshot current GUI state into the automation state holder.
    ///
    /// Called late in `update()` so the snapshot reflects the current frame.
    /// The one-frame lag is acceptable for all automation use cases.
    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn update_automation_state(&self) {
        let parameters: Vec<ParameterInfo> = self
            .image_viewer_panel
            .camera_params
            .iter()
            .map(|p| ParameterInfo {
                name: p.descriptor.name.clone(),
                value: p.current_value.clone(),
                param_type: p.descriptor.dtype.clone(),
                read_only: !p.descriptor.writable,
            })
            .collect();

        let echelle_profile = self.image_viewer_panel.echelle_profile_cache.profile();

        let state = AutomationState {
            connected: self.client.is_some(),
            streaming: self.image_viewer_panel.subscription.is_some(),
            camera: self.image_viewer_panel.device_id.clone(),
            frame_count: self.image_viewer_panel.frame_count,
            fps: self.image_viewer_panel.fps_counter.fps(),
            width: self.image_viewer_panel.width,
            height: self.image_viewer_panel.height,
            view_mode: match self.image_viewer_panel.spectrum_view_mode {
                SpectrumViewMode::Echellogram => "2D",
                SpectrumViewMode::Spectrum => "1D",
                SpectrumViewMode::Split => "Split",
            }
            .to_string(),
            available_cameras: self.image_viewer_panel.available_cameras.clone(),
            parameters,
            echelle_profile_loaded: echelle_profile.is_some(),
            echelle_orders_count: echelle_profile.map_or(0, |p| p.orders.len()),
            echelle_selected_order: self.image_viewer_panel.echelle_selected_order_plot,
            echelle_error: self.image_viewer_panel.echelle_preview_error.clone(),
            colormap: format!("{:?}", self.image_viewer_panel.colormap),
        };

        *self.automation_state.borrow_mut() = state;
    }
}
