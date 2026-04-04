//! Echelle calibration UI rendering - workspace tabs and overlays.

use super::super::*;

impl ImageViewerPanel {
    pub(in crate::panels::image_viewer) fn render_echelle_calibration_workspace(
        &mut self,
        ui: &mut egui::Ui,
    ) {
        egui::CollapsingHeader::new("Calibration Workspace (Mechelle / Echelle)")
            .default_open(true)
            .show(ui, |ui| {
                ui.small(
                    "Author and validate echelle calibration profiles, order traces, arc picks, and fit diagnostics.",
                );

                ui.horizontal_wrapped(|ui| {
                    for (tab, label) in [
                        (EchelleCalibrationTab::Profile, "Profile"),
                        (EchelleCalibrationTab::Trace, "Trace"),
                        (EchelleCalibrationTab::LinePoints, "Arc/Points"),
                        (EchelleCalibrationTab::WavelengthFit, "Wavelength Fit"),
                        (EchelleCalibrationTab::BlazeFlat, "Blaze/Flat"),
                        (EchelleCalibrationTab::MechelleNotes, "Mechelle UX"),
                    ] {
                        ui.selectable_value(&mut self.echelle_cal_ui.tab, tab, label);
                    }
                });

                // Show remote load state machine status when active (bd-zy7y.1),
                // otherwise fall back to the static status message.
                if let Some(load_msg) = self
                    .remote_profile_save
                    .status_message()
                    .or_else(|| self.remote_profile_load.status_message())
                {
                    if self.remote_profile_save.is_busy() || self.remote_profile_load.is_busy() {
                        ui.spinner();
                    }
                    ui.small(&load_msg);
                } else if let Some(msg) = &self.echelle_cal_ui.status_message {
                    ui.small(msg);
                }
                if let Some(err) = &self.echelle_cal_ui.last_error {
                    ui.colored_label(colors::ERROR, err);
                }

                ui.separator();

                let mut trigger_load_editor = false;
                let mut trigger_save_editor = false;
                let mut trigger_save_activate = false;
                let mut trigger_activate_only = false;
                let mut trigger_activate_editor = false;

                ui.horizontal_wrapped(|ui| {
                    ui.label("Profile path:");
                    ui.text_edit_singleline(&mut self.echelle_cal_ui.save_as_path_text);
                    ui.menu_button("Recent…", |ui| {
                        let recent = self.echelle_cal_ui.recent_profile_paths.clone();
                        if recent.is_empty() {
                            ui.weak("No recent paths yet");
                        } else {
                            for p in recent {
                                if ui.button(egui::RichText::new(&p).monospace()).clicked() {
                                    self.echelle_cal_ui.save_as_path_text.clone_from(&p);
                                    ui.close();
                                }
                            }
                        }
                    });
                    if ui.button("Load Editor").clicked() {
                        trigger_load_editor = true;
                    }
                    if ui.button("Save").clicked() {
                        trigger_save_editor = true;
                    }
                    if ui.button("Save + Activate").clicked() {
                        trigger_save_activate = true;
                    }
                    if ui.button("Activate Path").clicked() {
                        trigger_activate_only = true;
                    }
                    // Activate editor profile in-memory (works in WASM without filesystem)
                    if ui
                        .add_enabled(
                            self.echelle_cal_ui.editor_profile.is_some(),
                            egui::Button::new("Activate Editor"),
                        )
                        .on_hover_text(
                            "Activate the editor profile in-memory (no file save needed)",
                        )
                        .clicked()
                    {
                        trigger_activate_editor = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Clone Active -> Editor").clicked() {
                        if let Some(profile) = self.echelle_profile_cache.profile() {
                            self.echelle_cal_ui.editor_profile = Some((**profile).clone());
                            self.echelle_cal_ui.editor_dirty = false;
                            self.echelle_cal_ui.editor_last_loaded_path =
                                self.echelle_profile_cache.path().map(|p| p.to_path_buf());
                            if self.echelle_cal_ui.save_as_path_text.is_empty()
                                && let Some(path) = self.echelle_profile_cache.path() {
                                    let s = path.display().to_string();
                                    self.echelle_cal_ui.save_as_path_text.clone_from(&s);
                                    self.echelle_cal_ui.record_recent_profile_path(&s);
                                }
                            self.echelle_cal_ui.status_message =
                                Some("Editor cloned from active profile".to_string());
                            self.echelle_cal_ui.last_error = None;
                        } else {
                            self.echelle_cal_ui.last_error =
                                Some("No active profile available to clone".to_string());
                        }
                    }
                    if ui.button("New Draft From Frame").clicked() {
                        self.echelle_cal_ui.editor_profile = Some(self.default_echelle_calibration_profile());
                        self.echelle_cal_ui.editor_dirty = true;
                        self.echelle_cal_ui.editor_last_loaded_path = None;
                        self.echelle_cal_ui.status_message =
                            Some("Created new draft calibration profile".to_string());
                        self.echelle_cal_ui.last_error = None;
                    }
                    if ui.button("Reset Editor From Active").clicked()
                        && let Some(profile) = self.echelle_profile_cache.profile() {
                            self.echelle_cal_ui.editor_profile = Some((**profile).clone());
                            self.echelle_cal_ui.editor_dirty = false;
                            self.echelle_cal_ui.status_message =
                                Some("Editor reset from active profile".to_string());
                            self.echelle_cal_ui.last_error = None;
                        }
                    if let Some(path) = &self.echelle_cal_ui.editor_last_loaded_path {
                        ui.small(format!("Editor source: {}", path.display()));
                    } else {
                        ui.small("Editor source: draft");
                    }
                    if self.echelle_cal_ui.editor_dirty {
                        ui.colored_label(colors::WARNING, "Unsaved editor changes");
                    }
                });

                if trigger_load_editor || trigger_activate_only {
                    let path_text = self.echelle_cal_ui.save_as_path_text.trim().to_string();
                    if path_text.is_empty() {
                        self.echelle_cal_ui.last_error = Some(if trigger_load_editor {
                            "Enter a profile path before loading".to_string()
                        } else {
                            "Enter a profile path before activation".to_string()
                        });
                    } else if self.remote_profile_load.is_busy() {
                        self.echelle_cal_ui.last_error =
                            Some("A profile load is already in progress".to_string());
                    } else if self.remote_profile_save.is_busy() {
                        self.echelle_cal_ui.last_error =
                            Some("A profile save is already in progress".to_string());
                    } else {
                        // Transition to Pending; rendering.rs will pick this up and start gRPC call
                        self.remote_profile_load =
                            RemoteProfileLoadState::Pending { path: path_text };
                        self.echelle_cal_ui.last_error = None;
                    }
                }
                if trigger_save_editor
                    && let Err(err) = self.save_echelle_editor_profile_to_path(false) {
                        self.echelle_cal_ui.last_error = Some(err);
                    }
                if trigger_save_activate
                    && let Err(err) = self.save_echelle_editor_profile_to_path(true) {
                        self.echelle_cal_ui.last_error = Some(err);
                    }
                if trigger_activate_editor
                    && let Some(profile) = self.echelle_cal_ui.editor_profile.clone() {
                        // Do NOT patch compatibility dimensions — the profile
                        // carries its own frame/sensor geometry from calibration.
                        self.echelle_profile_cache.activate_in_memory(profile);
                        self.mark_echelle_run_engine_sync_dirty();
                        // Reset Y zoom and saved bounds so first frame auto-fits (bd-zy7y.4).
                        self.echelle_plot_y_locked = false;
                        self.echelle_plot_saved_y = None;
                        self.echelle_sidebar_plot_y_locked = false;
                        self.echelle_sidebar_saved_y = None;
                        self.echelle_plot_last_rendered = None;
                        self.echelle_cal_ui.status_message =
                            Some("Editor profile activated in-memory".to_string());
                        self.echelle_cal_ui.last_error = None;
                    }

                match self.echelle_cal_ui.tab {
                    EchelleCalibrationTab::Profile => self.render_echelle_calibration_profile_tab(ui),
                    EchelleCalibrationTab::Trace => self.render_echelle_calibration_trace_tab(ui),
                    EchelleCalibrationTab::LinePoints => {
                        self.render_echelle_calibration_line_points_tab(ui);
                    }
                    EchelleCalibrationTab::WavelengthFit => {
                        self.render_echelle_calibration_wavelength_fit_tab(ui);
                    }
                    EchelleCalibrationTab::BlazeFlat => self.render_echelle_calibration_blaze_tab(ui),
                    EchelleCalibrationTab::MechelleNotes => {
                        self.render_echelle_calibration_mechelle_notes_tab(ui);
                    }
                }
            });
    }

    pub(in crate::panels::image_viewer) fn render_echelle_calibration_profile_tab(
        &mut self,
        ui: &mut egui::Ui,
    ) {
        self.ensure_echelle_calibration_editor_profile();
        let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() else {
            ui.weak("No editor profile loaded.");
            return;
        };

        let mut changed = false;
        egui::Grid::new("echelle_cal_profile_grid")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Display name");
                changed |= ui.text_edit_singleline(&mut profile.display_name).changed();
                ui.end_row();

                ui.label("Profile ID");
                let mut id_text = profile.profile_id.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut id_text).changed() {
                    profile.profile_id = if id_text.trim().is_empty() {
                        None
                    } else {
                        Some(id_text)
                    };
                    changed = true;
                }
                ui.end_row();

