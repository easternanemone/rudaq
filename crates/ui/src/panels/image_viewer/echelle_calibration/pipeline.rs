//! Calibration pipeline interaction - profile management, import/export, auto-detect.

use super::super::*;
use super::rendering::detect_cross_dispersion_peaks_from_frame;

impl ImageViewerPanel {
    /// Poll for remote profile load results (bd-nss7).
    pub(in crate::panels::image_viewer) fn poll_remote_profile_load(&mut self) {
        let result = match &self.remote_profile_load_rx {
            Some(rx) => rx.try_recv().ok(),
            None => None,
        };
        if let Some(result) = result {
            self.remote_profile_load_rx = None;
            match result {
                Ok(toml_content) => {
                    match toml::from_str::<echelle::EchelleCalibrationProfile>(&toml_content) {
                        Ok(mut profile) => {
                            let name = profile.display_name.clone();
                            // Patch frame dimensions to match active camera stream.
                            // Without this, validate_for_frame() rejects every frame
                            // with "frame size mismatch" and extraction silently fails.
                            // (Same patching that "Activate Editor" does in rendering.rs)
                            if self.width > 0 && self.height > 0 {
                                profile.compatibility.sensor_width = self.width;
                                profile.compatibility.sensor_height = self.height;
                                profile.compatibility.frame_width = self.width;
                                profile.compatibility.frame_height = self.height;
                            }
                            // Load into editor AND activate
                            self.echelle_cal_ui.editor_profile = Some(profile.clone());
                            self.echelle_cal_ui.editor_dirty = false;
                            self.echelle_cal_ui.status_message =
                                Some(format!("Loaded profile from daemon: {name}"));
                            self.echelle_cal_ui.last_error = None;
                            // Also activate it
                            self.echelle_profile_cache.activate_in_memory(profile);
                            self.mark_echelle_run_engine_sync_dirty();
                        }
                        Err(e) => {
                            self.echelle_cal_ui.last_error =
                                Some(format!("Failed to parse profile TOML: {e}"));
                        }
                    }
                }
                Err(e) => {
                    self.echelle_cal_ui.last_error = Some(e);
                }
            }
        }
    }

    pub(in crate::panels::image_viewer) fn mark_echelle_run_engine_sync_dirty(&mut self) {
        self.echelle_run_engine_sync_dirty = true;
    }

    pub(in crate::panels::image_viewer) fn build_echelle_run_engine_snapshot(
        &self,
    ) -> Option<protocol::daq::CalibrationSnapshot> {
        let profile = self.echelle_profile_cache.profile()?;
        let target_device_id = self.device_id.clone()?;
        Some(protocol::daq::CalibrationSnapshot {
            device_type: "spectroscopy".to_string(),
            target_device_id: Some(target_device_id),
            calibration_timestamp_rfc3339: None,
            grating_wavelength_coverage: Vec::new(),
            echelle_frame_compatibility: Some(protocol::daq::EchelleFrameCompatibility {
                sensor_width: profile.compatibility.sensor_width,
                sensor_height: profile.compatibility.sensor_height,
                frame_width: profile.compatibility.frame_width,
                frame_height: profile.compatibility.frame_height,
                roi_x: profile.compatibility.roi_x,
                roi_y: profile.compatibility.roi_y,
                binning_x: profile.compatibility.binning_x,
                binning_y: profile.compatibility.binning_y,
                bit_depth: profile.compatibility.bit_depth,
            }),
        })
    }

