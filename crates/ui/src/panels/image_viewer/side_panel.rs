//! Side panel — camera settings, ROI stats, histogram, pixel stats, echelle preview.

use super::*;

impl ImageViewerPanel {
    pub(super) fn render_stats_side_panel(
        &mut self,
        ui: &mut egui::Ui,
        has_controls_panel: bool,
        has_roi_panel: bool,
        has_histogram_panel: bool,
        has_echelle_panel: bool,
        has_pixel_stats: bool,
    ) {
        ui.set_max_width(ui.available_width());

        // Fixed header: device name + refresh button (stays visible above scroll)
        if has_controls_panel {
            // Loading indicator
            if self.loading_params_device.is_some() {
                layout::card_frame(ui).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading parameters\u{2026}");
                    });
                });
                ui.add_space(2.0);
            }

            if let Some(device_id_ref) = &self.device_id {
                let device_id = device_id_ref.clone();

                // Refresh button header (fixed, not scrolled)
                ui.horizontal(|ui| {
                    ui.strong(format!("{} {}", icons::action::SETTINGS, device_id));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(icons::action::REFRESH)
                            .on_hover_text("Reload parameters from device")
                            .clicked()
                        {
                            // Clear params to trigger auto-reload in rendering.rs
                            self.camera_params.clear();
                            self.loading_params_device = None;
                        }
                    });
                });
                ui.add_space(2.0);
            }
        }

        // Scrollable area for parameter groups and panels (bd-1ue8)
        egui::ScrollArea::vertical()
            .auto_shrink([true, false])
            .id_salt("side_panel_scroll")
            .show(ui, |ui| {
                if has_controls_panel {
                    if let Some(device_id_ref) = &self.device_id {
                        let device_id = device_id_ref.clone();

                        // Collect favorite indices (bd-4wf7)
                        let fav_indices: Vec<usize> = (0..self.camera_params.len())
                            .filter(|&i| {
                                self.param_favorites
                                    .contains(&self.camera_params[i].descriptor.name)
                            })
                            .collect();

                        // Render favorites section at the top if any exist
                        if !fav_indices.is_empty() {
                            layout::card_frame(ui).show(ui, |ui| {
                                egui::CollapsingHeader::new("\u{2605} Quick Access")
                                .default_open(true)
                                .show(ui, |ui| {
                                    for (j, &i) in fav_indices.iter().enumerate() {
                                        self.render_camera_control(ui, &device_id, i);
                                        if j < fav_indices.len() - 1 {
                                            ui.add_space(4.0);
                                        }
                                    }
                                });
                            });
                            ui.add_space(2.0);
                        }

                        // Group remaining parameters by group_name (bd-4wf7)
                        let mut groups: std::collections::BTreeMap<String, Vec<usize>> =
                            std::collections::BTreeMap::new();
                        for i in 0..self.camera_params.len() {
                            let group = self.camera_params[i]
                                .descriptor
                                .group_name
                                .clone()
                                .unwrap_or_default();
                            groups.entry(group).or_default().push(i);
                        }

                        // Render each group as a collapsible card
                        for (group_name, indices) in &groups {
                            let label = if group_name.is_empty() {
                                format!("{} Camera Settings", icons::action::SETTINGS)
                            } else {
                                format!("{} {}", icons::action::SETTINGS, group_name)
                            };
                            // Open the unnamed (core) group by default
                            let default_open = group_name.is_empty();
                            layout::card_frame(ui).show(ui, |ui| {
                                egui::CollapsingHeader::new(label)
                                    .default_open(default_open)
                                    .show(ui, |ui| {
                                        for (j, &i) in indices.iter().enumerate() {
                                            self.render_camera_control(ui, &device_id, i);
                                            if j < indices.len() - 1 {
                                                ui.add_space(4.0);
                                            }
                                        }
                                    });
                            });
                            ui.add_space(2.0);
                        }
                    }
                    ui.add_space(layout::SECTION_SPACING);
                }

                if has_echelle_panel {
                    layout::card_frame(ui).show(ui, |ui| {
                        egui::CollapsingHeader::new("Echelle Spectrum (MVP Preview)")
                            .default_open(true)
                            .show(ui, |ui| {
                                self.render_echelle_preview_panel(ui);
                            });
                    });
                    ui.add_space(layout::SECTION_SPACING);
                }

                if has_roi_panel {
                    layout::card_frame(ui).show(ui, |ui| {
                        egui::CollapsingHeader::new("ROI Statistics")
                            .default_open(true)
                            .show(ui, |ui| {
                                self.roi_selector.show_statistics_panel(ui);

                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .button("Apply as Hardware ROI")
                                        .on_hover_text(
                                            "Update camera acquisition ROI (requires stream stopped)",
                                        )
                                        .clicked()
                                    {
                                        if self.subscription.is_some() {
                                            if let Some(dev_id) = self.device_id.clone() {
                                                self.param_errors.insert(
                                                    (dev_id, "acquisition.roi".to_string()),
                                                    "Stop streaming before applying hardware ROI"
                                                        .to_string(),
                                                );
                                            }
                                        } else if let Some(roi) = self.roi_selector.roi() {
                                            if let Some(dev_id) = self.device_id.clone() {
                                                use crate::widgets::roi_selector::RoiShape;
                                                let roi_json = match roi {
                                                    RoiShape::Rectangle {
                                                        x,
                                                        y,
                                                        width,
                                                        height,
                                                    } => {
                                                        serde_json::json!({
                                                            "type": "rectangle",
                                                            "x": x,
                                                            "y": y,
                                                            "width": width,
                                                            "height": height
                                                        })
                                                    }
                                                    RoiShape::Polygon { .. } => {
                                                        // For hardware ROI, convert polygon to bounding box
                                                        let (min_x, min_y, max_x, max_y) =
                                                            roi.bounding_box();
                                                        serde_json::json!({
                                                            "type": "rectangle",
                                                            "x": min_x,
                                                            "y": min_y,
                                                            "width": max_x.saturating_sub(min_x),
                                                            "height": max_y.saturating_sub(min_y)
                                                        })
                                                    }
                                                };
                                                self.pending_param_updates.push((
                                                    dev_id,
                                                    "acquisition.roi".to_string(),
                                                    roi_json.to_string(),
                                                ));
                                            }
                                        }
                                    }

                                    if ui
                                        .button("Clear Hardware ROI")
                                        .on_hover_text(
                                            "Reset hardware ROI to full sensor (requires stream stopped)",
                                        )
                                        .clicked()
                                    {
                                        self.queue_clear_hardware_roi();
                                    }
                                });
                            });
                    });
                    ui.add_space(layout::SECTION_SPACING);
                }

                if has_pixel_stats {
                    layout::card_frame(ui).show(ui, |ui| {
                        egui::CollapsingHeader::new("Pixel Statistics")
                            .default_open(true)
                            .show(ui, |ui| {
                                if let Some(stats) = &self.pixel_statistics {
                                    egui::Grid::new("pixel_stats_grid")
                                        .num_columns(2)
                                        .spacing([8.0, 2.0])
                                        .show(ui, |ui| {
                                            ui.label("Count:");
                                            ui.label(format!("{}", stats.count));
                                            ui.end_row();

                                            ui.label("Min:");
                                            ui.label(format!("{:.1}", stats.min));
                                            ui.end_row();

                                            ui.label("Max:");
                                            ui.label(format!("{:.1}", stats.max));
                                            ui.end_row();

                                            ui.label("Mean:");
                                            ui.label(format!("{:.2}", stats.mean));
                                            ui.end_row();

                                            ui.label("Std Dev:");
                                            ui.label(format!("{:.2}", stats.std_dev));
                                            ui.end_row();

                                            ui.label("Median:");
                                            ui.label(format!("{:.1}", stats.median));
                                            ui.end_row();

                                            ui.label("Sum:");
                                            ui.label(format!("{:.0}", stats.sum));
                                            ui.end_row();
                                        });

                                    ui.separator();
                                    ui.label("Percentiles");
                                    egui::Grid::new("pixel_stats_percentiles_grid")
                                        .num_columns(2)
                                        .spacing([8.0, 2.0])
                                        .show(ui, |ui| {
                                            ui.label("P1:");
                                            ui.label(format!("{:.1}", stats.p1));
                                            ui.end_row();

                                            ui.label("P5:");
                                            ui.label(format!("{:.1}", stats.p5));
                                            ui.end_row();

                                            ui.label("P25 (Q1):");
                                            ui.label(format!("{:.1}", stats.p25));
                                            ui.end_row();

                                            ui.label("P50:");
                                            ui.label(format!("{:.1}", stats.p50));
                                            ui.end_row();

                                            ui.label("P75 (Q3):");
                                            ui.label(format!("{:.1}", stats.p75));
                                            ui.end_row();

                                            ui.label("P95:");
                                            ui.label(format!("{:.1}", stats.p95));
                                            ui.end_row();

                                            ui.label("P99:");
                                            ui.label(format!("{:.1}", stats.p99));
                                            ui.end_row();
                                        });

                                    ui.add_space(4.0);
                                    if ui
                                        .button("Copy to Clipboard")
                                        .on_hover_text(
                                            "Copy pixel statistics as formatted text",
                                        )
                                        .clicked()
                                    {
                                        ui.ctx().copy_text(stats.to_clipboard_text());
                                    }
                                } else {
                                    ui.label("No frame data available");
                                }
                            });
                    });
                    ui.add_space(layout::SECTION_SPACING);
                }

                if has_histogram_panel {
                    layout::card_frame(ui).show(ui, |ui| {
                        egui::CollapsingHeader::new("Histogram")
                            .default_open(true)
                            .show(ui, |ui| {
                                self.histogram.show_panel(ui);
                            });

                        // Physical coordinate calibration UI (bd-4088.6)
                        egui::CollapsingHeader::new("Calibration")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label("Pixel to Physical Unit Conversion");
                                ui.separator();

                                ui.horizontal(|ui| {
                                    ui.label("X Scale:");
                                    let mut scale_x_str = self
                                        .pixel_scale_x
                                        .map(|v| format!("{:.4}", v))
                                        .unwrap_or_default();
                                    if ui.text_edit_singleline(&mut scale_x_str).changed() {
                                        self.pixel_scale_x = scale_x_str.parse().ok();
                                    }
                                    ui.label("units/pixel");
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Y Scale:");
                                    let mut scale_y_str = self
                                        .pixel_scale_y
                                        .map(|v| format!("{:.4}", v))
                                        .unwrap_or_default();
                                    if ui.text_edit_singleline(&mut scale_y_str).changed() {
                                        self.pixel_scale_y = scale_y_str.parse().ok();
                                    }
                                    ui.label("units/pixel");
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Unit:");
                                    egui::ComboBox::from_id_salt("scale_unit")
                                        .selected_text(&self.scale_unit)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.scale_unit,
                                                "\u{b5}m".to_string(),
                                                "\u{b5}m",
                                            );
                                            ui.selectable_value(
                                                &mut self.scale_unit,
                                                "mm".to_string(),
                                                "mm",
                                            );
                                            ui.selectable_value(
                                                &mut self.scale_unit,
                                                "nm".to_string(),
                                                "nm",
                                            );
                                        });
                                });

                                if ui.button("Clear Calibration").clicked() {
                                    self.pixel_scale_x = None;
                                    self.pixel_scale_y = None;
                                }
                            });
                    });
                }
            });
    }
}