                ui.label("Schema");
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(egui::DragValue::new(&mut profile.schema_version.major).range(1..=9))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut profile.schema_version.minor).range(0..=99))
                        .changed();
                    changed |= ui
                        .add(egui::DragValue::new(&mut profile.schema_version.patch).range(0..=99))
                        .changed();
                });
                ui.end_row();

                ui.label("Compatibility");
                ui.horizontal_wrapped(|ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.frame_width)
                                .range(1..=8192)
                                .prefix("frame_w "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.frame_height)
                                .range(1..=8192)
                                .prefix("frame_h "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.roi_x)
                                .range(0..=8192)
                                .prefix("roi_x "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.roi_y)
                                .range(0..=8192)
                                .prefix("roi_y "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.binning_x)
                                .range(1..=16)
                                .prefix("bin_x "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut profile.compatibility.binning_y)
                                .range(1..=16)
                                .prefix("bin_y "),
                        )
                        .changed();
                });
                ui.end_row();

                ui.label("Provenance");
                ui.vertical(|ui| {
                    changed |= ui
                        .text_edit_singleline(&mut profile.provenance.creator_tool)
                        .changed();
                    let mut version = profile
                        .provenance
                        .creator_version
                        .clone()
                        .unwrap_or_default();
                    if ui.text_edit_singleline(&mut version).changed() {
                        profile.provenance.creator_version = if version.trim().is_empty() {
                            None
                        } else {
                            Some(version)
                        };
                        changed = true;
                    }
                    let notes = profile.provenance.notes.get_or_insert_with(String::new);
                    changed |= ui.text_edit_multiline(notes).changed();
                    ui.small(format!(
                        "created_at_utc: {}",
                        profile.provenance.created_at_utc
                    ));
                });
                ui.end_row();
            });

        ui.separator();
        match profile.validate() {
            Ok(()) => ui.colored_label(colors::SUCCESS, "Profile validates"),
            Err(err) => ui.colored_label(colors::WARNING, format!("Validation issue: {err}")),
        };

        if changed {
            self.mark_echelle_editor_dirty();
        }
    }

    pub(in crate::panels::image_viewer) fn render_echelle_calibration_trace_tab(
        &mut self,
        ui: &mut egui::Ui,
    ) {
        self.ensure_echelle_calibration_editor_profile();

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.echelle_cal_ui.trace_overlay_enabled,
                "Show trace overlays on image",
            );
            ui.checkbox(
                &mut self.echelle_cal_ui.trace_overlay_all_orders,
                "All orders",
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_overlay_sample_step)
                    .range(1..=256)
                    .prefix("step "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_overlay_max_orders)
                    .range(1..=256)
                    .prefix("max "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_nudge_px)
                    .range(0.01..=20.0)
                    .speed(0.05)
                    .prefix("nudge "),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_auto_detect_min_separation_px)
                    .range(1..=512)
                    .prefix("auto min-sep "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.trace_auto_detect_threshold_fraction)
                    .range(0.01..=0.95)
                    .speed(0.01)
                    .prefix("auto thr "),
            );
            if ui
                .button("Auto-Detect Trace Seeds From Current Frame")
                .clicked()
            {
                match self.auto_detect_trace_seeds_from_current_frame() {
                    Ok(count) => {
                        self.echelle_cal_ui.status_message = Some(format!(
                            "Auto-detected {count} trace seed(s) from current frame"
                        ));
                        self.echelle_cal_ui.last_error = None;
                    }
                    Err(err) => self.echelle_cal_ui.last_error = Some(err),
                }
            }
            ui.small("Creates constant-trace seeds from cross-dispersion peaks; refine manually.");
        });

        let Some(profile_ref) = self.echelle_cal_ui.editor_profile.as_ref() else {
            ui.weak("No editor profile loaded.");
            return;
        };

        let order_labels: Vec<String> = profile_ref
            .orders
            .iter()
            .enumerate()
            .map(|(idx, order)| {
                format!(
                    "#{idx} rel={}{}{}",
                    order.relative_index,
                    order
                        .physical_order_number
                        .map(|m| format!(" m={m}"))
                        .unwrap_or_default(),
                    if order.enabled { "" } else { " (disabled)" }
                )
            })
            .collect();

        if order_labels.is_empty() {
            ui.weak("No orders in editor profile.");
            return;
        }

        if self.echelle_cal_ui.selected_order_edit_idx >= order_labels.len() {
            self.echelle_cal_ui.selected_order_edit_idx = 0;
        }

        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("echelle_cal_trace_order_select")
                .selected_text(
                    order_labels
                        .get(self.echelle_cal_ui.selected_order_edit_idx)
                        .cloned()
                        .unwrap_or_else(|| "order".to_string()),
                )
                .show_ui(ui, |ui| {
                    for (idx, label) in order_labels.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.echelle_cal_ui.selected_order_edit_idx,
                            idx,
                            label,
                        );
                    }
                });
            if ui.button("Add Order (Clone)").clicked()
                && let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut()
            {
                let selected = self
                    .echelle_cal_ui
                    .selected_order_edit_idx
                    .min(profile.orders.len() - 1);
                let mut new_order = profile.orders[selected].clone();
                let next_rel = profile
                    .orders
                    .iter()
                    .map(|o| o.relative_index)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                new_order.relative_index = next_rel;
                new_order.physical_order_number = None;
                new_order.notes = Some("Cloned for manual trace adjustment".to_string());
                profile.orders.push(new_order);
                self.echelle_cal_ui.selected_order_edit_idx = profile.orders.len() - 1;
                self.mark_echelle_editor_dirty();
            }
            if ui.button("Remove Selected Order").clicked()
                && let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut()
            {
                if profile.orders.len() > 1 {
                    let idx = self
                        .echelle_cal_ui
                        .selected_order_edit_idx
                        .min(profile.orders.len() - 1);
                    profile.orders.remove(idx);
                    self.echelle_cal_ui.selected_order_edit_idx = self
                        .echelle_cal_ui
                        .selected_order_edit_idx
                        .min(profile.orders.len().saturating_sub(1));
                    self.mark_echelle_editor_dirty();
                } else {
                    self.echelle_cal_ui.last_error =
                        Some("Profile must contain at least one order".to_string());
                }
            }
        });

        let selected_idx = self.echelle_cal_ui.selected_order_edit_idx;
        let mut changed = false;
        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut()
            && let Some(order) = profile.orders.get_mut(selected_idx)
        {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                changed |= ui.checkbox(&mut order.enabled, "Enabled").changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut order.relative_index)
                            .range(0..=999)
                            .prefix("rel "),
                    )
                    .changed();
                let mut physical_text = order
                    .physical_order_number
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                ui.label("m:");
                if ui.text_edit_singleline(&mut physical_text).changed() {
                    order.physical_order_number = if physical_text.trim().is_empty() {
                        None
                    } else {
                        physical_text.parse::<i32>().ok()
                    };
                    changed = true;
                }
            });

            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut order.sample_start)
                            .range(0..=8191)
                            .prefix("sample_start "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut order.sample_end)
                            .range(0..=8191)
                            .prefix("sample_end "),
                    )
                    .changed();
                let mut aperture_enabled = order.aperture_half_width_px.is_some();
                if ui
                    .checkbox(&mut aperture_enabled, "Order aperture override")
                    .changed()
                {
                    if aperture_enabled && order.aperture_half_width_px.is_none() {
                        order.aperture_half_width_px = Some(4.0);
                    }
                    if !aperture_enabled {
                        order.aperture_half_width_px = None;
                    }
                    changed = true;
                }
                if let Some(ap) = &mut order.aperture_half_width_px {
                    changed |= ui
                        .add(
                            egui::DragValue::new(ap)
                                .range(0.1..=128.0)
                                .speed(0.1)
                                .prefix("half-width "),
                        )
                        .changed();
                }
            });

            let notes = order.notes.get_or_insert_with(String::new);
            changed |= ui.text_edit_multiline(notes).changed();

            let EchelleTraceModel::Polynomial {
                basis,
                coefficients,
                domain_start,
                domain_end,
            } = &mut order.trace;

            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label("Trace basis:");
                ui.selectable_value(basis, PolynomialBasis::Monomial, "Monomial");
                ui.selectable_value(basis, PolynomialBasis::Chebyshev, "Chebyshev");
                changed |= ui
                    .add(
                        egui::DragValue::new(domain_start)
                            .speed(0.5)
                            .prefix("domain_start "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(domain_end)
                            .speed(0.5)
                            .prefix("domain_end "),
                    )
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Nudge -Y").clicked()
                    && let Some(c0) = coefficients.first_mut()
                {
                    *c0 -= self.echelle_cal_ui.trace_nudge_px;
                    changed = true;
                }
                if ui.button("Nudge +Y").clicked()
                    && let Some(c0) = coefficients.first_mut()
                {
                    *c0 += self.echelle_cal_ui.trace_nudge_px;
                    changed = true;
                }
                if ui.button("Add Coeff").clicked() {
                    coefficients.push(0.0);
                    changed = true;
                }
                if ui.button("Pop Coeff").clicked() && coefficients.len() > 1 {
                    coefficients.pop();
                    changed = true;
                }
            });
            egui::Grid::new("echelle_cal_trace_coeff_grid").show(ui, |ui| {
                for (i, coeff) in coefficients.iter_mut().enumerate() {
                    ui.label(format!("c{i}"));
                    changed |= ui.add(egui::DragValue::new(coeff).speed(0.01)).changed();
                    ui.end_row();
                }
            });
        }

        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_ref() {
            match profile.validate() {
                Ok(()) => ui.colored_label(colors::SUCCESS, "Trace/order edits validate"),
                Err(err) => ui.colored_label(colors::WARNING, format!("Validation issue: {err}")),
            };
        }
        if changed {
            self.mark_echelle_editor_dirty();
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub(in crate::panels::image_viewer) fn render_echelle_calibration_line_points_tab(
        &mut self,
        ui: &mut egui::Ui,
    ) {
        self.ensure_echelle_calibration_editor_profile();

        ui.horizontal_wrapped(|ui| {
            ui.label("Points JSON:");
            ui.text_edit_singleline(&mut self.echelle_cal_ui.points_path_text);
            if ui.button("Import Points").clicked() {
                let path_text = self.echelle_cal_ui.points_path_text.trim().to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error = Some("Enter a points JSON path".to_string());
                } else {
                    match self.import_echelle_calibration_points_from_path(std::path::Path::new(
                        &path_text,
                    )) {
                        Ok(count) => {
                            self.echelle_cal_ui.status_message =
                                Some(format!("Imported {count} calibration points"));
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
            if ui.button("Export Points").clicked() {
                let path_text = self.echelle_cal_ui.points_path_text.trim().to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error = Some("Enter a points JSON path".to_string());
                } else {
                    match self
                        .export_echelle_calibration_points_to_path(std::path::Path::new(&path_text))
                    {
                        Ok(()) => {
                            self.echelle_cal_ui.status_message =
                                Some("Exported calibration points".to_string());
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Line list JSON:");
            ui.text_edit_singleline(&mut self.echelle_cal_ui.line_list_path_text);
            if ui.button("Import Line List").clicked() {
                let path_text = self.echelle_cal_ui.line_list_path_text.trim().to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error =
                        Some("Enter a line list JSON path".to_string());
                } else {
                    match self.import_echelle_line_list_from_path(std::path::Path::new(&path_text))
                    {
                        Ok(count) => {
                            self.echelle_cal_ui.status_message =
                                Some(format!("Imported {count} line-list entries"));
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
            if ui.button("Export Line List").clicked() {
                let path_text = self.echelle_cal_ui.line_list_path_text.trim().to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error =
                        Some("Enter a line list JSON path".to_string());
                } else {
                    match self.export_echelle_line_list_to_path(std::path::Path::new(&path_text)) {
                        Ok(()) => {
                            self.echelle_cal_ui.status_message =
                                Some("Exported line list".to_string());
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
        });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add Point").clicked() {
                let (order_relative_index, x_sample, y_pixel, wavelength) =
                    if let Some(link) = self.echelle_plot_hover_link {
                        (
                            link.relative_index,
                            link.sample_index as f64,
                            0.0,
                            link.wavelength,
                        )
                    } else {
                        (0, 0.0, 0.0, 0.0)
                    };
                self.echelle_cal_ui
                    .calibration_points
                    .push(EchelleCalibrationPointUi {
                        enabled: true,
                        order_relative_index,
                        x_sample,
                        y_pixel,
                        wavelength,
                        note: String::new(),
                    });
                self.echelle_cal_ui.selected_point_idx = self
                    .echelle_cal_ui
                    .calibration_points
                    .len()
                    .saturating_sub(1);
            }
            if ui.button("Remove Point").clicked()
                && !self.echelle_cal_ui.calibration_points.is_empty()
            {
                let idx = self
                    .echelle_cal_ui
                    .selected_point_idx
                    .min(self.echelle_cal_ui.calibration_points.len() - 1);
                self.echelle_cal_ui.calibration_points.remove(idx);
                self.echelle_cal_ui.selected_point_idx =
                    self.echelle_cal_ui.selected_point_idx.min(
                        self.echelle_cal_ui
                            .calibration_points
                            .len()
                            .saturating_sub(1),
                    );
            }
            ui.small(format!(
                "{} calibration points",
                self.echelle_cal_ui.calibration_points.len()
            ));
        });
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                egui::Grid::new("echelle_cal_points_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Sel");
                        ui.strong("On");
                        ui.strong("Order");
                        ui.strong("x");
                        ui.strong("y");
                        ui.strong("\u{03bb}");
                        ui.strong("Note");
                        ui.end_row();
                        for (idx, p) in self
                            .echelle_cal_ui
                            .calibration_points
                            .iter_mut()
                            .enumerate()
                        {
                            ui.selectable_value(
                                &mut self.echelle_cal_ui.selected_point_idx,
                                idx,
                                "\u{2022}",
                            );
                            ui.checkbox(&mut p.enabled, "");
                            ui.add(
                                egui::DragValue::new(&mut p.order_relative_index).range(0..=999),
                            );
                            ui.add(egui::DragValue::new(&mut p.x_sample).speed(0.1));
                            ui.add(egui::DragValue::new(&mut p.y_pixel).speed(0.1));
                            ui.add(egui::DragValue::new(&mut p.wavelength).speed(0.001));
                            ui.text_edit_singleline(&mut p.note);
                            ui.end_row();
                        }
                    });
            });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add Line").clicked() {
                self.echelle_cal_ui.line_list.push(EchelleLineListEntryUi {
                    enabled: true,
                    wavelength: 0.0,
                    label: String::new(),
                });
            }
            if ui.button("Remove Line").clicked() && !self.echelle_cal_ui.line_list.is_empty() {
                self.echelle_cal_ui.line_list.pop();
            }
            ui.small(format!(
                "{} line-list entries",
                self.echelle_cal_ui.line_list.len()
            ));
        });
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                egui::Grid::new("echelle_cal_line_list_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("On");
                        ui.strong("\u{03bb}");
                        ui.strong("Label");
                        ui.end_row();
                        for line in &mut self.echelle_cal_ui.line_list {
                            ui.checkbox(&mut line.enabled, "");
                            ui.add(egui::DragValue::new(&mut line.wavelength).speed(0.001));
                            ui.text_edit_singleline(&mut line.label);
                            ui.end_row();
                        }
                    });
            });
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::too_many_lines
    )]
    pub(in crate::panels::image_viewer) fn render_echelle_calibration_wavelength_fit_tab(
        &mut self,
        ui: &mut egui::Ui,
    ) {
        self.ensure_echelle_calibration_editor_profile();
        let Some(_) = self.echelle_cal_ui.editor_profile.as_ref() else {
            ui.weak("No editor profile loaded.");
            return;
        };

        // ── Section 1: Manual LSQ Fit (legacy) ──────────────────────────
        let mut trigger_fit_selected = false;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.fit_outlier_sigma)
                    .range(0.5..=10.0)
                    .speed(0.1)
                    .prefix("outlier \u{03c3} "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.fit_rms_acceptance_px)
                    .range(0.01..=10.0)
                    .speed(0.01)
                    .prefix("accept RMS "),
            );
            if ui.button("Fit Selected Order (LSQ)").clicked() {
                trigger_fit_selected = true;
            }
            ui.small("Manual picks (see arc-line auto-match below).");
        });

        if trigger_fit_selected {
            let selected_idx = self.echelle_cal_ui.selected_order_edit_idx;
            let points = self.echelle_cal_ui.calibration_points.clone();
            let fit_result = if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
                if let Some(order) = profile.orders.get_mut(selected_idx) {
                    fit_wavelength_model_for_order_from_points(order, &points)
                } else {
                    Err("Selected order is out of range".to_string())
                }
            } else {
                Err("No editor profile loaded".to_string())
            };
            match fit_result {
                Ok(summary) => {
                    self.echelle_cal_ui.status_message = Some(summary);
                    self.echelle_cal_ui.last_error = None;
                    self.mark_echelle_editor_dirty();
                }
                Err(err) => {
                    self.echelle_cal_ui.last_error = Some(err);
                }
            }
        }

        // ── Section 2: Arc Line Detection (bd-a64a) ─────────────────────
        ui.separator();
        ui.strong("Arc Line Auto-Detection & Matching");

        let mut trigger_detect = false;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.arc_detect_config.sigdetect)
                    .range(1.0..=50.0)
                    .speed(0.5)
                    .prefix("SNR "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.arc_detect_config.min_fwhm)
                    .range(0.5..=10.0)
                    .speed(0.1)
                    .prefix("FWHM min "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.arc_detect_config.max_fwhm)
                    .range(1.0..=20.0)
                    .speed(0.1)
                    .prefix("FWHM max "),
            );
            if ui.button("Detect Arc Lines").clicked() {
                trigger_detect = true;
            }
        });

        if trigger_detect {
            // Get the 1D spectrum for the selected order from the extraction preview.
            let selected_order_idx = self.echelle_selected_order_plot;
            let spectrum_opt = self.echelle_preview.as_ref().and_then(|preview| {
                preview.orders.get(selected_order_idx).map(|order| {
                    (
                        order.relative_index,
                        order.flux.iter().map(|&v| v as f32).collect::<Vec<f32>>(),
                    )
                })
            });
            match spectrum_opt {
                Some((rel_idx, spectrum)) if !spectrum.is_empty() => {
                    let config = self.echelle_cal_ui.arc_detect_config.clone();
                    let lines = detect_arc_lines(&spectrum, rel_idx, &config);
                    self.echelle_cal_ui.status_message = Some(format!(
                        "Detected {} arc lines on order {rel_idx}",
                        lines.len()
                    ));
                    self.echelle_cal_ui.detected_arc_lines = lines;
                    self.echelle_cal_ui.last_error = None;
                    // Clear downstream state when detection changes.
                    self.echelle_cal_ui.matched_pairs.clear();
                    self.echelle_cal_ui.wl_fit_solution = None;
                }
                _ => {
                    self.echelle_cal_ui.last_error = Some(
                        "No extracted spectrum available. Stream frames and enable echelle extraction first."
                            .to_string(),
                    );
                }
            }
        }

        // Show detected lines summary.
        if !self.echelle_cal_ui.detected_arc_lines.is_empty() {
            let n = self.echelle_cal_ui.detected_arc_lines.len();
            ui.small(format!(
                "{n} detected arc lines (order {})",
                self.echelle_cal_ui
                    .detected_arc_lines
                    .first()
                    .map_or(0, |l| l.order)
            ));
        }

        // ── Section 3: Atlas Matching (bd-a64a) ─────────────────────────
        let mut trigger_match = false;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.atlas_match_tolerance_nm)
                    .range(0.01..=5.0)
                    .speed(0.01)
                    .prefix("tolerance nm "),
            );
            if ui
                .add_enabled(
                    !self.echelle_cal_ui.detected_arc_lines.is_empty(),
                    egui::Button::new("Match to Atlas"),
                )
                .clicked()
            {
                trigger_match = true;
            }
        });

        if trigger_match {
            // We need a seed wavelength function from the current profile order.
            let selected_idx = self.echelle_cal_ui.selected_order_edit_idx;
            let seed_fn_result: Result<Box<dyn Fn(f64) -> f64>, String> = if let Some(profile) =
                self.echelle_cal_ui.editor_profile.as_ref()
            {
                if let Some(order) = profile.orders.get(selected_idx) {
                    match &order.wavelength {
                        EchelleWavelengthModel::Polynomial {
                            basis,
                            coefficients,
                            domain_start,
                            domain_end,
                            ..
                        } => {
                            let basis = *basis;
                            let coeffs = coefficients.clone();
                            let ds = *domain_start;
                            let de = *domain_end;
                            Ok(Box::new(move |px: f64| -> f64 {
                                eval_polynomial_for_ui(basis, &coeffs, ds, de, px).unwrap_or(0.0)
                            }) as Box<dyn Fn(f64) -> f64>)
                        }
                        EchelleWavelengthModel::Sampled { wavelengths, .. } => {
                            let wls = wavelengths.clone();
                            Ok(Box::new(move |px: f64| -> f64 {
                                let idx = px.round().clamp(0.0, f64::MAX) as usize;
                                wls.get(idx).copied().unwrap_or(0.0)
                            }) as Box<dyn Fn(f64) -> f64>)
                        }
                    }
                } else {
                    Err("Selected order is out of range".to_string())
                }
            } else {
                Err("No editor profile loaded".to_string())
            };

            match seed_fn_result {
                Ok(seed_fn) => {
                    let atlas = load_hgar_atlas();
                    let tolerance = self.echelle_cal_ui.atlas_match_tolerance_nm;
                    let lines = &self.echelle_cal_ui.detected_arc_lines;
                    let raw_matches =
                        match_lines_to_atlas(lines, &atlas, seed_fn.as_ref(), tolerance);

                    // Build match table rows.
                    let rows: Vec<ArcLineMatchRow> = raw_matches
                        .iter()
                        .map(|&(li, ai)| {
                            let line = &lines[li];
                            let atlas_line = &atlas[ai];
                            let predicted_wl = seed_fn(line.pixel_center);
                            // Estimate noise from amplitude / SNR approximation.
                            // The detect function filters by sigdetect, so SNR >= config threshold.
                            let snr = line.amplitude
                                / self.echelle_cal_ui.arc_detect_config.sigdetect.max(1.0);
                            ArcLineMatchRow {
                                pixel_center: line.pixel_center,
                                snr,
                                fwhm: line.fwhm(),
                                matched_wavelength_nm: atlas_line.wavelength_nm,
                                residual_nm: predicted_wl - atlas_line.wavelength_nm,
                                species: atlas_line.species.clone(),
                                included: true,
                                detected_line_idx: li,
                                atlas_line_idx: ai,
                            }
                        })
                        .collect();
                    self.echelle_cal_ui.status_message =
                        Some(format!("Matched {} lines to HgAr atlas", rows.len()));
                    self.echelle_cal_ui.matched_pairs = rows;
                    self.echelle_cal_ui.last_error = None;
                    self.echelle_cal_ui.wl_fit_solution = None;
                }
                Err(err) => {
                    self.echelle_cal_ui.last_error = Some(err);
                }
            }
        }

        // ── Section 4: Chebyshev Fit (bd-a64a) ──────────────────────────
        let mut trigger_cheb_fit = false;
        let mut trigger_export_profile = false;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.wl_fit_config.poly_degree)
                    .range(1..=10)
                    .prefix("degree "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.wl_fit_config.sigma_clip)
                    .range(1.0..=10.0)
                    .speed(0.1)
                    .prefix("\u{03c3}-clip "),
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.wl_fit_config.max_clip_iters)
                    .range(0..=20)
                    .prefix("iters "),
            );
            let has_matches = !self.echelle_cal_ui.matched_pairs.is_empty();
            if ui
                .add_enabled(has_matches, egui::Button::new("Fit Chebyshev"))
                .clicked()
            {
                trigger_cheb_fit = true;
            }
            if ui
                .add_enabled(
                    self.echelle_cal_ui.wl_fit_solution.is_some(),
                    egui::Button::new("Export to Profile"),
                )
                .clicked()
            {
                trigger_export_profile = true;
            }
        });

        if trigger_cheb_fit {
            let atlas = load_hgar_atlas();
            let lines = &self.echelle_cal_ui.detected_arc_lines;
            let matches: Vec<(usize, usize)> = self
                .echelle_cal_ui
                .matched_pairs
                .iter()
                .filter(|r| r.included)
                .map(|r| (r.detected_line_idx, r.atlas_line_idx))
                .collect();
            let order_idx = lines.first().map_or(0, |l| l.order);
            let config = self.echelle_cal_ui.wl_fit_config.clone();
            match fit_order_wavelength(lines, &atlas, &matches, order_idx, &config) {
                Some(solution) => {
                    // Update residuals in match table rows using the fitted solution.
                    for row in &mut self.echelle_cal_ui.matched_pairs {
                        let fitted_wl = solution.eval(row.pixel_center);
                        row.residual_nm = fitted_wl - row.matched_wavelength_nm;
                    }
                    self.echelle_cal_ui.status_message = Some(format!(
                        "Chebyshev fit: RMS {:.4} nm, {}/{} lines used",
                        solution.rms_nm, solution.n_lines_used, solution.n_lines_total,
                    ));
                    self.echelle_cal_ui.wl_fit_solution = Some(solution);
                    self.echelle_cal_ui.last_error = None;
                }
                None => {
                    self.echelle_cal_ui.last_error = Some(
                        "Chebyshev fit failed: too few matched lines or singular matrix"
                            .to_string(),
                    );
                }
            }
        }

        // ── Section 5: Export to Profile (bd-a64a) ───────────────────────
        if trigger_export_profile && let Some(solution) = &self.echelle_cal_ui.wl_fit_solution {
            let selected_idx = self.echelle_cal_ui.selected_order_edit_idx;
            if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
                if let Some(order) = profile.orders.get_mut(selected_idx) {
                    order.wavelength = EchelleWavelengthModel::Polynomial {
                        basis: PolynomialBasis::Chebyshev,
                        coefficients: solution.coefficients.clone(),
                        domain_start: solution.pixel_min,
                        domain_end: solution.pixel_max,
                        unit: "nm".to_string(),
                    };
                    self.echelle_cal_ui.status_message = Some(format!(
                        "Exported Chebyshev wavelength model to order {} (RMS {:.4} nm)",
                        order.relative_index, solution.rms_nm,
                    ));
                    self.echelle_cal_ui.last_error = None;
                } else {
                    self.echelle_cal_ui.last_error =
                        Some("Selected order is out of range".to_string());
                }
            }
            self.mark_echelle_editor_dirty();
        }

        // ── Section 6: Match Table (bd-a64a) ────────────────────────────
        if !self.echelle_cal_ui.matched_pairs.is_empty() {
            ui.separator();

            // Compute RMS for the warning threshold (2*rms).
            let included_residuals: Vec<f64> = self
                .echelle_cal_ui
                .matched_pairs
                .iter()
                .filter(|r| r.included)
                .map(|r| r.residual_nm)
                .collect();
            let match_rms = if included_residuals.is_empty() {
                0.0
            } else {
                let sum_sq: f64 = included_residuals.iter().map(|r| r * r).sum();
                (sum_sq / included_residuals.len() as f64).sqrt()
            };
            let warning_threshold = 2.0 * match_rms;

            egui::ScrollArea::vertical()
                .max_height(200.0)
                .id_salt("arc_match_table")
                .show(ui, |ui| {
                    egui::Grid::new("echelle_arc_match_grid")
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Incl");
                            ui.strong("Pixel");
                            ui.strong("SNR");
                            ui.strong("FWHM");
                            ui.strong("Atlas nm");
                            ui.strong("Residual nm");
                            ui.strong("Species");
                            ui.end_row();

                            for row in &mut self.echelle_cal_ui.matched_pairs {
                                ui.checkbox(&mut row.included, "");
                                ui.label(format!("{:.2}", row.pixel_center));
                                ui.label(format!("{:.1}", row.snr));
                                ui.label(format!("{:.2}", row.fwhm));
                                ui.label(format!("{:.3}", row.matched_wavelength_nm));
                                if row.residual_nm.abs() > warning_threshold
                                    && warning_threshold > 0.0
                                {
                                    ui.colored_label(
                                        colors::WARNING,
                                        format!("{:.4}", row.residual_nm),
                                    );
                                } else {
                                    ui.label(format!("{:.4}", row.residual_nm));
                                }
                                ui.label(&row.species);
                                ui.end_row();
                            }
                        });
                });
        }

        // ── Section 7: Fit Diagnostics (bd-a64a) ────────────────────────
        if let Some(solution) = &self.echelle_cal_ui.wl_fit_solution {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.strong("Fit Diagnostics");
                ui.separator();
                ui.small(format!("RMS: {:.4} nm", solution.rms_nm));
                ui.separator();
                ui.small(format!(
                    "Lines: {}/{}",
                    solution.n_lines_used, solution.n_lines_total
                ));
                ui.separator();
                ui.small(format!(
                    "Degree: {}",
                    solution.coefficients.len().saturating_sub(1)
                ));
                ui.separator();
                if solution.rms_nm <= self.echelle_cal_ui.fit_rms_acceptance_px {
                    ui.colored_label(colors::SUCCESS, "PASS");
                } else {
                    ui.colored_label(colors::WARNING, "REVIEW");
                }
            });

            // Residual scatter plot with sigma bands.
            let residual_points: Vec<[f64; 2]> = self
                .echelle_cal_ui
                .matched_pairs
                .iter()
                .filter(|r| r.included)
                .map(|r| [r.pixel_center, r.residual_nm])
                .collect();
            let excluded_points: Vec<[f64; 2]> = self
                .echelle_cal_ui
                .matched_pairs
                .iter()
                .filter(|r| !r.included)
                .map(|r| [r.pixel_center, r.residual_nm])
                .collect();

            let rms = solution.rms_nm;
            let px_min = solution.pixel_min;
            let px_max = solution.pixel_max;

            Plot::new("echelle_cheb_residual_scatter")
                .height(170.0)
                .allow_scroll(false)
                .allow_zoom(true)
                .allow_drag(true)
                .x_axis_label("pixel")
                .y_axis_label("residual (nm)")
                .show(ui, |plot_ui| {
                    // Zero line.
                    plot_ui.line(
                        Line::new("zero", PlotPoints::new(vec![[px_min, 0.0], [px_max, 0.0]]))
                            .color(egui::Color32::GRAY),
                    );

                    // +/- 1 sigma bands.
                    plot_ui.line(
                        Line::new(
                            "+1\u{03c3}",
                            PlotPoints::new(vec![[px_min, rms], [px_max, rms]]),
                        )
                        .color(egui::Color32::from_rgba_premultiplied(100, 200, 100, 120))
                        .style(egui_plot::LineStyle::dashed_dense()),
                    );
                    plot_ui.line(
                        Line::new(
                            "-1\u{03c3}",
                            PlotPoints::new(vec![[px_min, -rms], [px_max, -rms]]),
                        )
                        .color(egui::Color32::from_rgba_premultiplied(100, 200, 100, 120))
                        .style(egui_plot::LineStyle::dashed_dense()),
                    );

                    // +/- 3 sigma bands.
                    plot_ui.line(
                        Line::new(
                            "+3\u{03c3}",
                            PlotPoints::new(vec![[px_min, 3.0 * rms], [px_max, 3.0 * rms]]),
                        )
                        .color(egui::Color32::from_rgba_premultiplied(200, 80, 80, 120))
                        .style(egui_plot::LineStyle::dashed_dense()),
                    );
                    plot_ui.line(
                        Line::new(
                            "-3\u{03c3}",
                            PlotPoints::new(vec![[px_min, -3.0 * rms], [px_max, -3.0 * rms]]),
                        )
                        .color(egui::Color32::from_rgba_premultiplied(200, 80, 80, 120))
                        .style(egui_plot::LineStyle::dashed_dense()),
                    );

                    // Included points.
                    if !residual_points.is_empty() {
                        plot_ui.points(
                            Points::new("included", PlotPoints::new(residual_points))
                                .radius(3.5)
                                .color(egui::Color32::from_rgb(100, 200, 255)),
                        );
                    }
                    // Excluded points.
                    if !excluded_points.is_empty() {
                        plot_ui.points(
                            Points::new("excluded", PlotPoints::new(excluded_points))
                                .radius(3.0)
                                .color(egui::Color32::from_rgb(180, 80, 80))
                                .shape(egui_plot::MarkerShape::Cross),
                        );
                    }
                });

            // Wavelength solution preview overlaid on spectrum.
            if let Some(preview) = &self.echelle_preview {
                let order_plot_idx = self.echelle_selected_order_plot;
                if let Some(order_preview) = preview.orders.get(order_plot_idx)
                    && !order_preview.flux.is_empty()
                {
                    ui.small("Wavelength solution overlay on arc spectrum:");
                    let n_samples = order_preview.flux.len();
                    // Build the fitted wavelength curve.
                    let wl_curve: Vec<[f64; 2]> = (0..n_samples)
                        .map(|i| {
                            let px = i as f64;
                            let wl = solution.eval(px);
                            [px, wl]
                        })
                        .collect();
                    // Build spectrum (normalized to fit on same plot).
                    let flux_max = order_preview
                        .flux
                        .iter()
                        .copied()
                        .fold(0.0_f64, f64::max)
                        .max(1.0);
                    let wl_range = (solution.eval(solution.pixel_max)
                        - solution.eval(solution.pixel_min))
                    .abs()
                    .max(1.0);
                    let flux_as_wl: Vec<[f64; 2]> = order_preview
                        .flux
                        .iter()
                        .enumerate()
                        .map(|(i, &f)| {
                            let px = i as f64;
                            let wl_base = solution.eval(solution.pixel_min);
                            [px, wl_base + (f / flux_max) * wl_range * 0.3]
                        })
                        .collect();

                    // Detected line markers as vertical lines in wavelength space.
                    let line_markers: Vec<[f64; 2]> = self
                        .echelle_cal_ui
                        .detected_arc_lines
                        .iter()
                        .map(|l| [l.pixel_center, solution.eval(l.pixel_center)])
                        .collect();

                    Plot::new("echelle_wl_solution_overlay")
                        .height(150.0)
                        .allow_scroll(false)
                        .allow_zoom(true)
                        .allow_drag(true)
                        .x_axis_label("pixel")
                        .y_axis_label("wavelength (nm)")
                        .show(ui, |plot_ui| {
                            plot_ui.line(
                                Line::new("\u{03bb}(pixel)", PlotPoints::new(wl_curve))
                                    .color(egui::Color32::from_rgb(255, 200, 60))
                                    .width(2.0),
                            );
                            if !flux_as_wl.is_empty() {
                                plot_ui.line(
                                    Line::new("spectrum", PlotPoints::new(flux_as_wl)).color(
                                        egui::Color32::from_rgba_premultiplied(120, 180, 255, 100),
                                    ),
                                );
                            }
                            if !line_markers.is_empty() {
                                plot_ui.points(
                                    Points::new("arc lines", PlotPoints::new(line_markers))
                                        .radius(3.0)
                                        .color(egui::Color32::from_rgb(255, 100, 100)),
                                );
                            }
                        });
                }
            }
        }

        // ── Section 8: Calibration quality report (bd-du24) ──────────────
        let Some(profile) = self.echelle_cal_ui.editor_profile.as_ref() else {
            return;
        };
        let selected_order = profile
            .orders
            .get(self.echelle_cal_ui.selected_order_edit_idx)
            .or_else(|| profile.orders.first());
        let selected_relative_index = selected_order.map(|o| o.relative_index);
        let matched_lines = build_quality_matched_lines(&self.echelle_cal_ui, profile);
        let quality_report = echelle::calibration_quality::compute_quality_report(
            profile,
            &matched_lines,
            36_300.0,
            4,
            3,
        );

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.strong("Calibration Quality");
            ui.separator();
            ui.small(format!(
                "Global RMS: {:.4} nm",
                quality_report.global_rms_nm
            ));
            ui.separator();
            ui.small(match quality_report.loo_rms {
                Some(v) => format!("LOO RMS: {:.4} nm", v),
                None => "LOO RMS: n/a".to_string(),
            });
            ui.separator();
            if let Some(max_overlap) = quality_report
                .overlap_disagreements
                .iter()
                .map(|o| o.max_disagreement_nm)
                .reduce(f64::max)
            {
                ui.small(format!("Max overlap Δλ: {:.4} nm", max_overlap));
            } else {
                ui.small("Max overlap Δλ: n/a");
            }
            ui.separator();
            if let Some(max_gc_frac) = quality_report
                .gc_deviations
                .iter()
                .map(|g| g.fractional_deviation.abs())
                .reduce(f64::max)
            {
                ui.small(format!("Max |mλ-gc|: {:.2}%", max_gc_frac * 100.0));
            } else {
                ui.small("Max |mλ-gc|: n/a");
            }
        });
        ui.small(format!(
            "Matched atlas lines used for quality metrics: {}",
            matched_lines.len()
        ));
        if let Some(rel_idx) = selected_relative_index
            && let Some(order_metrics) = quality_report
                .per_order_rms
                .iter()
                .find(|o| o.relative_index == rel_idx)
        {
            ui.small(format!(
                "Selected order rel={} | matched={} | RMS={:.4} nm{}",
                order_metrics.relative_index,
                order_metrics.n_matched_lines,
                order_metrics.rms_nm,
                order_metrics
                    .wavelength_range_nm
                    .map(|(a, b)| format!(" | range {:.2}-{:.2} nm", a, b))
                    .unwrap_or_default()
            ));
        }
        if let Some(max_overlap) = quality_report
            .overlap_disagreements
            .iter()
            .max_by(|a, b| a.max_disagreement_nm.total_cmp(&b.max_disagreement_nm))
        {
            ui.small(format!(
                "Worst overlap: rel {} vs {} | Δλ {:.4} nm over {:.2}-{:.2} nm",
                max_overlap.order_a,
                max_overlap.order_b,
                max_overlap.max_disagreement_nm,
                max_overlap.overlap_range_nm.0,
                max_overlap.overlap_range_nm.1
            ));
        }
        let n_gc_out_of_band = quality_report
            .gc_deviations
            .iter()
            .filter(|g| g.fractional_deviation.abs() > 0.01)
            .count();
        ui.small(format!(
            "Grating-constant consistency: {} / {} orders outside 1% band",
            n_gc_out_of_band,
            quality_report.gc_deviations.len()
        ));
        if !quality_report.per_order_rms.is_empty() {
            ui.collapsing("Per-order quality", |ui| {
                egui::ScrollArea::vertical()
                    .max_height(170.0)
                    .id_salt("echelle_quality_per_order")
                    .show(ui, |ui| {
                        egui::Grid::new("echelle_quality_per_order_grid")
                            .striped(true)
                            .show(ui, |ui| {
                                ui.strong("rel");
                                ui.strong("m");
                                ui.strong("RMS nm");
                                ui.strong("matched");
                                ui.strong("range nm");
                                ui.strong("blaze peak");
                                ui.end_row();

                                for order_metrics in &quality_report.per_order_rms {
                                    let order = profile
                                        .orders
                                        .iter()
                                        .find(|o| o.relative_index == order_metrics.relative_index);
                                    let blaze_peak = order.and_then(|o| {
                                        blaze_peak_wavelength_nm_for_order(
                                            profile,
                                            o,
                                            order_metrics.relative_index,
                                        )
                                    });
                                    ui.label(order_metrics.relative_index.to_string());
                                    ui.label(
                                        order_metrics
                                            .physical_order
                                            .map(|m| m.to_string())
                                            .unwrap_or_else(|| "-".to_string()),
                                    );
                                    ui.label(format!("{:.4}", order_metrics.rms_nm));
                                    ui.label(order_metrics.n_matched_lines.to_string());
                                    ui.label(
                                        order_metrics
                                            .wavelength_range_nm
                                            .map(|(a, b)| format!("{:.2}-{:.2}", a, b))
                                            .unwrap_or_else(|| "-".to_string()),
                                    );
                                    ui.label(
                                        blaze_peak
                                            .map(|p| format!("{:.2} nm", p))
                                            .unwrap_or_else(|| "-".to_string()),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
            });
        }
        if matched_lines.is_empty() {
            ui.small(
                "Quality metrics that depend on atlas matches (RMS/LOO) are empty. Detect lines and run atlas matching to populate them.",
            );
        }

        // ── Section 9: Legacy manual-points residual display ─────────────

        let mut global_count = 0usize;
        let mut global_sum_sq = 0.0f64;
        for order in &profile.orders {
            let residuals = compute_wavelength_fit_residuals_for_order(
                order,
                &self.echelle_cal_ui.calibration_points,
            );
            global_count += residuals.len();
            global_sum_sq += residuals.iter().map(|(_, r)| r * r).sum::<f64>();
        }
        if global_count > 0 {
            ui.separator();
            let global_rms = (global_sum_sq / global_count as f64).sqrt();
            ui.small(format!(
                "Manual points residual summary: {} points | RMS {:.6}",
                global_count, global_rms
            ));

            let selected_order = profile
                .orders
                .get(self.echelle_cal_ui.selected_order_edit_idx)
                .or_else(|| profile.orders.first());
            if let Some(order) = selected_order {
                let residuals = compute_wavelength_fit_residuals_for_order(
                    order,
                    &self.echelle_cal_ui.calibration_points,
                );
                let count = residuals.len();
                if count > 0 {
                    let rms =
                        (residuals.iter().map(|(_, r)| r * r).sum::<f64>() / count as f64).sqrt();
                    let mean = residuals.iter().map(|(_, r)| *r).sum::<f64>() / count as f64;
                    let stddev = (residuals
                        .iter()
                        .map(|(_, r)| {
                            let d = *r - mean;
                            d * d
                        })
                        .sum::<f64>()
                        / count as f64)
                        .sqrt();
                    let sigma = self.echelle_cal_ui.fit_outlier_sigma.max(0.1);
                    let outliers = residuals
                        .iter()
                        .filter(|(_, r)| stddev > 0.0 && ((*r - mean).abs() / stddev) > sigma)
                        .count();

                    ui.horizontal_wrapped(|ui| {
                        ui.small(format!("Order rel={}", order.relative_index));
                        ui.separator();
                        ui.small(format!("points: {count}"));
                        ui.separator();
                        ui.small(format!("RMS: {:.6}", rms));
                        ui.separator();
                        ui.small(format!("outliers@{sigma:.1}\u{03c3}: {outliers}"));
                        ui.separator();
                        if rms <= self.echelle_cal_ui.fit_rms_acceptance_px {
                            ui.colored_label(colors::SUCCESS, "Within acceptance");
                        } else {
                            ui.colored_label(colors::WARNING, "Exceeds acceptance");
                        }
                    });

                    let points: Vec<[f64; 2]> = residuals.iter().map(|(x, r)| [*x, *r]).collect();
                    Plot::new("echelle_wavelength_fit_residual_plot")
                        .height(140.0)
                        .allow_scroll(false)
                        .allow_zoom(true)
                        .allow_drag(true)
                        .x_axis_label("sample")
                        .y_axis_label("\u{03bb} residual")
                        .show(ui, |plot_ui| {
                            plot_ui.points(
                                Points::new("residuals", PlotPoints::new(points))
                                    .radius(3.0)
                                    .color(egui::Color32::from_rgb(255, 185, 60)),
                            );
                            plot_ui.line(Line::new(
                                "zero",
                                PlotPoints::new(vec![
                                    [f64::from(order.sample_start), 0.0],
                                    [f64::from(order.sample_end), 0.0],
                                ]),
                            ));
                        });
                }
            }
        }

        // ── Arc line markers on spectrum plot (bd-a64a) ──────────────────
        if !self.echelle_cal_ui.detected_arc_lines.is_empty()
            && let Some(preview) = &self.echelle_preview
        {
            let order_plot_idx = self.echelle_selected_order_plot;
            if let Some(order_preview) = preview.orders.get(order_plot_idx)
                && !order_preview.flux.is_empty()
            {
                ui.separator();
                ui.small("Detected arc lines on extracted spectrum:");
                let spectrum_points: Vec<[f64; 2]> = order_preview
                    .flux
                    .iter()
                    .enumerate()
                    .map(|(i, &f)| [i as f64, f])
                    .collect();
                let flux_max = order_preview
                    .flux
                    .iter()
                    .copied()
                    .fold(0.0_f64, f64::max)
                    .max(1.0);

                Plot::new("echelle_arc_detect_overlay_plot")
                    .height(160.0)
                    .allow_scroll(false)
                    .allow_zoom(true)
                    .allow_drag(true)
                    .x_axis_label("pixel")
                    .y_axis_label("counts")
                    .show(ui, |plot_ui| {
                        plot_ui.line(
                            Line::new("spectrum", PlotPoints::new(spectrum_points))
                                .color(egui::Color32::from_rgb(120, 200, 255)),
                        );
                        // Vertical markers at detected line positions.
                        for line in &self.echelle_cal_ui.detected_arc_lines {
                            let px = line.pixel_center;
                            plot_ui.line(
                                Line::new(
                                    "",
                                    PlotPoints::new(vec![[px, 0.0], [px, flux_max * 0.9]]),
                                )
                                .color(egui::Color32::from_rgba_premultiplied(255, 80, 80, 160))
                                .width(1.0),
                            );
                        }
                        // SNR/FWHM annotations as point markers at line peaks.
                        let line_peaks: Vec<[f64; 2]> = self
                            .echelle_cal_ui
                            .detected_arc_lines
                            .iter()
                            .map(|l| [l.pixel_center, l.amplitude])
                            .collect();
                        plot_ui.points(
                            Points::new("line peaks", PlotPoints::new(line_peaks))
                                .radius(4.0)
                                .color(egui::Color32::from_rgb(255, 120, 60)),
                        );
                    });

                // Line detail table.
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .id_salt("arc_line_detail_table")
                    .show(ui, |ui| {
                        egui::Grid::new("echelle_arc_line_detail_grid")
                            .striped(true)
                            .show(ui, |ui| {
                                ui.strong("Pixel");
                                ui.strong("Amplitude");
                                ui.strong("SNR");
                                ui.strong("FWHM");
                                ui.end_row();
                                let noise_est =
                                    self.echelle_cal_ui.arc_detect_config.sigdetect.max(1.0);
                                for line in &self.echelle_cal_ui.detected_arc_lines {
                                    ui.label(format!("{:.2}", line.pixel_center));
                                    ui.label(format!("{:.1}", line.amplitude));
                                    ui.label(format!("{:.1}", line.amplitude / noise_est));
                                    ui.label(format!("{:.2}", line.fwhm()));
                                    ui.end_row();
                                }
                            });
                    });
            }
        }
    }

    pub(in crate::panels::image_viewer) fn render_echelle_calibration_blaze_tab(
        &mut self,
        ui: &mut egui::Ui,
    ) {
        ui.small("Use a continuum flat (e.g. DH-3) on the camera stream, then extract blaze envelopes into the editor profile.");
        ui.add_space(4.0);

        let can_extract_flat = self.echelle_cal_ui.editor_profile.is_some()
            && self.last_frame_data.is_some()
            && self.width > 0
            && self.height > 0;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    can_extract_flat,
                    egui::Button::new("Extract blaze from current frame"),
                )
                .on_hover_text(
                    "Runs simple-sum extraction on the live frame buffer (treated as a flat lamp), peak-normalises each order, and writes corrections.blaze_curves",
                )
                .clicked()
            {
                match self.extract_flat_blaze_from_current_frame() {
                    Ok(()) => {}
                    Err(e) => self.echelle_cal_ui.last_error = Some(e),
                }
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.echelle_cal_ui.blaze_preview_enabled,
                "Preview blaze-corrected overlay",
            );
            ui.add(
                egui::DragValue::new(&mut self.echelle_cal_ui.blaze_preview_scale)
                    .range(0.05..=100.0)
                    .speed(0.05)
                    .prefix("scale "),
            );
            ui.small("Scalar overlay divisor (MVP); prefer empirical blaze_curves above for real correction.");
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.echelle_cal_ui.editor_profile.is_some(),
                    egui::Button::new("Apply preview → editor blaze_curves"),
                )
                .on_hover_text(
                    "Use the selected order's extracted 1D flux as empirical blaze for that order",
                )
                .clicked()
            {
                match self.apply_selected_preview_blaze_to_editor_blaze_curves() {
                    Ok(()) => {}
                    Err(e) => self.echelle_cal_ui.last_error = Some(e),
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Blaze export CSV:");
            ui.text_edit_singleline(&mut self.echelle_cal_ui.blaze_export_path_text);
            if ui.button("Generate From Selected Order Preview").clicked() {
                let path_text = self
                    .echelle_cal_ui
                    .blaze_export_path_text
                    .trim()
                    .to_string();
                if path_text.is_empty() {
                    self.echelle_cal_ui.last_error =
                        Some("Enter a blaze export CSV path".to_string());
                } else {
                    match self.export_selected_order_blaze_preview_artifact(std::path::Path::new(
                        &path_text,
                    )) {
                        Ok(()) => {
                            self.echelle_cal_ui.status_message =
                                Some(format!("Generated blaze preview artifact {}", path_text));
                            self.echelle_cal_ui.last_error = None;
                        }
                        Err(err) => self.echelle_cal_ui.last_error = Some(err),
                    }
                }
            }
        });

        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_ref() {
            ui.small(format!(
                "Blaze artifact: {}",
                profile
                    .corrections
                    .blaze
                    .as_ref()
                    .map(|b| b.path.as_str())
                    .unwrap_or("<not set>")
            ));
            ui.small(format!(
                "Flat-field artifact: {}",
                profile
                    .corrections
                    .flat_field
                    .as_ref()
                    .map(|f| f.path.as_str())
                    .unwrap_or("<not set>")
            ));
        }

        if let Some(buf) = self.last_frame_data.as_ref() {
            ui.small(format!(
                "Frame buffer: {}×{} px, {}-bit, {} bytes",
                self.width,
                self.height,
                self.bit_depth,
                buf.len()
            ));
        } else {
            ui.weak("No frame buffer — connect a camera and stream to extract a flat-lamp blaze.");
        }

        let Some(preview) = &self.echelle_preview else {
            ui.weak(
                "No extracted preview yet — activate a compatible profile so spectra render here.",
            );
            return;
        };
        let Some(order) = preview.orders.get(self.echelle_selected_order_plot) else {
            ui.weak("No selected order preview available.");
            return;
        };
        if order.flux.is_empty() {
            ui.weak("Selected order flux is empty.");
            return;
        }

        #[allow(clippy::cast_precision_loss)]
        let xs: Vec<f64> = if self.echelle_plot_x_axis_mode == EchellePlotXAxisMode::SampleIndex {
            (0..order.flux.len()).map(|i| i as f64).collect()
        } else {
            order.wavelengths.clone()
        };
        let raw_points: Vec<[f64; 2]> = xs
            .iter()
            .copied()
            .zip(order.flux.iter().copied())
            .map(|(x, y)| [x, y])
            .collect();
        let corrected_points: Vec<[f64; 2]> = if self.echelle_cal_ui.blaze_preview_enabled {
            let s = self.echelle_cal_ui.blaze_preview_scale.max(1e-9);
            xs.iter()
                .copied()
                .zip(order.flux.iter().copied())
                .map(|(x, y)| [x, y / s])
                .collect()
        } else {
            Vec::new()
        };

        Plot::new("echelle_blaze_preview_compare_plot")
            .height(170.0)
            .allow_scroll(false)
            .x_axis_label(
                if self.echelle_plot_x_axis_mode == EchellePlotXAxisMode::SampleIndex {
                    "sample"
                } else {
                    order.wavelength_unit.as_str()
                },
            )
            .y_axis_label("counts")
            .show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("Uncorrected", PlotPoints::new(raw_points))
                        .color(egui::Color32::from_rgb(120, 200, 255)),
                );
                if !corrected_points.is_empty() {
                    plot_ui.line(
                        Line::new("Preview corrected", PlotPoints::new(corrected_points))
                            .color(egui::Color32::from_rgb(255, 160, 60)),
                    );
                }
            });
    }

    pub(in crate::panels::image_viewer) fn render_echelle_calibration_mechelle_notes_tab(
        &mut self,
        ui: &mut egui::Ui,
    ) {
        self.ensure_echelle_calibration_editor_profile();
        ui.small(
            "Mechelle-specific UX planning for image slicer / multi-trace-per-order complexity.",
        );
        ui.separator();
        ui.label("Current design assumptions:");
        ui.small("1. A single physical echelle order may present multiple visible traces/slices.");
        ui.small(
            "2. Trace editing UI must support grouping multiple traces under one physical order.",
        );
        ui.small("3. Arc picks should support slice association and confidence labels.");
        ui.small("4. Blaze/flat correction previews should compare per-slice and merged views.");
        ui.small("5. Profile format may need future schema extension for sub-trace components.");
        ui.separator();
        if let Some(profile) = self.echelle_cal_ui.editor_profile.as_mut() {
            ui.label("Operator notes / vendor quirks / slicer observations:");
            let notes = profile.provenance.notes.get_or_insert_with(String::new);
            if ui.text_edit_multiline(notes).changed() {
                self.mark_echelle_editor_dirty();
            }
        }
    }

    pub(in crate::panels::image_viewer) fn build_echelle_trace_overlay_paths(
        &self,
    ) -> Vec<(u32, Vec<(f32, f32)>)> {
        if !self.echelle_cal_ui.trace_overlay_enabled {
            return Vec::new();
        }
        let profile = self
            .echelle_cal_ui
            .editor_profile
            .clone()
            .or_else(|| self.echelle_profile_cache.profile().map(|p| (**p).clone()));
        let Some(profile) = profile else {
            return Vec::new();
        };
        let sample_step = self.echelle_cal_ui.trace_overlay_sample_step.max(1) as usize;
        let max_orders = self.echelle_cal_ui.trace_overlay_max_orders.max(1) as usize;
        let selected_relative = self
            .echelle_cal_ui
            .editor_profile
            .as_ref()
            .and_then(|p| p.orders.get(self.echelle_cal_ui.selected_order_edit_idx))
            .map(|o| o.relative_index);

        let mut out = Vec::new();
        for order in profile.orders.iter().filter(|o| o.enabled) {
            if !self.echelle_cal_ui.trace_overlay_all_orders
                && selected_relative.is_some()
                && Some(order.relative_index) != selected_relative
            {
                continue;
            }
            if out.len() >= max_orders {
                break;
            }
            let mut pts = Vec::new();
            let total_samples = order
                .sample_end
                .saturating_sub(order.sample_start)
                .saturating_add(1) as usize;
            for sample_idx in (0..total_samples).step_by(sample_step) {
                if let Some((x, y)) = order_sample_image_position(&profile, order, sample_idx)
                    && x.is_finite()
                    && y.is_finite()
                {
                    pts.push((x, y));
                }
            }
            if total_samples > 0 {
                let last_idx = total_samples - 1;
                if let Some((x, y)) = order_sample_image_position(&profile, order, last_idx)
                    && x.is_finite()
                    && y.is_finite()
                    && pts
                        .last()
                        .map(|(px, py)| (*px - x).abs() > 1e-3 || (*py - y).abs() > 1e-3)
                        .unwrap_or(true)
                {
                    pts.push((x, y));
                }
            }
            if pts.len() >= 2 {
                out.push((order.relative_index, pts));
            }
        }
        out
    }
}