    pub(in crate::panels::image_viewer) fn sync_echelle_profile_to_run_engine(
        &mut self,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        if !self.echelle_run_engine_sync_dirty || self.echelle_run_engine_sync_in_flight {
            return;
        }
        let Some(client) = client else {
            return;
        };

        let snapshot = self.build_echelle_run_engine_snapshot();
        let action_tx = self.action_tx.clone();
        let mut client = client.clone();
        self.echelle_run_engine_sync_dirty = false;
        self.echelle_run_engine_sync_in_flight = true;

        runtime.spawn(async move {
            let result = if let Some(snapshot) = snapshot {
                client.set_calibration_snapshot(snapshot).await.map(|_| {
                    "Synced active echelle profile into RunEngine readiness gate".to_string()
                })
            } else {
                client
                    .clear_calibration_snapshot("spectroscopy", false, true)
                    .await
                    .map(|_| {
                        "Cleared echelle profile state from RunEngine readiness gate".to_string()
                    })
            };

            match result {
                Ok(message) => {
                    let _ = action_tx.send(ImageViewerAction::EchelleCalibrationSynced { message });
                }
                Err(error) => {
                    let _ = action_tx.send(ImageViewerAction::EchelleCalibrationSyncError(
                        format!("Failed to sync echelle calibration gate: {error}"),
                    ));
                }
            }
        });
    }

    pub(in crate::panels::image_viewer) fn ensure_echelle_calibration_editor_profile(&mut self) {
        if self.echelle_cal_ui.editor_profile.is_some() {
            return;
        }
        if let Some(profile) = self.echelle_profile_cache.profile() {
            self.echelle_cal_ui.editor_profile = Some((**profile).clone());
            self.echelle_cal_ui.editor_last_loaded_path =
                self.echelle_profile_cache.path().map(|p| p.to_path_buf());
            return;
        }
        // Wait for the first frame before creating a draft profile — otherwise
        // dimensions are 0x0 which produces a useless profile.
        if self.width == 0 || self.height == 0 {
            return;
        }
        self.echelle_cal_ui.editor_profile = Some(self.default_echelle_calibration_profile());
        self.echelle_cal_ui.editor_dirty = true;
        self.echelle_cal_ui.status_message =
            Some("Created new draft calibration profile from current frame metadata".to_string());
    }

