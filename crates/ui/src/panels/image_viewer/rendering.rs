//! Image viewer rendering - main UI method and display logic.

use super::*;

impl ImageViewerPanel {
    /// Render the image viewer panel
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn ui(&mut self, ui: &mut egui::Ui, mut client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Poll for async action results
        self.poll_actions();
        self.poll_param_results(ui.ctx());
        self.poll_echelle_profile_cache();
        self.poll_remote_profile_load();
        self.poll_remote_profile_save();
        // Reset terminal load states to Idle — outcomes are already propagated
        // to echelle_cal_ui.status_message/last_error by poll_remote_profile_load.
        // On success, trigger immediate extraction on the last frame (bd-zy7y.3)
        // so users see whether the new profile works without waiting for the next
        // frame from the camera stream.
        if self.remote_profile_load.is_terminal() {
            let was_success = matches!(
                self.remote_profile_load,
                RemoteProfileLoadState::Succeeded { .. }
            );
            self.remote_profile_load = RemoteProfileLoadState::default();
            if was_success {
                self.try_immediate_echelle_extraction();
            }
        }
        if self.remote_profile_save.is_terminal() {
            let was_success = matches!(
                self.remote_profile_save,
                RemoteProfileSaveState::Succeeded { .. }
            );
            self.remote_profile_save = RemoteProfileSaveState::default();
            if was_success {
                self.try_immediate_echelle_extraction();
            }
        }
        self.sync_echelle_profile_to_run_engine(client.as_deref_mut(), runtime);

        // Drain pending frame updates
        self.drain_updates(ui.ctx());

        // Request continuous repaint while streaming
        if self.subscription.is_some() {
            ui.ctx().request_repaint();
        }

        // bd-12qt + bd-7rk0: Auto-reconnect logic with exponential backoff
        // Pattern inspired by Rerun's well-tested gRPC implementation:
        // - Initial delay: 100ms
        // - Max delay: 10 seconds
        // - Backoff factor: 2x per retry
        let mut should_auto_reconnect = false;
        if self.auto_reconnect
            && self.connection_state == ConnectionState::Disconnected
            && self.device_id.is_some()
            && self.subscription.is_none()
        {
            // Exponential backoff: 100ms * 2^retry_count, capped at 10 seconds
            let backoff_ms = (100u64 * 2u64.pow(self.retry_count.min(7))).min(10_000);
            if let Some(last_disconnect) = self.last_disconnect {
                if last_disconnect.elapsed().as_millis() as u64 >= backoff_ms {
                    should_auto_reconnect = true;
                    tracing::debug!(
                        retry_count = self.retry_count,
                        backoff_ms = backoff_ms,
                        "Auto-reconnecting with exponential backoff"
                    );
                }
            }
        }

        // Auto-refresh camera list on first load or if stale
        let should_refresh = self.last_refresh.is_none_or(|t| t.elapsed().as_secs() > 30);

        // Track actions to take after UI rendering (avoid borrow issues)
        let mut start_stream_device: Option<String> = None;
        let mut stop_stream = false;
        let mut refresh_cameras = false;
        let mut start_recording = false;
        let mut stop_recording = false;

        // Header with connection state indicator
        ui.horizontal(|ui| {
            // Connection state indicator (colored dot)
            let (state_color, state_text) = match self.connection_state {
                ConnectionState::Idle => (colors::MUTED, ""),
                ConnectionState::Connected => (colors::CONNECTED, ""),
                ConnectionState::Disconnected => (colors::ERROR, ""),
                ConnectionState::Reconnecting => (colors::CONNECTING, ""),
            };
            if self.connection_state != ConnectionState::Idle {
                ui.colored_label(state_color, "●");
            }

            ui.heading("Image Viewer");

            if !state_text.is_empty() {
                ui.weak(state_text);
            }
        });

        ui.add_space(layout::SECTION_SPACING / 2.0);

        // Main toolbar in card frame
        layout::card_frame(ui).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = layout::ITEM_SPACING;

                // === Camera Selection Group ===
                ui.label(format!("{} Camera:", icons::device::CAMERA));

                let selected_text = self
                    .device_id
                    .clone()
                    .unwrap_or_else(|| "Select...".to_string());

                egui::ComboBox::from_id_salt("camera_selector")
                    .selected_text(&selected_text)
                    .show_ui(ui, |ui| {
                        if self.available_cameras.is_empty() {
                            ui.label("No cameras found");
                        } else {
                            for cam_id in &self.available_cameras.clone() {
                                let is_selected = self.device_id.as_ref() == Some(cam_id);
                                if ui.selectable_label(is_selected, cam_id).clicked()
                                    && self.device_id.as_deref() != Some(cam_id.as_str())
                                {
                                    self.device_id = Some(cam_id.clone());
                                    self.mark_echelle_run_engine_sync_dirty();
                                    self.camera_params.clear();
                                }
                            }
                        }
                    });

                if ui
                    .button(icons::action::REFRESH)
                    .on_hover_text("Refresh camera list")
                    .clicked()
                {
                    refresh_cameras = true;
                }

                // Auto-load parameters if needed
                if let Some(device_id) = &self.device_id {
                    if self.camera_params.is_empty() && self.loading_params_device.is_none() {
                        let device_id_clone = device_id.clone();
                        if let Some(client) = client.as_deref_mut() {
                            self.load_camera_params(client, runtime, &device_id_clone);
                        }
                    }
                }

                ui.separator();

                // === Stream Controls Group ===
                let is_streaming = self.subscription.is_some();
                if is_streaming {
                    if ui
                        .button(format!("{} Stop", icons::action::STOP))
                        .on_hover_text("Stop streaming")
                        .clicked()
                    {
                        stop_stream = true;
                    }
                } else if self.device_id.is_some()
                    && ui
                        .button(format!("{} Start", icons::action::START))
                        .on_hover_text("Start streaming")
                        .clicked()
                {
                    if let Some(device_id) = &self.device_id {
                        start_stream_device = Some(device_id.clone());
                    }
                }

                // Reconnect button when disconnected
                if self.connection_state == ConnectionState::Disconnected {
                    if ui
                        .button(format!("{} Reconnect", icons::action::REFRESH))
                        .on_hover_text("Attempt to reconnect to camera")
                        .clicked()
                    {
                        if let Some(device_id) = &self.device_id {
                            start_stream_device = Some(device_id.clone());
                            self.connection_state = ConnectionState::Reconnecting;
                        }
                    }
                    ui.checkbox(&mut self.auto_reconnect, "Auto")
                        .on_hover_text("Automatically attempt reconnection");
                }

                // === Recording Controls ===
                ui.separator();
                match self.recording_state {
                    RecordingState::Idle => {
                        if is_streaming
                            && ui
                                .button(icons::action::RECORD)
                                .on_hover_text("Start recording frames to HDF5")
                                .clicked()
                        {
                            start_recording = true;
                        }
                    }
                    RecordingState::Recording => {
                        // Pulsing recording indicator
                        let time = ui.ctx().input(|i| i.time);
                        #[allow(clippy::cast_possible_truncation)]
                        let pulse = ((time * 2.0).sin() * 0.5 + 0.5) as f32;
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let record_color = egui::Color32::from_rgb(
                            (200.0 + pulse * 55.0) as u8,
                            (20.0 + pulse * 20.0) as u8,
                            (20.0 + pulse * 20.0) as u8,
                        );

                        if ui
                            .add(
                                egui::Button::new(format!("{} Stop", icons::action::STOP))
                                    .fill(record_color),
                            )
                            .on_hover_text("Stop recording")
                            .clicked()
                        {
                            stop_recording = true;
                        }

                        // Pulsing recording dot
                        ui.colored_label(record_color, icons::action::RECORD);
                        if let Some(status) = &self.recording_status {
                            ui.monospace(format!("{} frames", status.samples_recorded));
                        }

                        // Request repaint for animation
                        ui.ctx().request_repaint();
                    }
                    RecordingState::Starting => {
                        ui.add_enabled(false, egui::Button::new("Starting..."));
                        ui.spinner();
                    }
                    RecordingState::Stopping => {
                        ui.add_enabled(false, egui::Button::new("Stopping..."));
                        ui.spinner();
                    }
                }
            });
        });

        ui.add_space(layout::SECTION_SPACING / 2.0);

        // Display controls toolbar
        layout::card_frame(ui).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = layout::ITEM_SPACING;

                // Stream quality selector (server-side downsampling)
                egui::ComboBox::from_id_salt("stream_quality")
                    .selected_text(stream_quality_label(self.stream_quality))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.stream_quality, StreamQuality::Full, "Full");
                        ui.selectable_value(
                            &mut self.stream_quality,
                            StreamQuality::Preview,
                            "Preview (2x)",
                        );
                        ui.selectable_value(
                            &mut self.stream_quality,
                            StreamQuality::Fast,
                            "Fast (4x)",
                        );
                    });

                ui.separator();

                // === Colormap & Scale ===
                ui.label("Color:");
                egui::ComboBox::from_id_salt("colormap_selector")
                    .width(80.0)
                    .selected_text(self.colormap.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.colormap, Colormap::Grayscale, "Grayscale");
                        ui.selectable_value(&mut self.colormap, Colormap::Viridis, "Viridis");
                        ui.selectable_value(&mut self.colormap, Colormap::Inferno, "Inferno");
                        ui.selectable_value(&mut self.colormap, Colormap::Plasma, "Plasma");
                        ui.selectable_value(&mut self.colormap, Colormap::Magma, "Magma");
                    });

                egui::ComboBox::from_id_salt("scale_mode")
                    .width(60.0)
                    .selected_text(self.scale_mode.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.scale_mode, ScaleMode::Linear, "Linear");
                        ui.selectable_value(&mut self.scale_mode, ScaleMode::Log, "Log");
                        ui.selectable_value(&mut self.scale_mode, ScaleMode::Sqrt, "Sqrt");
                    });

                // bd-07j1: Colorbar toggle
                if ui
                    .selectable_label(
                        self.show_colorbar,
                        if self.show_colorbar {
                            "Bar [ON]"
                        } else {
                            "Bar"
                        },
                    )
                    .on_hover_text("Show interactive colorbar")
                    .clicked()
                {
                    self.show_colorbar = !self.show_colorbar;
                }

                ui.separator();

                // === Contrast Enhancement (bd-j6xm) ===
                ui.label("Contrast:");
                egui::ComboBox::from_id_salt("contrast_mode_selector")
                    .width(100.0)
                    .selected_text(self.contrast_mode.label())
                    .show_ui(ui, |ui| {
                        for &mode in ContrastMode::all() {
                            ui.selectable_value(&mut self.contrast_mode, mode, mode.label());
                        }
                    });

                // Show controls based on mode
                match self.contrast_mode {
                    ContrastMode::Manual => {
                        ui.add(
                            egui::DragValue::new(&mut self.display_min)
                                .speed(0.01)
                                .range(0.0..=1.0)
                                .prefix("Min: ")
                                .max_decimals(2),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.display_max)
                                .speed(0.01)
                                .range(0.0..=1.0)
                                .prefix("Max: ")
                                .max_decimals(2),
                        );
                    }
                    ContrastMode::AutoPercentile => {
                        // Show percentile controls
                        ui.add(
                            egui::DragValue::new(&mut self.percentile_low)
                                .speed(0.1)
                                .range(0.0..=100.0)
                                .prefix("Low: ")
                                .suffix("%")
                                .max_decimals(1),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.percentile_high)
                                .speed(0.1)
                                .range(0.0..=100.0)
                                .prefix("High: ")
                                .suffix("%")
                                .max_decimals(1),
                        );
                    }
                    ContrastMode::AutoSimple | ContrastMode::HistogramEq | ContrastMode::Clahe => {
                        // Show computed min/max from last frame
                        ui.weak(format!(
                            "{:.0}%-{:.0}%",
                            self.display_min * 100.0,
                            self.display_max * 100.0
                        ));
                    }
                }

                ui.separator();

                // === Zoom Controls with Icons ===
                if ui
                    .button(icons::action::FIT)
                    .on_hover_text("Fit to window")
                    .clicked()
                {
                    self.auto_fit = true;
                }
                if ui
                    .button(icons::action::ZOOM_OUT)
                    .on_hover_text("Zoom out")
                    .clicked()
                {
                    self.zoom = (self.zoom * 0.8).max(0.1);
                    self.auto_fit = false;
                }
                ui.monospace(format!("{:>3.0}%", self.zoom * 100.0));
                if ui
                    .button(icons::action::ZOOM_IN)
                    .on_hover_text("Zoom in")
                    .clicked()
                {
                    self.zoom = (self.zoom * 1.25).min(10.0);
                    self.auto_fit = false;
                }

                ui.separator();

                // === ROI & Panel Controls ===
                let roi_selected = self.roi_selector.selection_mode;
                if ui
                    .selectable_label(roi_selected, if roi_selected { "ROI [ON]" } else { "ROI" })
                    .on_hover_text("Toggle ROI selection mode")
                    .clicked()
                {
                    self.roi_selector.selection_mode = !self.roi_selector.selection_mode;
                    if self.roi_selector.selection_mode {
                        self.measurement_tool = MeasurementTool::None;
                        self.clear_measurement_interaction_state();
                    }
                }

                // ROI mode selector (Rectangle/Polygon)
                use crate::widgets::roi_selector::RoiMode;
                if roi_selected {
                    let mode_label = match self.roi_selector.mode {
                        RoiMode::Rectangle => "□",
                        RoiMode::Polygon => "⬡",
                    };
                    if ui
                        .button(mode_label)
                        .on_hover_text("Switch ROI mode (Rectangle/Polygon)")
                        .clicked()
                    {
                        self.roi_selector.mode = match self.roi_selector.mode {
                            RoiMode::Rectangle => RoiMode::Polygon,
                            RoiMode::Polygon => RoiMode::Rectangle,
                        };
                    }
                }

                if self.roi_selector.roi().is_some()
                    && ui
                        .button(icons::action::DELETE)
                        .on_hover_text("Clear ROI")
                        .clicked()
                {
                    self.roi_selector.clear();
                }

                if !self.roi_selector.rois().is_empty()
                    && ui
                        .button("Clear All")
                        .on_hover_text("Clear all ROIs")
                        .clicked()
                {
                    self.roi_selector.clear_all();
                }

                if ui
                    .add_enabled(self.device_id.is_some(), egui::Button::new("Clear HW ROI"))
                    .on_hover_text(
                        "Reset camera acquisition ROI to full sensor (requires stream stopped)",
                    )
                    .clicked()
                {
                    self.queue_clear_hardware_roi();
                }

                ui.separator();

                let line_tool_selected = self.measurement_tool == MeasurementTool::Line;
                if ui
                    .selectable_label(
                        line_tool_selected,
                        if line_tool_selected {
                            "Line [ON]"
                        } else {
                            "Line"
                        },
                    )
                    .on_hover_text("Measure line distance by click-dragging on the image")
                    .clicked()
                {
                    self.measurement_tool = if line_tool_selected {
                        MeasurementTool::None
                    } else {
                        MeasurementTool::Line
                    };
                    self.roi_selector.selection_mode = false;
                    self.clear_measurement_interaction_state();
                }

                let angle_tool_selected = self.measurement_tool == MeasurementTool::Angle;
                if ui
                    .selectable_label(
                        angle_tool_selected,
                        if angle_tool_selected {
                            "Angle [ON]"
                        } else {
                            "Angle"
                        },
                    )
                    .on_hover_text("Measure an angle by clicking three points: arm, vertex, arm")
                    .clicked()
                {
                    self.measurement_tool = if angle_tool_selected {
                        MeasurementTool::None
                    } else {
                        MeasurementTool::Angle
                    };
                    self.roi_selector.selection_mode = false;
                    self.clear_measurement_interaction_state();
                }

                if self.has_measurements()
                    && ui
                        .button("Clear Measurements")
                        .on_hover_text("Remove all saved line and angle measurements")
                        .clicked()
                {
                    self.line_measurements.clear();
                    self.angle_measurements.clear();
                    self.selected_line_measurement = None;
                    self.clear_measurement_interaction_state();
                }

                ui.separator();

                // === Crosshair Toggle (bd-pgcb) ===
                if ui
                    .selectable_label(
                        self.crosshair_enabled,
                        if self.crosshair_enabled {
                            "⊕ [ON]"
                        } else {
                            "⊕"
                        },
                    )
                    .on_hover_text("Toggle crosshair cursor\nClick to lock position")
                    .clicked()
                {
                    self.crosshair_enabled = !self.crosshair_enabled;
                    if !self.crosshair_enabled {
                        self.crosshair_locked_pos = None;
                    }
                }

                ui.separator();

                ui.checkbox(&mut self.show_roi_panel, "Stats");
                ui.checkbox(&mut self.show_pixel_stats, "Px Stats");
                ui.checkbox(&mut self.show_controls, "Controls");
                ui.checkbox(&mut self.show_metadata_overlay, "Metadata Overlay");
                ui.checkbox(&mut self.show_scale_bar, "Scale Bar");
                if self.show_scale_bar {
                    egui::ComboBox::from_id_salt("scale_bar_pos")
                        .width(110.0)
                        .selected_text(format!("Bar: {}", self.scale_bar_position.label()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.scale_bar_position,
                                ScaleBarPosition::TopLeft,
                                "Top Left",
                            );
                            ui.selectable_value(
                                &mut self.scale_bar_position,
                                ScaleBarPosition::TopRight,
                                "Top Right",
                            );
                            ui.selectable_value(
                                &mut self.scale_bar_position,
                                ScaleBarPosition::BottomLeft,
                                "Bottom Left",
                            );
                            ui.selectable_value(
                                &mut self.scale_bar_position,
                                ScaleBarPosition::BottomRight,
                                "Bottom Right",
                            );
                        });

                    egui::ComboBox::from_id_salt("scale_bar_color")
                        .width(105.0)
                        .selected_text(format!("Color: {}", self.scale_bar_color.label()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.scale_bar_color,
                                ScaleBarColor::White,
                                "White",
                            );
                            ui.selectable_value(
                                &mut self.scale_bar_color,
                                ScaleBarColor::Black,
                                "Black",
                            );
                            ui.selectable_value(
                                &mut self.scale_bar_color,
                                ScaleBarColor::Cyan,
                                "Cyan",
                            );
                            ui.selectable_value(
                                &mut self.scale_bar_color,
                                ScaleBarColor::Yellow,
                                "Yellow",
                            );
                        });

                    egui::ComboBox::from_id_salt("scale_bar_style")
                        .width(110.0)
                        .selected_text(format!("Style: {}", self.scale_bar_style.label()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.scale_bar_style,
                                ScaleBarStyle::Solid,
                                "Solid",
                            );
                            ui.selectable_value(
                                &mut self.scale_bar_style,
                                ScaleBarStyle::Outlined,
                                "Outlined",
                            );
                            ui.selectable_value(
                                &mut self.scale_bar_style,
                                ScaleBarStyle::Minimal,
                                "Minimal",
                            );
                        });
                }

                // === Histogram Position ===
                egui::ComboBox::from_id_salt("histogram_pos")
                    .width(100.0)
                    .selected_text(format!("Hist: {}", self.histogram_position.label()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::Hidden,
                            "Hidden",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::BottomRight,
                            "Bottom Right",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::BottomLeft,
                            "Bottom Left",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::TopRight,
                            "Top Right",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::TopLeft,
                            "Top Left",
                        );
                        ui.selectable_value(
                            &mut self.histogram_position,
                            HistogramPosition::SidePanel,
                            "Side Panel",
                        );
                    });
                if self.histogram_position.is_visible() {
                    ui.checkbox(&mut self.histogram.log_scale, "Log");
                }

                // === Spectrum View Mode (bd-alxb) ===
                // Show when any echelle context exists: active profile, extraction preview,
                // or editor draft (which auto-creates in WASM where filesystem isn't available).
                if self.echelle_profile_cache.profile().is_some()
                    || self.echelle_preview.is_some()
                    || self.echelle_cal_ui.editor_profile.is_some()
                {
                    ui.separator();
                    ui.label("View:");
                    ui.selectable_value(
                        &mut self.spectrum_view_mode,
                        SpectrumViewMode::Echellogram,
                        "2D",
                    )
                    .on_hover_text("2D echellogram");
                    ui.selectable_value(
                        &mut self.spectrum_view_mode,
                        SpectrumViewMode::Spectrum,
                        "1D",
                    )
                    .on_hover_text("1D spectrum (full width)");
                    ui.selectable_value(
                        &mut self.spectrum_view_mode,
                        SpectrumViewMode::Split,
                        "Split",
                    )
                    .on_hover_text("Split: 2D echellogram + 1D spectrum");
                }
            });
        });

        // Execute collected actions after UI rendering
        let client = if let Some(client_val) = client {
            // Auto-refresh on first load
            if should_refresh {
                self.refresh_cameras(client_val, runtime);
            }

            // Handle manual refresh
            if refresh_cameras {
                self.refresh_cameras(client_val, runtime);
            }

            // Handle start stream (manual or auto-reconnect)
            if let Some(device_id) = start_stream_device {
                self.start_stream(&device_id, client_val, runtime);
            } else if should_auto_reconnect {
                // bd-12qt: Auto-reconnect
                if let Some(device_id) = self.device_id.clone() {
                    self.connection_state = ConnectionState::Reconnecting;
                    self.last_disconnect = Some(Instant::now()); // Reset timer for next attempt
                    self.start_stream(&device_id, client_val, runtime);
                }
            }

            // Handle pending param updates
            let updates: Vec<_> = self.pending_param_updates.drain(..).collect();
            if !updates.is_empty() {
                tracing::debug!(count = updates.len(), "flushing pending_param_updates");
            }
            for (dev, name, val) in &updates {
                tracing::debug!(device_id = %dev, param = %name, value = %val, "flushing pending param update");
                self.set_camera_parameter(client_val, runtime, dev, name, val);
            }

            // Handle remote profile load state machine (bd-zy7y.1)
            // Only transition from Pending — do NOT use std::mem::take which would
            // drop Loading/Succeeded/Failed states and lose in-flight results.
            if let Some(path) = match &self.remote_profile_load {
                RemoteProfileLoadState::Pending { path } => Some(path.clone()),
                _ => None,
            } {
                let mut client_clone = client_val.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                let path_clone = path.clone();
                self.remote_profile_load = RemoteProfileLoadState::Loading { path, rx };
                runtime.spawn(async move {
                    match client_clone.load_calibration_profile(&path_clone).await {
                        Ok(resp) if resp.success => {
                            let _ = tx.send(Ok(resp.content));
                        }
                        Ok(resp) => {
                            let _ = tx.send(Err(resp.error_message));
                        }
                        Err(e) => {
                            let _ = tx.send(Err(format!("gRPC error: {e}")));
                        }
                    }
                });
            }

            // Remote profile save (WASM: SaveCalibrationProfile, bd-qyhh).
            if let Some((path, content, activate_after, profile)) = match &self.remote_profile_save
            {
                RemoteProfileSaveState::Pending {
                    path,
                    content,
                    activate_after,
                    profile,
                } => Some((
                    path.clone(),
                    content.clone(),
                    *activate_after,
                    profile.clone(),
                )),
                _ => None,
            } {
                let mut client_clone = client_val.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                let path_clone = path.clone();
                self.remote_profile_save = RemoteProfileSaveState::Loading {
                    path: path.clone(),
                    activate_after,
                    profile,
                    rx,
                };
                runtime.spawn(async move {
                    match client_clone
                        .save_calibration_profile(&path_clone, &content)
                        .await
                    {
                        Ok(resp) if resp.success => {
                            let _ = tx.send(Ok(()));
                        }
                        Ok(resp) => {
                            let _ = tx.send(Err(resp.error_message));
                        }
                        Err(e) => {
                            let _ = tx.send(Err(format!("gRPC error: {e}")));
                        }
                    }
                });
            }

            Some(client_val)
        } else {
            // bd-aruo.4: Show per-parameter error when updates are dropped
            if !self.pending_param_updates.is_empty() {
                tracing::warn!(
                    count = self.pending_param_updates.len(),
                    "dropping pending_param_updates — no gRPC client connected"
                );
                for (dev, name, _val) in &self.pending_param_updates {
                    self.param_errors.insert(
                        (dev.clone(), name.clone()),
                        "Not connected — change not applied".to_string(),
                    );
                }
            }
            self.pending_param_updates.clear();
            #[cfg(target_arch = "wasm32")]
            if let RemoteProfileSaveState::Pending { path, .. } = &self.remote_profile_save {
                let path = path.clone();
                let error = "Not connected to daemon — cannot save calibration profile remotely"
                    .to_string();
                self.echelle_cal_ui.last_error = Some(error.clone());
                self.remote_profile_save = RemoteProfileSaveState::Failed { path, error };
            }
            None
        };

        // Handle stop stream and recording actions
        if let Some(client) = client {
            if stop_stream {
                self.stop_stream(Some(client), runtime);
            } else {
                // Handle recording actions (bd-3pdi.5.3)
                if start_recording {
                    self.start_recording(client, runtime);
                }
                if stop_recording {
                    self.stop_recording(client, runtime);
                }
                // Poll recording status while recording
                if matches!(self.recording_state, RecordingState::Recording) {
                    let should_poll = self
                        .last_recording_poll
                        .is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(500));
                    if should_poll {
                        self.poll_recording_status(client, runtime);
                    }
                }
            }
        } else if stop_stream {
            self.stop_stream(None, runtime);
        }

        ui.add_space(layout::SECTION_SPACING / 2.0);

        // Status bar with frame info
        ui.horizontal(|ui| {
            if self.width > 0 {
                ui.monospace(format!(
                    "{}x{} @ {}bit",
                    self.width, self.height, self.bit_depth
                ));
                ui.separator();
                ui.monospace(format!("Frame: {}", self.frame_count));
                ui.separator();
                ui.monospace(format!("{:.1} FPS", self.fps_counter.fps()));

                if let Some(ref metrics) = self.stream_metrics {
                    ui.separator();
                    ui.weak(format!("{:.1}ms latency", metrics.avg_latency_ms));
                    if metrics.frames_dropped > 0 {
                        ui.separator();
                        ui.colored_label(
                            colors::WARNING,
                            format!("{} dropped", metrics.frames_dropped),
                        );
                    }
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(err) = &self.error {
                    ui.colored_label(colors::ERROR, format!("{} {}", icons::status::ERROR, err));
                }
                if let Some(status) = &self.status {
                    ui.colored_label(
                        colors::WARNING,
                        format!("{} {}", icons::status::WARNING, status),
                    );
                }
            });
        });

        ui.add_space(layout::SECTION_SPACING / 2.0);

        // Image display area with optional statistics panel
        // Calculate side panel width based on what's visible
        let has_roi_panel = self.show_roi_panel && self.roi_selector.roi().is_some();
        let has_histogram_panel = matches!(self.histogram_position, HistogramPosition::SidePanel);
        let has_controls_panel = self.show_controls && !self.camera_params.is_empty();
        let has_pixel_stats = self.show_pixel_stats;
        let has_measurements_panel =
            self.measurement_tool != MeasurementTool::None || self.has_measurements();
        // Always show the echelle panel so the calibration workspace can be used
        // to create/load the first profile before any preview exists.
        let has_echelle_panel = true;

        let has_side_panel = has_roi_panel
            || has_histogram_panel
            || has_controls_panel
            || has_echelle_panel
            || has_pixel_stats
            || has_measurements_panel;

        let side_panel_default_width = if has_controls_panel || has_echelle_panel {
            380.0
        } else {
            220.0
        };

        // Side panel for stats/controls (resizable, drawn first so remainder goes to image)
        if has_side_panel {
            egui::SidePanel::right("image_viewer_stats_panel")
                .default_width(side_panel_default_width)
                .width_range(200.0..=600.0)
                .resizable(true)
                .show_inside(ui, |ui| {
                    self.render_stats_side_panel(
                        ui,
                        has_controls_panel,
                        has_roi_panel,
                        has_histogram_panel,
                        has_echelle_panel,
                        has_pixel_stats,
                        has_measurements_panel,
                    );
                });
        }

        // Image area gets all remaining space via CentralPanel
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                // bd-alxb: Spectrum-only mode — full-width spectrum plot, skip image
                if self.spectrum_view_mode == SpectrumViewMode::Spectrum {
                    self.render_spectrum_plot_area(ui, true);
                    return;
                }

                // bd-alxb/bd-zy7y.5: Split mode — reserve bottom 30% for spectrum plot.
                // Use explicit height allocation instead of TopBottomPanel::bottom
                // which doesn't reliably constrain inside nested show_inside.
                if self.spectrum_view_mode == SpectrumViewMode::Split {
                    let total = ui.available_height();
                    let spectrum_h = (total * 0.3).clamp(100.0, 300.0);
                    egui::TopBottomPanel::bottom("spectrum_split_panel")
                        .exact_height(spectrum_h)
                        .show_inside(ui, |ui| {
                            self.render_spectrum_plot_area(ui, true);
                        });
                }

                // Remaining space: 2D echellogram (Echellogram mode uses full area,
                // Split mode uses whatever's left after the bottom panel)
                let available_size = ui.available_size();

                if let Some(texture) = &self.texture {
                    // bd-07j1: Reserve space for colorbar if enabled
                    let colorbar_width = if self.show_colorbar { 60.0 } else { 0.0 };
                    let image_available =
                        egui::vec2(available_size.x - colorbar_width, available_size.y);

                    // Calculate fit zoom if needed - continuously fit when auto_fit is enabled
                    if self.auto_fit && self.width > 0 && self.height > 0 {
                        #[allow(clippy::cast_precision_loss)]
                        let scale_x = image_available.x / self.width as f32;
                        #[allow(clippy::cast_precision_loss)]
                        let scale_y = image_available.y / self.height as f32;
                        // Allow upscaling to fill available space (remove .min(1.0) cap)
                        self.zoom = scale_x.min(scale_y);
                        self.pan = egui::Vec2::ZERO;
                        // Keep auto_fit true for continuous fitting as window resizes
                    }

                    #[allow(clippy::cast_precision_loss)]
                    let image_size = egui::vec2(
                        self.width as f32 * self.zoom,
                        self.height as f32 * self.zoom,
                    );

                    // Extract crosshair state for use in closure (bd-pgcb)
                    let crosshair_enabled = self.crosshair_enabled;
                    let crosshair_locked_pos = self.crosshair_locked_pos;
                    let width = self.width;
                    let height = self.height;
                    let bit_depth = self.bit_depth;
                    let zoom = self.zoom;
                    let pixel_scale_x = self.pixel_scale_x;
                    let pixel_scale_y = self.pixel_scale_y;
                    let scale_unit = self.scale_unit.clone();
                    let last_frame_data = self.last_frame_data.clone();
                    let roi_selection_mode = self.roi_selector.selection_mode;

                    // Extract metadata overlay state for use in closure (bd-6h1c)
                    let show_metadata_overlay = self.show_metadata_overlay;
                    let overlay_frame_count = self.frame_count;
                    let overlay_fps = self.fps_counter.fps();
                    let overlay_timestamp_ns = self.last_frame_timestamp_ns;

                    // Extract scale bar state for use in closure (bd-0tcg)
                    let show_scale_bar = self.show_scale_bar;
                    let scale_bar_pixel_scale_x = self.pixel_scale_x;
                    let scale_bar_unit = self.scale_unit.clone();
                    let scale_bar_position = self.scale_bar_position;
                    let scale_bar_color = self.scale_bar_color;
                    let scale_bar_style = self.scale_bar_style;

                    let echelle_trace_overlay_paths = self.build_echelle_trace_overlay_paths();
                    let echelle_trace_overlay_selected_relative = self
                        .echelle_cal_ui
                        .editor_profile
                        .as_ref()
                        .and_then(|p| p.orders.get(self.echelle_cal_ui.selected_order_edit_idx))
                        .map(|o| o.relative_index);
                    let echelle_hover_marker = self.echelle_plot_hover_link.and_then(|link| {
                        let profile = self.echelle_profile_cache.profile()?;
                        let order = profile
                            .orders
                            .iter()
                            .find(|o| o.enabled && o.relative_index == link.relative_index)?;
                        let (x, y) =
                            order_sample_image_position(profile, order, link.sample_index)?;
                        Some((
                            x,
                            y,
                            format!("mvp λ={:.4}, f={:.1}", link.wavelength, link.flux),
                        ))
                    });

                    // Track crosshair lock changes to apply after closure
                    let mut crosshair_lock_action: Option<Option<(i32, i32)>> = None;

                    // StripBuilder for full-height horizontal split: image + colorbar
                    StripBuilder::new(ui)
                        .size(Size::remainder()) // image column
                        .size(Size::exact(colorbar_width)) // colorbar column
                        .horizontal(|mut strip| {
                            strip.cell(|ui| {
                                // Scrollable/pannable area for image
                                egui::ScrollArea::both()
                                    .scroll_bar_visibility(
                                        egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                                    )
                                    .id_salt("image_scroll")
                                    .show(ui, |ui| {
                                        let (rect, response) = ui.allocate_exact_size(
                                            image_available.max(image_size),
                                            egui::Sense::click_and_drag(),
                                        );

                                        // Calculate image offset (centered)
                                        let offset =
                                            (image_available - image_size) / 2.0 + self.pan;
                                        let image_rect = egui::Rect::from_min_size(
                                            rect.min + offset,
                                            image_size,
                                        );

                                        match self.measurement_tool {
                                            MeasurementTool::Line => {
                                                if response
                                                    .drag_started_by(egui::PointerButton::Primary)
                                                {
                                                    if let Some(pos) = response.interact_pointer_pos()
                                                    {
                                                        let start = MeasurementPoint::from_screen_pos(
                                                            pos,
                                                            rect,
                                                            offset,
                                                            zoom,
                                                            width,
                                                            height,
                                                        );
                                                        self.line_measurement_start = start;
                                                        self.line_measurement_current = start;
                                                    }
                                                }

                                                if response.dragged_by(egui::PointerButton::Primary) {
                                                    if let Some(pos) = response.interact_pointer_pos()
                                                    {
                                                        self.line_measurement_current =
                                                            MeasurementPoint::from_screen_pos(
                                                                pos,
                                                                rect,
                                                                offset,
                                                                zoom,
                                                                width,
                                                                height,
                                                            );
                                                    }
                                                }

                                                if response.drag_stopped_by(
                                                    egui::PointerButton::Primary,
                                                ) {
                                                    if let (Some(start), Some(end)) = (
                                                        self.line_measurement_start,
                                                        self.line_measurement_current,
                                                    ) {
                                                        let measurement = LineMeasurement {
                                                            start,
                                                            end,
                                                        };
                                                        if measurement.pixel_length() > 0.5 {
                                                            self.line_measurements.push(measurement);
                                                            self.selected_line_measurement = Some(
                                                                self.line_measurements.len() - 1,
                                                            );
                                                        }
                                                    }
                                                    self.line_measurement_start = None;
                                                    self.line_measurement_current = None;
                                                }
                                            }
                                            MeasurementTool::Angle => {
                                                if response.clicked_by(egui::PointerButton::Primary) {
                                                    if let Some(pos) = response.interact_pointer_pos()
                                                    {
                                                        if let Some(point) =
                                                            MeasurementPoint::from_screen_pos(
                                                                pos,
                                                                rect,
                                                                offset,
                                                                zoom,
                                                                width,
                                                                height,
                                                            )
                                                        {
                                                            self.angle_measurement_points.push(point);
                                                            if self.angle_measurement_points.len() == 3 {
                                                                let measurement = AngleMeasurement {
                                                                    arm_a: self.angle_measurement_points[0],
                                                                    vertex: self.angle_measurement_points[1],
                                                                    arm_b: self.angle_measurement_points[2],
                                                                };
                                                                if measurement.degrees() > 0.0 {
                                                                    self.angle_measurements
                                                                        .push(measurement);
                                                                }
                                                                self.angle_measurement_points.clear();
                                                            }
                                                        }
                                                    }
                                                }

                                                if response.clicked_by(egui::PointerButton::Secondary)
                                                    || response
                                                        .ctx
                                                        .input(|i| i.key_pressed(egui::Key::Backspace))
                                                {
                                                    self.angle_measurement_points.pop();
                                                }
                                            }
                                            MeasurementTool::None if self.roi_selector.selection_mode => {
                                                let roi_finalized = self.roi_selector.handle_input(
                                                    &response,
                                                    rect,
                                                    (self.width, self.height),
                                                    self.zoom,
                                                    self.pan,
                                                );

                                                if roi_finalized {
                                                    if let (Some(roi), Some(frame_data)) =
                                                        (self.roi_selector.roi(), &self.last_frame_data)
                                                    {
                                                        self.roi_selector.set_roi_from_frame(
                                                            roi.clone(),
                                                            frame_data,
                                                            self.width,
                                                            self.height,
                                                            self.bit_depth,
                                                        );
                                                    }
                                                }
                                            }
                                            MeasurementTool::None => {
                                                if response.dragged() {
                                                    self.auto_fit = false;
                                                    self.pan += response.drag_delta();
                                                }
                                            }
                                        }

                                        // Handle zoom with scroll wheel (always active)
                                        if response.hovered() {
                                            let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
                                            if scroll_delta != 0.0 {
                                                let zoom_factor = 1.0 + scroll_delta * 0.001;
                                                self.zoom =
                                                    (self.zoom * zoom_factor).clamp(0.1, 10.0);
                                                self.auto_fit = false;
                                            }
                                        }

                                        // Draw the image
                                        ui.painter().image(
                                            texture.id(),
                                            image_rect,
                                            egui::Rect::from_min_max(
                                                egui::pos2(0.0, 0.0),
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            egui::Color32::WHITE,
                                        );

                                        // Draw ROI overlay
                                        self.roi_selector.draw_overlay(
                                            ui.painter(),
                                            rect,
                                            (self.width, self.height),
                                            self.zoom,
                                            self.pan,
                                        );

                                        let measurement_color =
                                            egui::Color32::from_rgb(80, 220, 255);
                                        let preview_color =
                                            egui::Color32::from_rgb(255, 200, 80);
                                        let measurement_stroke =
                                            egui::Stroke::new(2.0, measurement_color);
                                        let preview_stroke =
                                            egui::Stroke::new(2.0, preview_color);
                                        let painter = ui.painter();

                                        let draw_measurement_point = |point: MeasurementPoint,
                                                                      color: egui::Color32| {
                                            let pos =
                                                point.to_screen_pos(rect, offset, zoom);
                                            painter.circle_filled(pos, 4.0, color);
                                        };

                                        let draw_measurement_label =
                                            |pos: egui::Pos2, label: String, color: egui::Color32| {
                                                painter.text(
                                                    pos + egui::vec2(6.0, -6.0),
                                                    egui::Align2::LEFT_BOTTOM,
                                                    label,
                                                    egui::FontId::monospace(11.0),
                                                    color,
                                                );
                                            };

                                        for measurement in &self.line_measurements {
                                            let start =
                                                measurement.start.to_screen_pos(rect, offset, zoom);
                                            let end =
                                                measurement.end.to_screen_pos(rect, offset, zoom);
                                            painter.line_segment(
                                                [start, end],
                                                measurement_stroke,
                                            );
                                            draw_measurement_point(
                                                measurement.start,
                                                measurement_color,
                                            );
                                            draw_measurement_point(
                                                measurement.end,
                                                measurement_color,
                                            );
                                            let midpoint = egui::pos2(
                                                (start.x + end.x) * 0.5,
                                                (start.y + end.y) * 0.5,
                                            );
                                            draw_measurement_label(
                                                midpoint,
                                                measurement.label(
                                                    pixel_scale_x,
                                                    pixel_scale_y,
                                                    &scale_unit,
                                                ),
                                                measurement_color,
                                            );
                                        }

                                        if let (Some(start), Some(current)) = (
                                            self.line_measurement_start,
                                            self.line_measurement_current,
                                        ) {
                                            let start_pos =
                                                start.to_screen_pos(rect, offset, zoom);
                                            let current_pos =
                                                current.to_screen_pos(rect, offset, zoom);
                                            painter.line_segment(
                                                [start_pos, current_pos],
                                                preview_stroke,
                                            );
                                            draw_measurement_point(start, preview_color);
                                            draw_measurement_point(current, preview_color);
                                            let preview = LineMeasurement { start, end: current };
                                            let midpoint = egui::pos2(
                                                (start_pos.x + current_pos.x) * 0.5,
                                                (start_pos.y + current_pos.y) * 0.5,
                                            );
                                            draw_measurement_label(
                                                midpoint,
                                                preview.label(
                                                    pixel_scale_x,
                                                    pixel_scale_y,
                                                    &scale_unit,
                                                ),
                                                preview_color,
                                            );
                                        }

                                        for measurement in &self.angle_measurements {
                                            let arm_a =
                                                measurement.arm_a.to_screen_pos(rect, offset, zoom);
                                            let vertex =
                                                measurement.vertex.to_screen_pos(rect, offset, zoom);
                                            let arm_b =
                                                measurement.arm_b.to_screen_pos(rect, offset, zoom);
                                            painter.line_segment(
                                                [arm_a, vertex],
                                                measurement_stroke,
                                            );
                                            painter.line_segment(
                                                [vertex, arm_b],
                                                measurement_stroke,
                                            );
                                            draw_measurement_point(
                                                measurement.arm_a,
                                                measurement_color,
                                            );
                                            draw_measurement_point(
                                                measurement.vertex,
                                                measurement_color,
                                            );
                                            draw_measurement_point(
                                                measurement.arm_b,
                                                measurement_color,
                                            );
                                            draw_measurement_label(
                                                vertex,
                                                measurement.label(),
                                                measurement_color,
                                            );
                                        }

                                        if !self.angle_measurement_points.is_empty() {
                                            for point in &self.angle_measurement_points {
                                                draw_measurement_point(*point, preview_color);
                                            }
                                            for segment in self.angle_measurement_points.windows(2) {
                                                let start = segment[0].to_screen_pos(
                                                    rect, offset, zoom,
                                                );
                                                let end = segment[1].to_screen_pos(
                                                    rect, offset, zoom,
                                                );
                                                painter.line_segment(
                                                    [start, end],
                                                    preview_stroke,
                                                );
                                            }
                                            if self.angle_measurement_points.len() == 2 {
                                                if let Some(hover_pos) = response.hover_pos() {
                                                    if let Some(point) =
                                                        MeasurementPoint::from_screen_pos(
                                                            hover_pos,
                                                            rect,
                                                            offset,
                                                            zoom,
                                                            width,
                                                            height,
                                                        )
                                                    {
                                                        let start = self.angle_measurement_points[1]
                                                            .to_screen_pos(rect, offset, zoom);
                                                        let end = point.to_screen_pos(
                                                            rect, offset, zoom,
                                                        );
                                                        painter.line_segment(
                                                            [start, end],
                                                            egui::Stroke::new(
                                                                1.0,
                                                                egui::Color32::from_rgba_unmultiplied(
                                                                    preview_color.r(),
                                                                    preview_color.g(),
                                                                    preview_color.b(),
                                                                    160,
                                                                ),
                                                            ),
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        // bd-6h1c: Draw metadata overlay on the image
                                        if show_metadata_overlay && overlay_frame_count > 0 {
                                            let painter = ui.painter();
                                            let overlay_padding = 8.0_f32;
                                            let overlay_pos = egui::pos2(
                                                image_rect.min.x + overlay_padding,
                                                image_rect.min.y + overlay_padding,
                                            );

                                            // Build overlay text lines
                                            let mut lines = Vec::with_capacity(3);
                                            lines.push(format!("Frame: {}", overlay_frame_count));
                                            lines.push(format!("FPS: {:.1}", overlay_fps));
                                            if overlay_timestamp_ns > 0 {
                                                let secs = overlay_timestamp_ns / 1_000_000_000;
                                                let subsec_ms = (overlay_timestamp_ns
                                                    % 1_000_000_000)
                                                    / 1_000_000;
                                                let h = secs / 3600;
                                                let m = (secs % 3600) / 60;
                                                let s = secs % 60;
                                                lines.push(format!(
                                                    "T: {:02}:{:02}:{:02}.{:03}",
                                                    h, m, s, subsec_ms
                                                ));
                                            }

                                            let text = lines.join("\n");
                                            let text_color = egui::Color32::WHITE;
                                            let galley = painter.layout_no_wrap(
                                                text,
                                                egui::FontId::monospace(12.0),
                                                text_color,
                                            );
                                            let bg_rect = egui::Rect::from_min_size(
                                                overlay_pos,
                                                galley.size() + egui::vec2(8.0, 8.0),
                                            );
                                            painter.rect_filled(
                                                bg_rect,
                                                4.0,
                                                egui::Color32::from_black_alpha(160),
                                            );
                                            painter.galley(
                                                overlay_pos + egui::vec2(4.0, 4.0),
                                                galley,
                                                text_color,
                                            );
                                        }

                                        // bd-0tcg: Draw scale bar overlay on the image (bottom-left)
                                        if show_scale_bar && width > 0 && height > 0 {
                                            let painter = ui.painter();
                                            let padding = 12.0_f32;
                                            let bar_height = 4.0_f32;
                                            let label_gap = 4.0_f32;
                                            let overlay_color = scale_bar_color.bar_color();
                                            let contrast_color = scale_bar_color.contrast_color();
                                            let top_aligned = matches!(
                                                scale_bar_position,
                                                ScaleBarPosition::TopLeft
                                                    | ScaleBarPosition::TopRight
                                            );

                                            if let Some(um_per_px) = scale_bar_pixel_scale_x {
                                                // Calibrated: compute a "nice" bar length
                                                #[allow(clippy::cast_precision_loss)]
                                                let image_width_um = f64::from(width) * um_per_px;
                                                let target_um = image_width_um * 0.2; // ~20% of image

                                                // Pick the nearest "nice" value from a fixed set
                                                let nice_values: &[f64] = &[
                                                    0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0,
                                                    100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0,
                                                ];
                                                let bar_um = nice_values
                                                    .iter()
                                                    .copied()
                                                    .min_by(|a, b| {
                                                        let da = (a - target_um).abs();
                                                        let db = (b - target_um).abs();
                                                        da.partial_cmp(&db)
                                                            .unwrap_or(std::cmp::Ordering::Equal)
                                                    })
                                                    .unwrap_or(100.0);

                                                // Convert bar length from physical units to screen pixels
                                                let bar_pixels = bar_um / um_per_px; // image pixels
                                                #[allow(clippy::cast_possible_truncation)]
                                                let unclamped_bar_screen_width =
                                                    (bar_pixels as f32) * zoom;
                                                let max_bar_width =
                                                    (image_rect.width() - padding * 2.0).max(8.0);
                                                let bar_screen_width =
                                                    unclamped_bar_screen_width.min(max_bar_width);

                                                let bar_x = match scale_bar_position {
                                                    ScaleBarPosition::TopLeft
                                                    | ScaleBarPosition::BottomLeft => {
                                                        image_rect.min.x + padding
                                                    }
                                                    ScaleBarPosition::TopRight
                                                    | ScaleBarPosition::BottomRight => {
                                                        image_rect.max.x
                                                            - padding
                                                            - bar_screen_width
                                                    }
                                                };
                                                let bar_y = if top_aligned {
                                                    image_rect.min.y + padding
                                                } else {
                                                    image_rect.max.y - padding - bar_height
                                                };

                                                let bar_rect = egui::Rect::from_min_size(
                                                    egui::pos2(bar_x, bar_y),
                                                    egui::vec2(bar_screen_width, bar_height),
                                                );

                                                match scale_bar_style {
                                                    ScaleBarStyle::Solid => {
                                                        let outline_rect =
                                                            egui::Rect::from_min_size(
                                                                egui::pos2(
                                                                    bar_x - 1.0,
                                                                    bar_y - 1.0,
                                                                ),
                                                                egui::vec2(
                                                                    bar_screen_width + 2.0,
                                                                    bar_height + 2.0,
                                                                ),
                                                            );
                                                        painter.rect_filled(
                                                            outline_rect,
                                                            0.0,
                                                            contrast_color,
                                                        );
                                                        painter.rect_filled(
                                                            bar_rect,
                                                            0.0,
                                                            overlay_color,
                                                        );
                                                    }
                                                    ScaleBarStyle::Outlined => {
                                                        let stroke = egui::Stroke::new(
                                                            2.0,
                                                            overlay_color,
                                                        );
                                                        painter.line_segment(
                                                            [
                                                                bar_rect.left_top(),
                                                                bar_rect.right_top(),
                                                            ],
                                                            stroke,
                                                        );
                                                        painter.line_segment(
                                                            [
                                                                bar_rect.right_top(),
                                                                bar_rect.right_bottom(),
                                                            ],
                                                            stroke,
                                                        );
                                                        painter.line_segment(
                                                            [
                                                                bar_rect.right_bottom(),
                                                                bar_rect.left_bottom(),
                                                            ],
                                                            stroke,
                                                        );
                                                        painter.line_segment(
                                                            [
                                                                bar_rect.left_bottom(),
                                                                bar_rect.left_top(),
                                                            ],
                                                            stroke,
                                                        );
                                                    }
                                                    ScaleBarStyle::Minimal => {
                                                        let stroke = egui::Stroke::new(
                                                            2.0,
                                                            overlay_color,
                                                        );
                                                        painter.line_segment(
                                                            [
                                                                egui::pos2(bar_x, bar_y),
                                                                egui::pos2(
                                                                    bar_x + bar_screen_width,
                                                                    bar_y,
                                                                ),
                                                            ],
                                                            stroke,
                                                        );
                                                        painter.line_segment(
                                                            [
                                                                egui::pos2(bar_x, bar_y - 5.0),
                                                                egui::pos2(bar_x, bar_y + 5.0),
                                                            ],
                                                            stroke,
                                                        );
                                                        painter.line_segment(
                                                            [
                                                                egui::pos2(
                                                                    bar_x + bar_screen_width,
                                                                    bar_y - 5.0,
                                                                ),
                                                                egui::pos2(
                                                                    bar_x + bar_screen_width,
                                                                    bar_y + 5.0,
                                                                ),
                                                            ],
                                                            stroke,
                                                        );
                                                    }
                                                }

                                                // Format label: use integer if whole number, else one decimal
                                                let label = if bar_um.fract() < f64::EPSILON {
                                                    #[allow(clippy::cast_possible_truncation)]
                                                    let v = bar_um as u64;
                                                    format!("{} {}", v, &scale_bar_unit)
                                                } else {
                                                    format!("{:.1} {}", bar_um, &scale_bar_unit)
                                                };

                                                let label_pos = egui::pos2(
                                                    bar_x + bar_screen_width / 2.0,
                                                    if top_aligned {
                                                        bar_y + bar_height + label_gap
                                                    } else {
                                                        bar_y - label_gap
                                                    },
                                                );
                                                let label_align = if top_aligned {
                                                    egui::Align2::CENTER_TOP
                                                } else {
                                                    egui::Align2::CENTER_BOTTOM
                                                };

                                                if scale_bar_style == ScaleBarStyle::Outlined {
                                                    let galley = painter.layout_no_wrap(
                                                        label.clone(),
                                                        egui::FontId::proportional(12.0),
                                                        overlay_color,
                                                    );
                                                    let label_min = if top_aligned {
                                                        label_pos - egui::vec2(
                                                            galley.size().x / 2.0 + 4.0,
                                                            2.0,
                                                        )
                                                    } else {
                                                        label_pos
                                                            - egui::vec2(
                                                                galley.size().x / 2.0 + 4.0,
                                                                galley.size().y + 6.0,
                                                            )
                                                    };
                                                    let label_rect = egui::Rect::from_min_size(
                                                        label_min,
                                                        galley.size() + egui::vec2(8.0, 4.0),
                                                    );
                                                    painter.rect_filled(
                                                        label_rect,
                                                        4.0,
                                                        contrast_color.linear_multiply(0.75),
                                                    );
                                                    painter.galley(
                                                        label_rect.min + egui::vec2(4.0, 2.0),
                                                        galley,
                                                        overlay_color,
                                                    );
                                                } else {
                                                    for dx in [-1.0_f32, 0.0, 1.0] {
                                                        for dy in [-1.0_f32, 0.0, 1.0] {
                                                            if dx != 0.0 || dy != 0.0 {
                                                                painter.text(
                                                                    label_pos
                                                                        + egui::vec2(dx, dy),
                                                                    label_align,
                                                                    &label,
                                                                    egui::FontId::proportional(
                                                                        12.0,
                                                                    ),
                                                                    contrast_color,
                                                                );
                                                            }
                                                        }
                                                    }
                                                    painter.text(
                                                        label_pos,
                                                        label_align,
                                                        &label,
                                                        egui::FontId::proportional(12.0),
                                                        overlay_color,
                                                    );
                                                }
                                            } else {
                                                let warn_pos = match scale_bar_position {
                                                    ScaleBarPosition::TopLeft => egui::pos2(
                                                        image_rect.min.x + padding,
                                                        image_rect.min.y + padding,
                                                    ),
                                                    ScaleBarPosition::TopRight => egui::pos2(
                                                        image_rect.max.x - padding,
                                                        image_rect.min.y + padding,
                                                    ),
                                                    ScaleBarPosition::BottomLeft => egui::pos2(
                                                        image_rect.min.x + padding,
                                                        image_rect.max.y - padding - bar_height,
                                                    ),
                                                    ScaleBarPosition::BottomRight => egui::pos2(
                                                        image_rect.max.x - padding,
                                                        image_rect.max.y - padding - bar_height,
                                                    ),
                                                };
                                                let warn_text = "Scale bar: uncalibrated";
                                                let warn_galley = painter.layout_no_wrap(
                                                    warn_text.to_string(),
                                                    egui::FontId::proportional(11.0),
                                                    egui::Color32::from_rgb(255, 200, 80),
                                                );
                                                let warn_bg = match scale_bar_position {
                                                    ScaleBarPosition::TopLeft => {
                                                        egui::Rect::from_min_size(
                                                            warn_pos,
                                                            warn_galley.size()
                                                                + egui::vec2(8.0, 4.0),
                                                        )
                                                    }
                                                    ScaleBarPosition::TopRight => {
                                                        egui::Rect::from_min_size(
                                                            warn_pos
                                                                - egui::vec2(
                                                                    warn_galley.size().x + 8.0,
                                                                    0.0,
                                                                ),
                                                            warn_galley.size()
                                                                + egui::vec2(8.0, 4.0),
                                                        )
                                                    }
                                                    ScaleBarPosition::BottomLeft => {
                                                        egui::Rect::from_min_size(
                                                            warn_pos
                                                                - egui::vec2(
                                                                    0.0,
                                                                    warn_galley.size().y + 4.0,
                                                                ),
                                                            warn_galley.size()
                                                                + egui::vec2(8.0, 4.0),
                                                        )
                                                    }
                                                    ScaleBarPosition::BottomRight => {
                                                        egui::Rect::from_min_size(
                                                            warn_pos
                                                                - egui::vec2(
                                                                    warn_galley.size().x + 8.0,
                                                                    warn_galley.size().y + 4.0,
                                                                ),
                                                            warn_galley.size()
                                                                + egui::vec2(8.0, 4.0),
                                                        )
                                                    }
                                                };
                                                painter.rect_filled(
                                                    warn_bg,
                                                    4.0,
                                                    egui::Color32::from_black_alpha(180),
                                                );
                                                painter.galley(
                                                    warn_bg.min + egui::vec2(4.0, 2.0),
                                                    warn_galley,
                                                    egui::Color32::from_rgb(255, 200, 80),
                                                );
                                            }
                                        }

                                        // Draw histogram overlay if positioned on image
                                        if self.histogram_position.is_overlay() {
                                            let hist_size = egui::vec2(180.0, 80.0);
                                            let hist_rect = self
                                                .histogram_position
                                                .overlay_rect(image_rect, hist_size);

                                            // Create a child UI at the overlay position
                                            let mut hist_ui = ui.new_child(
                                                egui::UiBuilder::new().max_rect(hist_rect).layout(
                                                    egui::Layout::left_to_right(egui::Align::Min),
                                                ),
                                            );
                                            self.histogram.show_overlay(&mut hist_ui, hist_size);
                                        }

                                        if !echelle_trace_overlay_paths.is_empty() {
                                            let painter = ui.painter();
                                            for (relative_index, path) in
                                                &echelle_trace_overlay_paths
                                            {
                                                let color = if Some(*relative_index)
                                                    == echelle_trace_overlay_selected_relative
                                                {
                                                    egui::Color32::from_rgb(80, 220, 120)
                                                } else {
                                                    egui::Color32::from_rgba_unmultiplied(
                                                        100, 180, 255, 180,
                                                    )
                                                };
                                                let stroke = egui::Stroke::new(
                                                    if Some(*relative_index)
                                                        == echelle_trace_overlay_selected_relative
                                                    {
                                                        2.0
                                                    } else {
                                                        1.0
                                                    },
                                                    color,
                                                );
                                                for segment in path.windows(2) {
                                                    let (x0, y0) = segment[0];
                                                    let (x1, y1) = segment[1];
                                                    let p0 = egui::pos2(
                                                        rect.min.x + offset.x + x0 * zoom,
                                                        rect.min.y + offset.y + y0 * zoom,
                                                    );
                                                    let p1 = egui::pos2(
                                                        rect.min.x + offset.x + x1 * zoom,
                                                        rect.min.y + offset.y + y1 * zoom,
                                                    );
                                                    if image_rect.contains(p0)
                                                        || image_rect.contains(p1)
                                                    {
                                                        painter.line_segment([p0, p1], stroke);
                                                    }
                                                }
                                                if let Some((x, y)) = path.first().copied() {
                                                    let p = egui::pos2(
                                                        rect.min.x + offset.x + x * zoom,
                                                        rect.min.y + offset.y + y * zoom,
                                                    );
                                                    if image_rect.contains(p) {
                                                        painter.text(
                                                            p + egui::vec2(6.0, 6.0),
                                                            egui::Align2::LEFT_TOP,
                                                            format!("rel {}", relative_index),
                                                            egui::FontId::monospace(10.0),
                                                            color,
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        if let Some((px, py, label)) = &echelle_hover_marker {
                                            let marker_x = rect.min.x + offset.x + *px * zoom;
                                            let marker_y = rect.min.y + offset.y + *py * zoom;
                                            let marker_pos = egui::pos2(marker_x, marker_y);
                                            if image_rect.contains(marker_pos) {
                                                let painter = ui.painter();
                                                let color = egui::Color32::from_rgb(255, 120, 0);
                                                painter.circle_stroke(
                                                    marker_pos,
                                                    (4.0 * zoom.clamp(0.5, 2.0)).max(4.0),
                                                    egui::Stroke::new(2.0, color),
                                                );
                                                painter.circle_filled(marker_pos, 2.0, color);
                                                painter.text(
                                                    marker_pos + egui::vec2(8.0, -8.0),
                                                    egui::Align2::LEFT_BOTTOM,
                                                    label,
                                                    egui::FontId::monospace(11.0),
                                                    color,
                                                );
                                            }
                                        }

                                        // Crosshair cursor with pixel readout (bd-pgcb)
                                        if crosshair_enabled {
                                            // Determine crosshair position (locked or hover)
                                            let crosshair_pixel_pos = if let Some(locked_pos) =
                                                crosshair_locked_pos
                                            {
                                                Some(locked_pos)
                                            } else if let Some(hover_pos) = response.hover_pos() {
                                                let image_pos = hover_pos - rect.min - offset;
                                                #[allow(clippy::cast_possible_truncation)]
                                                let pixel_x = (image_pos.x / zoom) as i32;
                                                #[allow(
                                                    clippy::cast_possible_truncation,
                                                    clippy::cast_possible_wrap
                                                )]
                                                let pixel_y = (image_pos.y / zoom) as i32;
                                                #[allow(clippy::cast_possible_wrap)]
                                                let w_i32 = width as i32;
                                                #[allow(clippy::cast_possible_wrap)]
                                                let h_i32 = height as i32;
                                                if pixel_x >= 0
                                                    && pixel_x < w_i32
                                                    && pixel_y >= 0
                                                    && pixel_y < h_i32
                                                {
                                                    Some((pixel_x, pixel_y))
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            };

                                            // Handle click to lock/unlock crosshair (defer mutation)
                                            if response.clicked()
                                                && !roi_selection_mode
                                                && self.measurement_tool == MeasurementTool::None
                                            {
                                                if let Some(hover_pos) =
                                                    response.interact_pointer_pos()
                                                {
                                                    let image_pos = hover_pos - rect.min - offset;
                                                    #[allow(clippy::cast_possible_truncation)]
                                                    let pixel_x = (image_pos.x / zoom) as i32;
                                                    #[allow(
                                                        clippy::cast_possible_truncation,
                                                        clippy::cast_possible_wrap
                                                    )]
                                                    let pixel_y = (image_pos.y / zoom) as i32;
                                                    #[allow(clippy::cast_possible_wrap)]
                                                    let w_i32 = width as i32;
                                                    #[allow(clippy::cast_possible_wrap)]
                                                    let h_i32 = height as i32;
                                                    if pixel_x >= 0
                                                        && pixel_x < w_i32
                                                        && pixel_y >= 0
                                                        && pixel_y < h_i32
                                                    {
                                                        // Toggle lock: if already locked at this position, unlock
                                                        if crosshair_locked_pos
                                                            == Some((pixel_x, pixel_y))
                                                        {
                                                            crosshair_lock_action = Some(None);
                                                        } else {
                                                            crosshair_lock_action =
                                                                Some(Some((pixel_x, pixel_y)));
                                                        }
                                                    }
                                                }
                                            }

                                            // Draw crosshair and readout if position is valid
                                            if let Some((pixel_x, pixel_y)) = crosshair_pixel_pos {
                                                // Convert pixel coordinates to screen coordinates
                                                #[allow(clippy::cast_precision_loss)]
                                                let screen_x = rect.min.x
                                                    + offset.x
                                                    + (pixel_x as f32 + 0.5) * zoom;
                                                #[allow(clippy::cast_precision_loss)]
                                                let screen_y = rect.min.y
                                                    + offset.y
                                                    + (pixel_y as f32 + 0.5) * zoom;
                                                let crosshair_pos = egui::pos2(screen_x, screen_y);

                                                let painter = ui.painter();
                                                let crosshair_color =
                                                    if crosshair_locked_pos.is_some() {
                                                        egui::Color32::from_rgb(255, 200, 0)
                                                    } else {
                                                        egui::Color32::from_rgb(0, 255, 0)
                                                    };
                                                let stroke =
                                                    egui::Stroke::new(1.5, crosshair_color);

                                                // Draw crosshair lines
                                                let line_length = 15.0;
                                                painter.line_segment(
                                                    [
                                                        egui::pos2(
                                                            crosshair_pos.x - line_length,
                                                            crosshair_pos.y,
                                                        ),
                                                        egui::pos2(
                                                            crosshair_pos.x - 3.0,
                                                            crosshair_pos.y,
                                                        ),
                                                    ],
                                                    stroke,
                                                );
                                                painter.line_segment(
                                                    [
                                                        egui::pos2(
                                                            crosshair_pos.x + 3.0,
                                                            crosshair_pos.y,
                                                        ),
                                                        egui::pos2(
                                                            crosshair_pos.x + line_length,
                                                            crosshair_pos.y,
                                                        ),
                                                    ],
                                                    stroke,
                                                );
                                                painter.line_segment(
                                                    [
                                                        egui::pos2(
                                                            crosshair_pos.x,
                                                            crosshair_pos.y - line_length,
                                                        ),
                                                        egui::pos2(
                                                            crosshair_pos.x,
                                                            crosshair_pos.y - 3.0,
                                                        ),
                                                    ],
                                                    stroke,
                                                );
                                                painter.line_segment(
                                                    [
                                                        egui::pos2(
                                                            crosshair_pos.x,
                                                            crosshair_pos.y + 3.0,
                                                        ),
                                                        egui::pos2(
                                                            crosshair_pos.x,
                                                            crosshair_pos.y + line_length,
                                                        ),
                                                    ],
                                                    stroke,
                                                );

                                                // Draw center dot
                                                painter.circle_filled(
                                                    crosshair_pos,
                                                    2.0,
                                                    crosshair_color,
                                                );

                                                // Get pixel intensity value
                                                let pixel_value =
                                                    if let Some(frame_data) = &last_frame_data {
                                                        get_pixel_value_inline(
                                                            frame_data,
                                                            pixel_x as u32,
                                                            pixel_y as u32,
                                                            width,
                                                            height,
                                                            bit_depth,
                                                        )
                                                    } else {
                                                        None
                                                    };

                                                // Build readout text
                                                let mut readout_lines = Vec::new();
                                                readout_lines.push(format!(
                                                    "X: {} px, Y: {} px",
                                                    pixel_x, pixel_y
                                                ));

                                                // Physical coordinates if calibrated
                                                if let (Some(scale_x), Some(scale_y)) =
                                                    (pixel_scale_x, pixel_scale_y)
                                                {
                                                    let phys_x = f64::from(pixel_x) * scale_x;
                                                    let phys_y = f64::from(pixel_y) * scale_y;
                                                    readout_lines.push(format!(
                                                        "X: {:.2} {}, Y: {:.2} {}",
                                                        phys_x, &scale_unit, phys_y, &scale_unit
                                                    ));
                                                }

                                                // Pixel intensity
                                                if let Some(value) = pixel_value {
                                                    readout_lines
                                                        .push(format!("Intensity: {}", value));
                                                }

                                                // Draw readout panel (top-left corner of image)
                                                let panel_padding = 8.0;
                                                let panel_pos = egui::pos2(
                                                    image_rect.min.x + panel_padding,
                                                    image_rect.min.y + panel_padding,
                                                );
                                                let text_galley = painter.layout_no_wrap(
                                                    readout_lines.join("\n"),
                                                    egui::FontId::monospace(12.0),
                                                    crosshair_color,
                                                );
                                                let panel_rect = egui::Rect::from_min_size(
                                                    panel_pos,
                                                    text_galley.size() + egui::vec2(8.0, 8.0),
                                                );
                                                painter.rect_filled(
                                                    panel_rect,
                                                    4.0,
                                                    egui::Color32::from_black_alpha(180),
                                                );
                                                painter.galley(
                                                    panel_pos + egui::vec2(4.0, 4.0),
                                                    text_galley,
                                                    crosshair_color,
                                                );
                                            }
                                        } else {
                                            // Simple hover text when crosshair is disabled (bd-07j1)
                                            if let Some(pos) = response.hover_pos() {
                                                let image_pos = pos - rect.min - offset;
                                                #[allow(clippy::cast_possible_truncation)]
                                                let pixel_x = (image_pos.x / self.zoom) as i32;
                                                #[allow(
                                                    clippy::cast_possible_truncation,
                                                    clippy::cast_possible_wrap
                                                )]
                                                let pixel_y = (image_pos.y / self.zoom) as i32;
                                                #[allow(clippy::cast_possible_wrap)]
                                                let w_i32 = self.width as i32;
                                                #[allow(clippy::cast_possible_wrap)]
                                                let h_i32 = self.height as i32;
                                                if pixel_x >= 0
                                                    && pixel_x < w_i32
                                                    && pixel_y >= 0
                                                    && pixel_y < h_i32
                                                {
                                                    // Build hover text with pixel and optional physical coordinates
                                                    let hover_text =
                                                        if let (Some(scale_x), Some(scale_y)) =
                                                            (self.pixel_scale_x, self.pixel_scale_y)
                                                        {
                                                            let phys_x =
                                                                f64::from(pixel_x) * scale_x;
                                                            let phys_y =
                                                                f64::from(pixel_y) * scale_y;
                                                            format!(
                                                            "Pixel: ({}, {}) | {:.2} {} x {:.2} {}",
                                                            pixel_x,
                                                            pixel_y,
                                                            phys_x,
                                                            &self.scale_unit,
                                                            phys_y,
                                                            &self.scale_unit
                                                        )
                                                        } else {
                                                            format!(
                                                                "Pixel: ({}, {})",
                                                                pixel_x, pixel_y
                                                            )
                                                        };
                                                    response.on_hover_text(hover_text);
                                                }
                                            }
                                        }
                                    });
                            });
                            strip.cell(|ui| {
                                // bd-07j1: Colorbar widget
                                if self.show_colorbar {
                                    ui.add_space(4.0);
                                    let colorbar_size =
                                        egui::vec2(40.0, ui.available_height() - 20.0);
                                    if self.colorbar.show(ui, &self.colormap, colorbar_size) {
                                        // Midpoint changed - request repaint to update image
                                        ui.ctx().request_repaint();
                                    }
                                }
                            });
                        });

                    // Apply crosshair lock changes after closure (bd-pgcb)
                    if let Some(action) = crosshair_lock_action {
                        self.crosshair_locked_pos = action;
                    }
                } else {
                    // No image - show placeholder
                    ui.centered_and_justified(|ui| {
                        ui.label("No image. Select a camera device and start streaming.");
                    });
                }
            });
    }

    // =========================================================================
    // Public API for programmatic control
    // =========================================================================

    /// Set the device to stream from (for external control)
    ///
    /// This allows programmatic selection of which camera to stream.
    /// Use in automated workflows or scripted interactions.
    #[allow(dead_code)]
    pub fn set_device(&mut self, device_id: &str, client: &mut DaqClient, runtime: &Runtime) {
        self.start_stream(device_id, client, runtime);
        // Eagerly load camera parameters so settings panel populates on device selection
        // rather than requiring the user to start a stream first.
        self.load_camera_params(client, runtime, device_id);
    }

    /// Check if currently streaming
    #[allow(dead_code)]
    pub fn is_streaming(&self) -> bool {
        self.subscription.is_some()
    }

    /// Get current device ID being streamed
    #[allow(dead_code)]
    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }
}