fn build_quality_matched_lines(
    ui_state: &EchelleCalibrationUiState,
    profile: &EchelleCalibrationProfile,
) -> Vec<echelle::calibration_quality::MatchedLine> {
    ui_state
        .matched_pairs
        .iter()
        .filter(|r| r.included)
        .filter_map(|row| {
            let line = ui_state.detected_arc_lines.get(row.detected_line_idx)?;
            let relative_order = line.order;
            let order = profile
                .orders
                .iter()
                .find(|o| o.relative_index == relative_order)?;
            let physical_order = order.physical_order_number.and_then(|m| {
                let abs = m.unsigned_abs();
                (abs != 0).then_some(abs)
            })?;
            Some(echelle::calibration_quality::MatchedLine {
                pixel: line.pixel_center,
                physical_order,
                relative_order,
                atlas_wavelength_nm: row.matched_wavelength_nm,
            })
        })
        .collect()
}

fn blaze_peak_wavelength_nm_for_order(
    profile: &EchelleCalibrationProfile,
    order: &EchelleOrderCalibration,
    relative_index: u32,
) -> Option<f64> {
    let curves = profile.corrections.blaze_curves.as_ref()?;
    let pos = profile
        .orders
        .iter()
        .position(|o| o.relative_index == relative_index)?;
    let curve = curves.get(pos)?;
    let (peak_idx, _) = curve.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    let peak_idx = u32::try_from(peak_idx).ok()?;
    let sample = order.sample_start.saturating_add(peak_idx);
    wavelength_at_sample(order, sample)
}

