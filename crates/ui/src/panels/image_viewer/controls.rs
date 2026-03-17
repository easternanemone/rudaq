//! Camera controls — parameter management, action polling, hardware ROI.

use super::*;

impl ImageViewerPanel {
    pub(super) fn poll_actions(&mut self) {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                ImageViewerAction::CamerasLoaded {
                    ids,
                    full_frame_dims,
                } => {
                    self.available_cameras = ids;
                    self.camera_full_frame_dims = full_frame_dims;
                    self.status = Some(format!("Found {} camera(s)", self.available_cameras.len()));
                }
                ImageViewerAction::Error(msg) => {
                    self.error = Some(msg);
                    // Clear subscription state on error to allow restart
                    self.subscription = None;
                    // bd-12qt: Update connection state on error
                    if self.connection_state == ConnectionState::Connected {
                        self.connection_state = ConnectionState::Disconnected;
                        self.last_disconnect = Some(Instant::now());
                        self.retry_count = 0;
                    }
                }
                ImageViewerAction::ReconnectResult { device_id, success } => {
                    // bd-12qt: Handle reconnection result
                    if success {
                        self.connection_state = ConnectionState::Connected;
                        self.retry_count = 0;
                        self.error = None;
                        self.status = Some(format!("Reconnected to {}", device_id));
                    } else {
                        self.connection_state = ConnectionState::Disconnected;
                        self.retry_count += 1;
                        self.status =
                            Some(format!("Reconnect failed (attempt {})", self.retry_count));
                    }
                }
                // bd-3pdi.5.3: Recording action handlers
                ImageViewerAction::RecordingStarted { output_path } => {
                    self.recording_state = RecordingState::Recording;
                    self.recording_output_path = Some(output_path.clone());
                    self.status = Some(format!("Recording to {}", output_path));
                    self.error = None;
                }
                ImageViewerAction::RecordingStopped {
                    output_path,
                    file_size_bytes,
                    total_samples,
                } => {
                    self.recording_state = RecordingState::Idle;
                    #[allow(clippy::cast_precision_loss)]
                    let size_mb = file_size_bytes as f64 / 1_000_000.0;
                    self.status = Some(format!(
                        "Saved: {} ({:.2} MB, {} frames)",
                        output_path, size_mb, total_samples
                    ));
                    self.error = None;
                }
                ImageViewerAction::RecordingStatus(status) => {
                    if let Some(s) = status {
                        self.recording_status = Some(s);
                        // Update recording state based on status
                        self.recording_state = match self.recording_status.as_ref().map(|s| s.state)
                        {
                            Some(2) => RecordingState::Recording, // RECORDING_ACTIVE
                            _ => RecordingState::Idle,
                        };
                    }
                }
                ImageViewerAction::EchelleCalibrationSynced { message } => {
                    self.echelle_run_engine_sync_in_flight = false;
                    self.echelle_cal_ui.status_message = Some(message);
                    self.echelle_cal_ui.last_error = None;
                }
                ImageViewerAction::EchelleCalibrationSyncError(message) => {
                    self.echelle_run_engine_sync_in_flight = false;
                    self.echelle_run_engine_sync_dirty = true;
                    self.echelle_cal_ui.last_error = Some(message);
                }
            }
        }
    }

    /// Refresh the list of available cameras
    pub(super) fn refresh_cameras(&mut self, client: &mut DaqClient, runtime: &Runtime) {
        let action_tx = self.action_tx.clone();
        let mut client = client.clone();

        runtime.spawn(async move {
            match client.list_devices().await {
                Ok(devices) => {
                    // Filter for camera devices (FrameProducer capability)
                    let mut cameras: Vec<String> = Vec::new();
                    let mut full_frame_dims: std::collections::HashMap<String, (u32, u32)> =
                        std::collections::HashMap::new();

                    for d in devices.into_iter().filter(|d| {
                        // Check is_frame_producer flag or camera category
                        d.is_frame_producer()
                            || d.category == protocol::daq::DeviceCategory::Camera as i32
                    }) {
                        let id = d.id.clone();
                        if let Some(meta) = d.metadata {
                            if let (Some(w), Some(h)) = (meta.frame_width, meta.frame_height) {
                                if w > 0 && h > 0 {
                                    full_frame_dims.insert(id.clone(), (w, h));
                                }
                            }
                        }
                        cameras.push(id);
                    }

                    let _ = action_tx.send(ImageViewerAction::CamerasLoaded {
                        ids: cameras,
                        full_frame_dims,
                    });
                }
                Err(e) => {
                    let _ = action_tx.send(ImageViewerAction::Error(format!(
                        "Failed to list cameras: {}",
                        e
                    )));
                }
            }
        });

        self.last_refresh = Some(Instant::now());
    }

    /// Load parameters for the selected camera (filtered for quick access)
    pub(super) fn load_camera_params(
        &mut self,
        client: &mut DaqClient,
        runtime: &Runtime,
        device_id: &str,
    ) {
        // Don't start another load if already loading
        if self.loading_params_device.as_deref() == Some(device_id) {
            return;
        }

        let mut client = client.clone();
        let device_id_str = device_id.to_string();

        // Clear existing edit buffers and errors for this device
        self.param_edit_buffers
            .retain(|(dev_id, _), _| dev_id != device_id);
        self.param_errors
            .retain(|(dev_id, _), _| dev_id != device_id);

        // Set loading state
        self.loading_params_device = Some(device_id_str.clone());

        // Create channel for result
        let (tx, rx) = mpsc::channel();
        self.param_load_rx = Some(rx);

        // Spawn async task to load parameters in background
        runtime.spawn(async move {
            let device_id_for_error = device_id_str.clone();

            let result = async {
                let descriptors = client.list_parameters(&device_id_str).await?;

                // Fetch ALL parameters — the UI groups them by group_name
                // for camera-agnostic display (bd-4wf7)

                // Parallel fetch of parameter values
                let fetch_futures: Vec<_> = descriptors
                    .iter()
                    .map(|desc| {
                        let mut client = client.clone();
                        let device_id = device_id_str.clone();
                        let param_name = desc.name.clone();
                        async move {
                            let value = client.get_parameter(&device_id, &param_name).await;
                            (param_name, value)
                        }
                    })
                    .collect();

                let fetch_results = futures::future::join_all(fetch_futures).await;

                // Combine descriptors with fetched values
                let mut params = Vec::new();
                let mut load_errors = Vec::new();

                for (desc, (param_name, value_result)) in descriptors.into_iter().zip(fetch_results)
                {
                    match value_result {
                        Ok(v) => {
                            params.push(ParameterCache::new(desc, v.value));
                        }
                        Err(e) => {
                            load_errors.push((param_name, e.to_string()));
                            params.push(ParameterCache::new(desc, String::new()));
                        }
                    }
                }

                // Fetch favorites (non-fatal if it fails)
                let favorites = client
                    .get_parameter_favorites(&device_id_str)
                    .await
                    .unwrap_or_default();

                Ok::<_, anyhow::Error>(ParamLoadResult {
                    device_id: device_id_str,
                    params,
                    errors: load_errors,
                    favorites,
                })
            }
            .await;

            match result {
                Ok(load_result) => {
                    let _ = tx.send(load_result);
                }
                Err(e) => {
                    let _ = tx.send(ParamLoadResult {
                        device_id: device_id_for_error,
                        params: Vec::new(),
                        errors: vec![("_load".to_string(), e.to_string())],
                        favorites: Vec::new(),
                    });
                }
            }
        });
    }

    /// Set a camera parameter value
    pub(super) fn set_camera_parameter(
        &mut self,
        client: &mut DaqClient,
        runtime: &Runtime,
        device_id: &str,
        name: &str,
        value: &str,
    ) {
        let mut client = client.clone();
        let device_id_str = device_id.to_string();
        let name_str = name.to_string();
        let value_str = value.to_string();
        let buffer_key = (device_id_str.clone(), name_str.clone());
        tracing::debug!(
            device_id = %device_id,
            param = %name,
            value = %value,
            "set_camera_parameter: sending parameter update"
        );

        // Clear any previous error
        self.param_errors.remove(&buffer_key);
        // Mark as setting
        self.setting_params.insert(buffer_key);

        // Clone the persistent sender - this preserves all in-flight responses
        let tx = self.param_set_tx.clone();

        runtime.spawn(async move {
            let result = client
                .set_parameter(&device_id_str, &name_str, &value_str)
                .await;

            let set_result = match result {
                Ok(response) => ParamSetResult {
                    device_id: device_id_str,
                    param_name: name_str,
                    success: response.success,
                    actual_value: response.actual_value,
                    error: if response.success {
                        None
                    } else {
                        Some(response.error_message)
                    },
                },
                Err(e) => ParamSetResult {
                    device_id: device_id_str,
                    param_name: name_str,
                    success: false,
                    actual_value: String::new(),
                    error: Some(e.to_string()),
                },
            };

            let _ = tx.send(set_result);
        });
    }

    /// Poll for parameter async results
    pub(super) fn poll_param_results(&mut self, ctx: &egui::Context) {
        // Poll loads
        if let Some(rx) = &self.param_load_rx {
            if let Ok(result) = rx.try_recv() {
                // If this result matches our current device, update
                if Some(&result.device_id) == self.device_id.as_ref() {
                    self.camera_params = result.params;
                    self.param_favorites = result.favorites.into_iter().collect();
                    self.loading_params_device = None;

                    for (name, err) in result.errors {
                        self.param_errors
                            .insert((result.device_id.clone(), name), err);
                    }
                }
                self.param_load_rx = None; // One-shot load
                ctx.request_repaint();
            }
        }

        // Poll sets (persistent channel - drain all available)
        while let Ok(result) = self.param_set_rx.try_recv() {
            let key = (result.device_id.clone(), result.param_name.clone());
            self.setting_params.remove(&key);
            tracing::debug!(
                device_id = %result.device_id,
                param = %result.param_name,
                success = result.success,
                actual_value = ?result.actual_value,
                error = ?result.error,
                "poll_param_results: received ParamSetResult"
            );

            if result.success {
                // Update cache if device matches
                if Some(&result.device_id) == self.device_id.as_ref() {
                    if let Some(param) = self
                        .camera_params
                        .iter_mut()
                        .find(|p| p.descriptor.name == result.param_name)
                    {
                        param.update_value(result.actual_value.clone());
                    }
                }
                // Update buffer
                let unquoted = result.actual_value.trim_matches('"').to_string();
                self.param_edit_buffers.insert(key.clone(), unquoted);
                self.param_errors.remove(&key);
            } else if let Some(err) = result.error {
                self.param_errors.insert(key, err);
            }
            ctx.request_repaint();
        }

        // Request repaint if we're waiting for parameter set results
        if !self.setting_params.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Render a single camera parameter control
    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn render_camera_control(
        &mut self,
        ui: &mut egui::Ui,
        device_id: &str,
        param_idx: usize,
    ) {
        ui.set_max_width(ui.available_width());

        // Safe access to parameter to avoid borrowing self for the whole method
        let param = &self.camera_params[param_idx];
        let desc = &param.descriptor;
        let param_name = desc.name.clone();
        let buffer_key = (device_id.to_string(), param_name.clone());
        let is_fav = self.param_favorites.contains(&param_name);

        // Star button for favorite pinning (bd-4wf7) — rendered inline with param name
        let star = if is_fav { "\u{2605}" } else { "\u{2606}" }; // ★ / ☆
        let star_tooltip = if is_fav {
            "Unpin from Quick Access"
        } else {
            "Pin to Quick Access"
        };

        // Check if setting
        if self.setting_params.contains(&buffer_key) {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new(star).frame(false))
                    .on_hover_text(star_tooltip)
                    .clicked()
                {
                    if is_fav {
                        self.param_favorites.remove(&param_name);
                    } else {
                        self.param_favorites.insert(param_name.clone());
                    }
                }
                ui.spinner();
                ui.label(&param_name);
            });
            return;
        }

        // Read-only
        if !desc.writable {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new(star).frame(false))
                    .on_hover_text(star_tooltip)
                    .clicked()
                {
                    if is_fav {
                        self.param_favorites.remove(&param_name);
                    } else {
                        self.param_favorites.insert(param_name.clone());
                    }
                }
                ui.label(&param_name);
                let mut value = param.current_value.clone();
                if !desc.units.is_empty() {
                    value.push(' ');
                    value.push_str(&desc.units);
                }
                ui.add(egui::Label::new(value).wrap());
            });
            return;
        }

        let mut pending_update: Option<String> = None;

        // Enums
        if !desc.enum_values.is_empty() {
            let current = param.current_value.trim_matches('"').to_string();
            let mut selected = current.clone();

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new(star).frame(false))
                    .on_hover_text(star_tooltip)
                    .clicked()
                {
                    if is_fav {
                        self.param_favorites.remove(&param_name);
                    } else {
                        self.param_favorites.insert(param_name.clone());
                    }
                }
                ui.label(&desc.name);
                let id = egui::Id::new("cam_ctrl").with(device_id).with(&desc.name);
                egui::ComboBox::from_id_salt(id)
                    .selected_text(&selected)
                    .show_ui(ui, |ui| {
                        for val in &desc.enum_values {
                            ui.selectable_value(&mut selected, val.clone(), val);
                        }
                    });
            });

            if selected != current {
                pending_update = Some(format!("\"{}\"", selected));
            }
        }
        // Boolean
        else if desc.dtype == "bool" {
            let mut val = param.current_value.parse::<bool>().unwrap_or(false);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new(star).frame(false))
                    .on_hover_text(star_tooltip)
                    .clicked()
                {
                    if is_fav {
                        self.param_favorites.remove(&param_name);
                    } else {
                        self.param_favorites.insert(param_name.clone());
                    }
                }
            });
            if ui.checkbox(&mut val, &desc.name).changed() {
                pending_update = Some(val.to_string());
            }
        }
        // Integer
        else if desc.dtype == "int" {
            // Get edit buffer or init from current
            let buffer = self
                .param_edit_buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| param.current_value.clone());

            let mut val: i64 = buffer.parse().unwrap_or(0);
            let original = val;

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new(star).frame(false))
                    .on_hover_text(star_tooltip)
                    .clicked()
                {
                    if is_fav {
                        self.param_favorites.remove(&param_name);
                    } else {
                        self.param_favorites.insert(param_name.clone());
                    }
                }
                ui.label(&desc.name);
                let mut drag = egui::DragValue::new(&mut val).speed(1);
                if let Some(min) = desc.min_value {
                    drag = drag.range(min as i64..=i64::MAX);
                }
                if let Some(max) = desc.max_value {
                    drag = drag.range(i64::MIN..=max as i64);
                }

                let response = ui.add(drag);
                if !desc.units.is_empty() {
                    ui.weak(&desc.units);
                }

                // Update buffer immediately for visual feedback
                if val != original {
                    self.param_edit_buffers
                        .insert(buffer_key.clone(), val.to_string());
                }

                // Commit on drag stop, focus loss, Enter, or step-button click.
                let commit_now = (response.changed()
                    && !response.dragged()
                    && ui.input(|i| i.pointer.any_released() || i.key_pressed(egui::Key::Enter)))
                    || response.drag_stopped()
                    || response.lost_focus();

                if commit_now && val != param.current_value.parse().unwrap_or(0) {
                    pending_update = Some(val.to_string());
                }
            });
        }
        // Float
        else if desc.dtype == "float" {
            let buffer = self
                .param_edit_buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| param.current_value.clone());

            let mut val: f64 = buffer.parse().unwrap_or(0.0);
            let original = val;

            // Check if this is an exposure parameter
            let is_exposure = desc.name.to_lowercase().contains("exposure");

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new(star).frame(false))
                    .on_hover_text(star_tooltip)
                    .clicked()
                {
                    if is_fav {
                        self.param_favorites.remove(&param_name);
                    } else {
                        self.param_favorites.insert(param_name.clone());
                    }
                }
                ui.label(&desc.name);
                let mut drag = egui::DragValue::new(&mut val).speed(0.1);
                if let Some(min) = desc.min_value {
                    drag = drag.range(min..=f64::MAX);
                }
                if let Some(max) = desc.max_value {
                    drag = drag.range(f64::MIN..=max);
                }

                let response = ui.add(drag);
                if !desc.units.is_empty() {
                    ui.weak(&desc.units);
                }

                // Live toggle for exposure parameters
                if is_exposure {
                    ui.checkbox(&mut self.live_exposure, "Live");
                }

                if (val - original).abs() > f64::EPSILON {
                    self.param_edit_buffers
                        .insert(buffer_key.clone(), val.to_string());
                }

                let current_float: f64 = param.current_value.parse().unwrap_or(0.0);
                let value_changed = (val - current_float).abs() > f64::EPSILON;

                // Live exposure: send during drag with debounce
                if is_exposure && self.live_exposure && response.dragged() && value_changed {
                    let now = Instant::now();
                    let should_send = self
                        .exposure_last_sent
                        .map(|t| now.duration_since(t) >= EXPOSURE_DEBOUNCE)
                        .unwrap_or(true);

                    if should_send {
                        pending_update = Some(val.to_string());
                        self.exposure_last_sent = Some(now);
                    }
                }

                // Always send on drag stop/focus loss, and also on Enter/step-button changes.
                let commit_now = (response.changed()
                    && !response.dragged()
                    && ui.input(|i| i.pointer.any_released() || i.key_pressed(egui::Key::Enter)))
                    || response.drag_stopped()
                    || response.lost_focus();

                if commit_now && value_changed {
                    pending_update = Some(val.to_string());
                    if is_exposure {
                        self.exposure_last_sent = Some(Instant::now());
                    }
                }
            });
        }
        // String
        else if desc.dtype == "string" {
            let buffer = self
                .param_edit_buffers
                .entry(buffer_key.clone())
                .or_insert_with(|| param.current_value.clone());

            ui.horizontal_wrapped(|ui| {
                ui.label(&desc.name);
                let response = ui.text_edit_singleline(buffer);

                if response.lost_focus() && buffer != &param.current_value {
                    pending_update = Some(format!("\"{}\"", buffer));
                }
            });
        }
        // Fallback
        else {
            ui.horizontal_wrapped(|ui| {
                ui.label(&desc.name);
                ui.label(&param.current_value);
            });
        }

        // Show error
        if let Some(err) = self.param_errors.get(&buffer_key) {
            ui.colored_label(egui::Color32::RED, err);
        }

        // Apply update if needed
        if let Some(val) = pending_update {
            self.pending_param_updates
                .push((device_id.to_string(), desc.name.clone(), val));
        }
    }

    /// Queue a parameter update to reset hardware ROI to the full sensor.
    pub(super) fn queue_clear_hardware_roi(&mut self) {
        if let Some(dev_id) = self.device_id.clone() {
            if self.subscription.is_some() {
                self.param_errors.insert(
                    (dev_id, "acquisition.roi".to_string()),
                    "Stop streaming before clearing hardware ROI".to_string(),
                );
            } else if let Some((full_w, full_h)) = self.camera_full_frame_dims.get(&dev_id).copied()
            {
                let roi_json = serde_json::json!({
                    "type": "rectangle",
                    "x": 0,
                    "y": 0,
                    "width": full_w,
                    "height": full_h
                });
                self.pending_param_updates.push((
                    dev_id.clone(),
                    "acquisition.roi".to_string(),
                    roi_json.to_string(),
                ));
                self.param_errors
                    .remove(&(dev_id, "acquisition.roi".to_string()));
            } else {
                self.param_errors.insert(
                    (dev_id, "acquisition.roi".to_string()),
                    "Unknown full-frame size; refresh camera list and retry".to_string(),
                );
            }
        }
    }
}