    pub(in crate::panels::image_viewer) fn default_echelle_calibration_profile(
        &self,
    ) -> EchelleCalibrationProfile {
        // Try to load the embedded fixture. If the fixture's original dimensions
        // match the current camera, use it directly (real traces + wavelength cal).
        // Otherwise, fall back to a draft with traces at the frame center — the
        // fixture's trace positions are calibrated for a specific sensor and would
        // be out of bounds on a differently-sized camera.
        const FIXTURE_TOML: &str =
            include_str!("../../../../../common/tests/fixtures/echelle_profile_v1.toml");

        let frame_width = self.width.max(1);
        let frame_height = self.height.max(1);

        let mut profile =
            if let Ok(fixture) = toml::from_str::<EchelleCalibrationProfile>(FIXTURE_TOML) {
                // Use the fixture only if its frame dimensions are close enough
                // that traces won't be out of bounds.
                if fixture.compatibility.frame_height <= frame_height
                    && fixture.compatibility.frame_width <= frame_width
                {
                    fixture
                } else {
                    self.fallback_draft_profile()
                }
            } else {
                self.fallback_draft_profile()
            };

        // Patch dimensions to match the active camera stream
        profile.display_name = format!("Mechelle Draft {}x{}", frame_width, frame_height);
        profile.compatibility.sensor_width = frame_width;
        profile.compatibility.sensor_height = frame_height;
        profile.compatibility.frame_width = frame_width;
        profile.compatibility.frame_height = frame_height;
        profile.compatibility.bit_depth = (self.bit_depth > 0).then_some(self.bit_depth);
        profile.provenance.creator_tool = "rust-daq-image-viewer".to_string();
        profile.provenance.created_at_utc = chrono::Utc::now();
        profile.provenance.notes =
            Some("Draft created from Image Viewer calibration workspace".to_string());

        profile
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap
    )]
    // SAFETY: NUM_ORDERS is 10; loop index i ∈ [0,9]. All casts to f64/u32/i32 are exact.
    fn fallback_draft_profile(&self) -> EchelleCalibrationProfile {
        // Layout contract: these constants must match driver-mock's echelle pattern
        // generator (pattern.rs: ECHELLE_NUM_ORDERS, echelle_order_y_centers).
        const NUM_ORDERS: usize = 10;
        const WAVELENGTH_START_NM: f64 = 400.0;
        const WAVELENGTH_STEP_NM: f64 = 30.0;
        const FIRST_PHYSICAL_ORDER: i32 = 25;

        let frame_width = self.width.max(1);
        let frame_height = self.height.max(1);
        let sample_end = frame_width.saturating_sub(1);
        let domain_end = f64::from(sample_end) + 1.0;

        // Y centers: evenly spaced in middle 80% of frame (same formula as pattern.rs)
        let h = f64::from(frame_height);
        let margin = h * 0.1;
        let spacing = if NUM_ORDERS > 1 {
            (h * 0.8) / (NUM_ORDERS - 1) as f64
        } else {
            0.0
        };

        let orders: Vec<EchelleOrderCalibration> = (0..NUM_ORDERS)
            .map(|i| {
                let y_center = margin + i as f64 * spacing;
                let wl_start = WAVELENGTH_START_NM + i as f64 * WAVELENGTH_STEP_NM;
                let wl_scale = WAVELENGTH_STEP_NM / domain_end;

                EchelleOrderCalibration {
                    relative_index: i as u32,
                    physical_order_number: Some(FIRST_PHYSICAL_ORDER - i as i32),
                    sample_start: 0,
                    sample_end,
                    trace: EchelleTraceModel::Polynomial {
                        basis: PolynomialBasis::Monomial,
                        coefficients: vec![y_center],
                        domain_start: 0.0,
                        domain_end,
                    },
                    wavelength: EchelleWavelengthModel::Polynomial {
                        basis: PolynomialBasis::Monomial,
                        coefficients: vec![wl_start, wl_scale],
                        domain_start: 0.0,
                        domain_end,
                        unit: "nm".to_string(),
                    },
                    aperture_half_width_px: Some(4.0),
                    enabled: true,
                    notes: Some(format!(
                        "Synthetic order m={}, {:.0}-{:.0} nm",
                        FIRST_PHYSICAL_ORDER - i as i32,
                        wl_start,
                        wl_start + WAVELENGTH_STEP_NM
                    )),
                }
            })
            .collect();

        EchelleCalibrationProfile {
            schema_version: EchelleSchemaVersion::v1(),
            profile_id: None,
            display_name: format!("Synthetic Echelle {}x{}", frame_width, frame_height),
            compatibility: EchelleFrameCompatibility {
                sensor_width: frame_width,
                sensor_height: frame_height,
                frame_width,
                frame_height,
                roi_x: 0,
                roi_y: 0,
                binning_x: 1,
                binning_y: 1,
                bit_depth: (self.bit_depth > 0).then_some(self.bit_depth),
            },
            orientation: EchelleOrientation {
                dispersion_axis: DetectorAxis::X,
                cross_dispersion_axis: DetectorAxis::Y,
                order_number_increase_direction: AxisDirection::Positive,
                wavelength_increase_with_dispersion_positive: true,
            },
            extraction: EchelleExtractionConfig {
                summation_mode: EchelleSummationMode::SimpleSum,
                default_aperture_half_width_px: 4.0,
                background: None,
            },
            orders,
            corrections: Default::default(),
            provenance: EchelleProvenance {
                creator_tool: "rust-daq-image-viewer".to_string(),
                creator_version: None,
                created_at_utc: chrono::Utc::now(),
                source_frame_ids: Vec::new(),
                notes: Some("Fallback draft profile matching mock echelle pattern".to_string()),
            },
        }
    }

    pub(in crate::panels::image_viewer) fn mark_echelle_editor_dirty(&mut self) {
        self.echelle_cal_ui.editor_dirty = true;
        self.echelle_cal_ui.last_error = None;
    }

    pub(in crate::panels::image_viewer) fn save_echelle_editor_profile_to_path(
        &mut self,
        activate_after_save: bool,
    ) -> Result<std::path::PathBuf, String> {
        let mut profile = self
            .echelle_cal_ui
            .editor_profile
            .clone()
            .ok_or_else(|| "No calibration profile in editor".to_string())?;
        profile.provenance.created_at_utc = chrono::Utc::now();
        let mut note = String::from("Saved from Image Viewer calibration workspace");
        if let Some(existing) = profile.provenance.notes.as_deref() {
            if !existing.trim().is_empty() && !existing.contains("Saved from Image Viewer") {
                note = format!("{existing}\n{note}");
            } else if !existing.trim().is_empty() {
                note = existing.to_string();
            }
        }
        profile.provenance.notes = Some(note);
        let path_text = self.echelle_cal_ui.save_as_path_text.trim();
        if path_text.is_empty() {
            return Err("Enter a .toml or .json path to save the calibration profile".to_string());
        }
        let path = std::path::PathBuf::from(path_text);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create calibration profile directory {}: {e}",
                        parent.display()
                    )
                })?;
            }
        }
        profile
            .save_to_path(&path)
            .map_err(|e| format!("Failed to save calibration profile {}: {e}", path.display()))?;
        self.echelle_cal_ui.editor_dirty = false;
        self.echelle_cal_ui.editor_last_loaded_path = Some(path.clone());
        self.echelle_cal_ui.status_message =
            Some(format!("Saved calibration profile to {}", path.display()));
        self.echelle_cal_ui.last_error = None;
        if activate_after_save {
            self.set_echelle_profile_path(path.clone());
            self.poll_echelle_profile_cache();
        }
        Ok(path)
    }

    pub(in crate::panels::image_viewer) fn import_echelle_calibration_points_from_path(
        &mut self,
        path: &std::path::Path,
    ) -> Result<usize, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        let items = serde_json::from_str::<Vec<EchelleCalibrationPointUi>>(&text)
            .map_err(|e| format!("Failed to parse calibration points JSON: {e}"))?;
        let count = items.len();
        self.echelle_cal_ui.calibration_points = items;
        self.echelle_cal_ui.selected_point_idx = self
            .echelle_cal_ui
            .selected_point_idx
            .min(count.saturating_sub(1));
        Ok(count)
    }

    pub(in crate::panels::image_viewer) fn export_echelle_calibration_points_to_path(
        &self,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.echelle_cal_ui.calibration_points)
            .map_err(|e| format!("Failed to serialize calibration points: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("Failed to write {}: {e}", path.display()))
    }

    pub(in crate::panels::image_viewer) fn import_echelle_line_list_from_path(
        &mut self,
        path: &std::path::Path,
    ) -> Result<usize, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        let items = serde_json::from_str::<Vec<EchelleLineListEntryUi>>(&text)
            .map_err(|e| format!("Failed to parse line list JSON: {e}"))?;
        let count = items.len();
        self.echelle_cal_ui.line_list = items;
        Ok(count)
    }

    pub(in crate::panels::image_viewer) fn export_echelle_line_list_to_path(
        &self,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.echelle_cal_ui.line_list)
            .map_err(|e| format!("Failed to serialize line list: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("Failed to write {}: {e}", path.display()))
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(in crate::panels::image_viewer) fn auto_detect_trace_seeds_from_current_frame(
        &mut self,
    ) -> Result<usize, String> {
        self.ensure_echelle_calibration_editor_profile();
        let frame = self
            .last_frame_data
            .as_ref()
            .ok_or_else(|| "No frame available for auto-detect".to_string())?;
        let profile = self
            .echelle_cal_ui
            .editor_profile
            .as_ref()
            .ok_or_else(|| "No editor profile loaded".to_string())?;

        let centers = detect_cross_dispersion_peaks_from_frame(
            frame.as_ref(),
            self.width,
            self.height,
            self.bit_depth,
            profile.orientation.dispersion_axis,
            self.echelle_cal_ui
                .trace_auto_detect_min_separation_px
                .max(1),
            self.echelle_cal_ui.trace_overlay_max_orders.max(1) as usize,
            self.echelle_cal_ui
                .trace_auto_detect_threshold_fraction
                .clamp(0.01, 0.95),
        )?;
        if centers.is_empty() {
            return Err("No candidate order peaks detected in the current frame".to_string());
        }

        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
            let dispersion_len = match profile.orientation.dispersion_axis {
                DetectorAxis::X => profile.compatibility.frame_width.max(1),
                DetectorAxis::Y => profile.compatibility.frame_height.max(1),
            };
            let cross_roi_offset = match profile.orientation.cross_dispersion_axis {
                DetectorAxis::X => f64::from(profile.compatibility.roi_x),
                DetectorAxis::Y => f64::from(profile.compatibility.roi_y),
            };
            let template_wavelength = profile
                .orders
                .get(self.echelle_cal_ui.selected_order_edit_idx)
                .map(|o| o.wavelength.clone())
                .or_else(|| profile.orders.first().map(|o| o.wavelength.clone()))
                .unwrap_or(EchelleWavelengthModel::Polynomial {
                    basis: PolynomialBasis::Monomial,
                    coefficients: vec![0.0, 1.0],
                    domain_start: 0.0,
                    domain_end: f64::from(dispersion_len.saturating_sub(1)) + 1.0,
                    unit: "nm".to_string(),
                });

            let mut new_orders = Vec::with_capacity(centers.len());
            for (idx, center_local) in centers.iter().enumerate() {
                new_orders.push(EchelleOrderCalibration {
                    relative_index: idx as u32,
                    physical_order_number: None,
                    sample_start: 0,
                    sample_end: dispersion_len.saturating_sub(1),
                    trace: EchelleTraceModel::Polynomial {
                        basis: PolynomialBasis::Monomial,
                        coefficients: vec![*center_local + cross_roi_offset],
                        domain_start: 0.0,
                        domain_end: f64::from(dispersion_len.saturating_sub(1)) + 1.0,
                    },
                    wavelength: template_wavelength.clone(),
                    aperture_half_width_px: None,
                    enabled: true,
                    notes: Some("Auto-detected constant trace seed from frame peaks".to_string()),
                });
            }
            profile.orders = new_orders;
        }
        self.echelle_cal_ui.selected_order_edit_idx = 0;
        self.mark_echelle_editor_dirty();
        Ok(centers.len())
    }

    pub(in crate::panels::image_viewer) fn export_selected_order_blaze_preview_artifact(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let preview = self
            .echelle_preview
            .as_ref()
            .ok_or_else(|| "No extracted preview is available".to_string())?;
        let order = preview
            .orders
            .get(self.echelle_selected_order_plot)
            .ok_or_else(|| "No selected order preview available".to_string())?;
        if order.flux.is_empty() {
            return Err("Selected order preview has empty flux".to_string());
        }

        let max_flux = order
            .flux
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
            .max(1e-12);
        use std::fmt::Write;
        let mut out = String::from("sample,wavelength,raw_flux,normalized_flux\n");
        for (i, (&w, &f)) in order.wavelengths.iter().zip(&order.flux).enumerate() {
            let _ = writeln!(out, "{i},{w},{f},{}", f / max_flux);
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create blaze artifact directory {}: {e}",
                        parent.display()
                    )
                })?;
            }
        }
        std::fs::write(path, out)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;

        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
            profile.corrections.blaze = Some(EchelleArtifactRef {
                path: path.display().to_string(),
                sha256: None,
                format: Some("csv".to_string()),
            });
            self.mark_echelle_editor_dirty();
        }
        Ok(())
    }
}