fn wavelength_at_sample(order: &EchelleOrderCalibration, sample: u32) -> Option<f64> {
    match &order.wavelength {
        EchelleWavelengthModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
            ..
        } => eval_polynomial_for_ui(
            *basis,
            coefficients,
            *domain_start,
            *domain_end,
            f64::from(sample),
        ),
        EchelleWavelengthModel::Sampled { wavelengths, .. } => {
            let idx = sample.checked_sub(order.sample_start)? as usize;
            wavelengths.get(idx).copied()
        }
    }
}

pub(in crate::panels::image_viewer) fn compute_wavelength_fit_residuals_for_order(
    order: &EchelleOrderCalibration,
    points: &[EchelleCalibrationPointUi],
) -> Vec<(f64, f64)> {
    let mut residuals = Vec::new();
    for point in points
        .iter()
        .filter(|p| p.enabled && p.order_relative_index == order.relative_index)
    {
        let predicted = match &order.wavelength {
            EchelleWavelengthModel::Polynomial {
                basis,
                coefficients,
                domain_start,
                domain_end,
                ..
            } => eval_polynomial_for_ui(
                *basis,
                coefficients,
                *domain_start,
                *domain_end,
                point.x_sample,
            ),
            EchelleWavelengthModel::Sampled { wavelengths, .. } => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let idx = point.x_sample.round().clamp(0.0, f64::MAX) as usize;
                wavelengths.get(idx).copied()
            }
        };
        if let Some(predicted) = predicted {
            residuals.push((point.x_sample, point.wavelength - predicted));
        }
    }
    residuals
}

pub(in crate::panels::image_viewer) fn fit_wavelength_model_for_order_from_points(
    order: &mut EchelleOrderCalibration,
    points: &[EchelleCalibrationPointUi],
) -> Result<String, String> {
    let enabled_points: Vec<&EchelleCalibrationPointUi> = points
        .iter()
        .filter(|p| p.enabled && p.order_relative_index == order.relative_index)
        .collect();
    if enabled_points.len() < 2 {
        return Err(format!(
            "Need at least 2 enabled calibration points for order {}",
            order.relative_index
        ));
    }

    match &mut order.wavelength {
        EchelleWavelengthModel::Polynomial {
            basis,
            coefficients,
            domain_start,
            domain_end,
            unit,
        } => {
            if coefficients.is_empty() {
                return Err(format!(
                    "Order {} has an empty polynomial (no coefficients) — cannot fit",
                    order.relative_index
                ));
            }
            let degree = coefficients.len().saturating_sub(1);
            if enabled_points.len() < coefficients.len() {
                return Err(format!(
                    "Need at least {} enabled points to fit degree {} polynomial for order {}",
                    coefficients.len(),
                    degree,
                    order.relative_index
                ));
            }
            let xs: Vec<f64> = enabled_points.iter().map(|p| p.x_sample).collect();
            let ys: Vec<f64> = enabled_points.iter().map(|p| p.wavelength).collect();
            let fitted = fit_polynomial_basis_least_squares_ui(
                *basis,
                degree,
                *domain_start,
                *domain_end,
                &xs,
                &ys,
            )?;
            *coefficients = fitted;
            Ok(format!(
                "Fitted order {} wavelength polynomial (degree {}, {} points, unit {})",
                order.relative_index,
                degree,
                enabled_points.len(),
                unit
            ))
        }
        EchelleWavelengthModel::Sampled { .. } => Err(format!(
            "Selected order {} uses sampled wavelengths; sampled refit UI not implemented yet",
            order.relative_index
        )),
    }
}

pub(in crate::panels::image_viewer) fn eval_polynomial_for_ui(
    basis: PolynomialBasis,
    coefficients: &[f64],
    domain_start: f64,
    domain_end: f64,
    x: f64,
) -> Option<f64> {
    if coefficients.is_empty()
        || !x.is_finite()
        || !domain_start.is_finite()
        || !domain_end.is_finite()
        || domain_start >= domain_end
    {
        return None;
    }
    let value = match basis {
        PolynomialBasis::Monomial => {
            let mut acc = 0.0f64;
            for &c in coefficients.iter().rev() {
                acc = acc * x + c;
            }
            acc
        }
        PolynomialBasis::Chebyshev => {
            let t = ((2.0 * (x - domain_start)) / (domain_end - domain_start)) - 1.0;
            if coefficients.len() == 1 {
                coefficients[0]
            } else {
                let mut t0 = 1.0f64;
                let mut t1 = t;
                let mut acc = coefficients[0] * t0 + coefficients[1] * t1;
                for &c in coefficients.iter().skip(2) {
                    let tn = 2.0 * t * t1 - t0;
                    acc += c * tn;
                    t0 = t1;
                    t1 = tn;
                }
                acc
            }
        }
    };
    value.is_finite().then_some(value)
}

pub(in crate::panels::image_viewer) fn fit_polynomial_basis_least_squares_ui(
    basis: PolynomialBasis,
    degree: usize,
    domain_start: f64,
    domain_end: f64,
    xs: &[f64],
    ys: &[f64],
) -> Result<Vec<f64>, String> {
    if xs.len() != ys.len() || xs.is_empty() {
        return Err("xs/ys must be non-empty and same length".to_string());
    }
    let n_coeff = degree + 1;
    let mut ata = vec![vec![0.0f64; n_coeff]; n_coeff];
    let mut aty = vec![0.0f64; n_coeff];

    for (&x, &y) in xs.iter().zip(ys) {
        let terms = basis_terms_for_ui(basis, degree, domain_start, domain_end, x)
            .ok_or_else(|| "invalid polynomial basis/domain/input while fitting".to_string())?;
        for i in 0..n_coeff {
            aty[i] += terms[i] * y;
            for j in 0..n_coeff {
                ata[i][j] += terms[i] * terms[j];
            }
        }
    }

    solve_linear_system_gaussian(ata, aty)
        .ok_or_else(|| "least-squares solve failed (singular/ill-conditioned matrix)".to_string())
}

pub(in crate::panels::image_viewer) fn basis_terms_for_ui(
    basis: PolynomialBasis,
    degree: usize,
    domain_start: f64,
    domain_end: f64,
    x: f64,
) -> Option<Vec<f64>> {
    if !x.is_finite()
        || !domain_start.is_finite()
        || !domain_end.is_finite()
        || domain_start >= domain_end
    {
        return None;
    }
    let mut out = vec![0.0; degree + 1];
    match basis {
        PolynomialBasis::Monomial => {
            let mut p = 1.0;
            for term in &mut out {
                *term = p;
                p *= x;
            }
        }
        PolynomialBasis::Chebyshev => {
            let t = ((2.0 * (x - domain_start)) / (domain_end - domain_start)) - 1.0;
            out[0] = 1.0;
            if degree >= 1 {
                out[1] = t;
            }
            for n in 2..=degree {
                out[n] = 2.0 * t * out[n - 1] - out[n - 2];
            }
        }
    }
    Some(out)
}

#[allow(clippy::needless_range_loop)]
pub(in crate::panels::image_viewer) fn solve_linear_system_gaussian(
    mut a: Vec<Vec<f64>>,
    mut b: Vec<f64>,
) -> Option<Vec<f64>> {
    let n = a.len();
    if n == 0 || b.len() != n || a.iter().any(|row| row.len() != n) {
        return None;
    }

    for col in 0..n {
        let mut pivot = col;
        let mut best = a[col][col].abs();
        for row in (col + 1)..n {
            let v = a[row][col].abs();
            if v > best {
                best = v;
                pivot = row;
            }
        }
        if best <= 1e-12 || !best.is_finite() {
            return None;
        }
        if pivot != col {
            a.swap(pivot, col);
            b.swap(pivot, col);
        }

        let pivot_val = a[col][col];
        for j in col..n {
            a[col][j] /= pivot_val;
        }
        b[col] /= pivot_val;

        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            if factor == 0.0 {
                continue;
            }
            for j in col..n {
                a[row][j] -= factor * a[col][j];
            }
            b[row] -= factor * b[col];
        }
    }

    if b.iter().all(|v| v.is_finite()) {
        Some(b)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
#[allow(clippy::cast_possible_wrap)]
pub(in crate::panels::image_viewer) fn detect_cross_dispersion_peaks_from_frame(
    data: &[u8],
    width: u32,
    height: u32,
    bit_depth: u32,
    dispersion_axis: DetectorAxis,
    min_separation_px: u32,
    max_peaks: usize,
    threshold_fraction: f64,
) -> Result<Vec<f64>, String> {
    if width == 0 || height == 0 {
        return Err("Frame dimensions must be non-zero".to_string());
    }
    if max_peaks == 0 {
        return Ok(Vec::new());
    }

    let cross_len = match dispersion_axis {
        DetectorAxis::X => height as usize,
        DetectorAxis::Y => width as usize,
    };
    let disp_len = match dispersion_axis {
        DetectorAxis::X => width as usize,
        DetectorAxis::Y => height as usize,
    };
    let mut profile = vec![0.0f64; cross_len];

    for cross in 0..cross_len {
        #[allow(clippy::cast_precision_loss)]
        let mut sum = 0.0f64;
        for disp in 0..disp_len {
            #[allow(clippy::cast_possible_truncation)]
            let (x, y) = match dispersion_axis {
                DetectorAxis::X => (disp as u32, cross as u32),
                DetectorAxis::Y => (cross as u32, disp as u32),
            };
            let px =
                get_pixel_value_inline(data, x, y, width, height, bit_depth).ok_or_else(|| {
                    "Failed to read pixel while auto-detecting trace seeds".to_string()
                })?;
            sum += f64::from(px);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg = sum / disp_len.max(1) as f64;
        profile[cross] = avg;
    }

    let mut sorted = profile.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let max_value = profile
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        .max(median);
    let threshold = median + (max_value - median) * threshold_fraction.clamp(0.0, 1.0);

    let mut candidates: Vec<(usize, f64)> = Vec::new();
    for i in 1..profile.len().saturating_sub(1) {
        let v = profile[i];
        if v >= threshold && v > profile[i - 1] && v >= profile[i + 1] {
            candidates.push((i, v));
        }
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));

    #[allow(clippy::cast_possible_wrap)]
    let min_sep = min_separation_px as isize;
    #[allow(clippy::cast_precision_loss)]
    let mut selected: Vec<usize> = Vec::new();
    for (idx, _v) in candidates {
        if selected
            .iter()
            .all(|&keep| (keep as isize - idx as isize).abs() >= min_sep)
        {
            selected.push(idx);
            if selected.len() >= max_peaks {
                break;
            }
        }
    }
    selected.sort_unstable();

    #[allow(clippy::cast_precision_loss)]
    let result: Vec<f64> = selected.into_iter().map(|i| i as f64).collect();
    Ok(result)
}
