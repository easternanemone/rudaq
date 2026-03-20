//! Echelle spectrum preview — 1D spectral plot rendering and display helpers.

use super::*;

impl ImageViewerPanel {
    /// Render the spectrum plot area — used in Spectrum and Split view modes (bd-alxb).
    ///
    /// When `full_width` is true, a compact control strip is rendered above the plot.
    /// When false, the plot renders without controls (sidebar already has them).
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn render_spectrum_plot_area(&mut self, ui: &mut egui::Ui, full_width: bool) {
        let Some(preview) = &self.echelle_preview else {
            ui.centered_and_justified(|ui| {
                if let Some(err) = &self.echelle_preview_error {
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), err);
                } else {
                    ui.weak("Waiting for extracted spectrum preview...");
                }
            });
            return;
        };

        if preview.orders.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.weak("No order spectra available.");
            });
            return;
        }

        // Compact control strip for full-width mode
        if full_width {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                // Order selector
                let selected_text = preview
                    .orders
                    .get(self.echelle_selected_order_plot)
                    .map(|o| match o.physical_order_number {
                        Some(m) => format!("Order {} (m={})", o.relative_index, m),
                        None => format!("Order {}", o.relative_index),
                    })
                    .unwrap_or_else(|| "Order 0".to_string());

                ui.label("Order:");
                egui::ComboBox::from_id_salt("spectrum_view_order_selector")
                    .selected_text(selected_text)
                    .width(120.0)
                    .show_ui(ui, |ui| {
                        for (idx, order) in preview.orders.iter().enumerate() {
                            let label = match order.physical_order_number {
                                Some(m) => format!("Order {} (m={})", order.relative_index, m),
                                None => format!("Order {}", order.relative_index),
                            };
                            ui.selectable_value(&mut self.echelle_selected_order_plot, idx, label);
                        }
                    });

                ui.separator();
                ui.checkbox(&mut self.echelle_show_merged_plot, "Merged");

                ui.separator();
                ui.label("X:");
                ui.selectable_value(
                    &mut self.echelle_plot_x_axis_mode,
                    EchellePlotXAxisMode::Wavelength,
                    "\u{03bb}",
                );
                ui.selectable_value(
                    &mut self.echelle_plot_x_axis_mode,
                    EchellePlotXAxisMode::SampleIndex,
                    "px",
                );

                ui.separator();
                ui.add(
                    egui::DragValue::new(&mut self.echelle_plot_smoothing_window)
                        .range(1..=25)
                        .prefix("Smooth ")
                        .suffix(" px"),
                );

                // Frame/profile info on the right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!(
                        "Frame {} | {} orders",
                        preview.frame_number,
                        preview.orders.len()
                    ));
                });
            });
        }

        // Resolve the data to plot (same logic as render_echelle_preview_panel)
        let preview = self.echelle_preview.as_ref().unwrap();
        let mut plot_label = "Order";
        let (mut xs, ys, unit) = if self.echelle_show_merged_plot {
            if let Some(merged) = &preview.merged {
                plot_label = "Merged";
                (
                    merged.wavelengths.clone(),
                    &merged.flux,
                    merged.wavelength_unit.as_str(),
                )
            } else if let Some(order) = preview.orders.get(self.echelle_selected_order_plot) {
                (
                    order.wavelengths.clone(),
                    &order.flux,
                    order.wavelength_unit.as_str(),
                )
            } else {
                return;
            }
        } else if let Some(order) = preview.orders.get(self.echelle_selected_order_plot) {
            (
                order.wavelengths.clone(),
                &order.flux,
                order.wavelength_unit.as_str(),
            )
        } else {
            return;
        };

        if xs.is_empty() || ys.is_empty() {
            ui.weak("Extracted spectrum is empty.");
            return;
        }

        if self.echelle_plot_x_axis_mode == EchellePlotXAxisMode::SampleIndex {
            #[allow(clippy::cast_precision_loss)]
            {
                xs = (0..ys.len()).map(|i| i as f64).collect();
            }
        }
        let display_flux = if self.echelle_plot_smoothing_window > 1 {
            smooth_display_series(ys, self.echelle_plot_smoothing_window as usize)
        } else {
            ys.clone()
        };

        let points: Vec<[f64; 2]> = xs
            .iter()
            .copied()
            .zip(display_flux.iter().copied())
            .map(|(x, y)| [x, y])
            .collect();
        let line = Line::new(format!("{plot_label} flux"), PlotPoints::new(points));
        let selected_order_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview.orders.get(self.echelle_selected_order_plot)
        };
        let wavelength_lookup_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview
                .orders
                .get(self.echelle_selected_order_plot)
                .map(|o| o.wavelengths.as_slice())
        };
        let flux_lookup_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview
                .orders
                .get(self.echelle_selected_order_plot)
                .map(|o| o.flux.as_slice())
        };
        let mut hover_link = None;

        // Use a unique plot ID for full-width vs sidebar to avoid egui ID conflicts
        let plot_id = if full_width {
            "spectrum_view_plot_fullwidth"
        } else {
            "spectrum_view_plot_sidebar"
        };

        let mut plot = Plot::new(plot_id)
            .auto_bounds(egui::Vec2b::FALSE)
            .allow_scroll(false)
            .allow_drag(true)
            .allow_zoom(true)
            .x_axis_label(match self.echelle_plot_x_axis_mode {
                EchellePlotXAxisMode::Wavelength => unit,
                EchellePlotXAxisMode::SampleIndex => "sample",
            })
            .y_axis_label("counts");

        if !full_width {
            plot = plot.height(180.0);
        }

        plot.show(ui, |plot_ui| {
            plot_ui.line(line);
            if let (Some(order), Some(w_lookup), Some(f_lookup)) = (
                selected_order_for_hover,
                wavelength_lookup_for_hover,
                flux_lookup_for_hover,
            ) {
                if let Some(pointer) = plot_ui.pointer_coordinate() {
                    let idx = match self.echelle_plot_x_axis_mode {
                        EchellePlotXAxisMode::SampleIndex => {
                            #[allow(
                                clippy::cast_possible_truncation,
                                clippy::cast_possible_wrap,
                                clippy::cast_sign_loss
                            )]
                            let idx = pointer.x.round() as isize;
                            #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                            let clamped =
                                idx.clamp(0, (f_lookup.len().saturating_sub(1)) as isize) as usize;
                            clamped
                        }
                        EchellePlotXAxisMode::Wavelength => nearest_index_by_x(&xs, pointer.x),
                    };
                    if idx < w_lookup.len() && idx < f_lookup.len() {
                        hover_link = Some(EchellePlotHoverLink {
                            relative_index: order.relative_index,
                            sample_index: idx,
                            wavelength: w_lookup[idx],
                            flux: f_lookup[idx],
                        });
                    }
                }
            }
        });
        self.echelle_plot_hover_link = hover_link;

        // Coverage/hover info below the plot
        if let Some(order) = self
            .echelle_preview
            .as_ref()
            .and_then(|p| p.orders.get(self.echelle_selected_order_plot))
        {
            let mean_valid = if order.valid_fraction.is_empty() {
                0.0
            } else {
                order
                    .valid_fraction
                    .iter()
                    .map(|&v| f64::from(v))
                    .sum::<f64>()
                    / order.valid_fraction.len() as f64
            };
            ui.small(format!(
                "Coverage: {}/{} samples | mean valid frac: {:.2} | saturated: {}",
                order.covered_samples, order.total_samples, mean_valid, order.saturated_samples
            ));
            if let Some(link) = self.echelle_plot_hover_link {
                if link.relative_index == order.relative_index {
                    ui.small(format!(
                        "Hover: sample {} | \u{03bb}={:.6} {} | flux={:.2}",
                        link.sample_index, link.wavelength, order.wavelength_unit, link.flux
                    ));
                }
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub(super) fn render_echelle_preview_panel(&mut self, ui: &mut egui::Ui) {
        self.render_echelle_calibration_workspace(ui);
        ui.separator();

        if self.echelle_profile_cache.profile().is_none() {
            ui.weak("No active echelle calibration profile loaded for live extraction preview.");
            ui.weak("Use the calibration workspace above to load/save+activate a profile path.");
            if self.echelle_preview.is_none() {
                return;
            }
        }

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.echelle_extraction_enabled, "Enabled");
            ui.add(
                egui::DragValue::new(&mut self.echelle_extract_every_n_frames)
                    .range(1..=120)
                    .prefix("Every ")
                    .suffix(" frames"),
            );
            ui.checkbox(&mut self.echelle_show_merged_plot, "Merged");
            ui.separator();
            ui.label("X:");
            ui.selectable_value(
                &mut self.echelle_plot_x_axis_mode,
                EchellePlotXAxisMode::Wavelength,
                "Wavelength",
            );
            ui.selectable_value(
                &mut self.echelle_plot_x_axis_mode,
                EchellePlotXAxisMode::SampleIndex,
                "Sample",
            );
            ui.separator();
            ui.add(
                egui::DragValue::new(&mut self.echelle_plot_smoothing_window)
                    .range(1..=25)
                    .prefix("Smooth ")
                    .suffix(" px"),
            );
        });

        if let Some(profile) = self.echelle_profile_cache.profile() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.small("Calibration Profile");
                ui.small(format!(
                    "{}{}",
                    profile.display_name,
                    profile
                        .profile_id
                        .as_ref()
                        .map(|id| format!(" ({id})"))
                        .unwrap_or_default()
                ));
                ui.small(format!(
                    "Schema {}.{}.{} | Frame {}x{} | ROI ({}, {}) | Bin {}x{}",
                    profile.schema_version.major,
                    profile.schema_version.minor,
                    profile.schema_version.patch,
                    profile.compatibility.frame_width,
                    profile.compatibility.frame_height,
                    profile.compatibility.roi_x,
                    profile.compatibility.roi_y,
                    profile.compatibility.binning_x,
                    profile.compatibility.binning_y
                ));
                ui.small(format!(
                    "Created by {}{} at {}",
                    profile.provenance.creator_tool,
                    profile
                        .provenance
                        .creator_version
                        .as_ref()
                        .map(|v| format!("@{v}"))
                        .unwrap_or_default(),
                    profile.provenance.created_at_utc
                ));
                if let Some(path) = self.echelle_profile_cache.path() {
                    ui.small(format!("Path: {}", path.display()));
                }
            });
        }

        if let Some(err) = &self.echelle_preview_error {
            ui.colored_label(colors::ERROR, err);
        }

        let Some(preview) = &self.echelle_preview else {
            ui.weak("Waiting for extracted spectrum preview...");
            return;
        };

        ui.small(format!(
            "Profile: {} | Frame {} | Orders: {}",
            preview.profile_display_name,
            preview.frame_number,
            preview.orders.len()
        ));

        if !preview.orders.is_empty() {
            let selected_text = preview
                .orders
                .get(self.echelle_selected_order_plot)
                .map(|o| match o.physical_order_number {
                    Some(m) => format!("Order {} (m={})", o.relative_index, m),
                    None => format!("Order {}", o.relative_index),
                })
                .unwrap_or_else(|| "Order 0".to_string());

            egui::ComboBox::from_id_salt("echelle_order_plot_selector")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (idx, order) in preview.orders.iter().enumerate() {
                        let label = match order.physical_order_number {
                            Some(m) => format!("Order {} (m={})", order.relative_index, m),
                            None => format!("Order {}", order.relative_index),
                        };
                        ui.selectable_value(&mut self.echelle_selected_order_plot, idx, label);
                    }
                });
        }

        let mut plot_label = "Order";
        let (mut xs, ys, unit) = if self.echelle_show_merged_plot {
            if let Some(merged) = &preview.merged {
                plot_label = "Merged";
                (
                    merged.wavelengths.clone(),
                    &merged.flux,
                    merged.wavelength_unit.as_str(),
                )
            } else if let Some(order) = preview.orders.get(self.echelle_selected_order_plot) {
                (
                    order.wavelengths.clone(),
                    &order.flux,
                    order.wavelength_unit.as_str(),
                )
            } else {
                ui.weak("No order spectra available.");
                return;
            }
        } else if let Some(order) = preview.orders.get(self.echelle_selected_order_plot) {
            (
                order.wavelengths.clone(),
                &order.flux,
                order.wavelength_unit.as_str(),
            )
        } else {
            ui.weak("No order spectra available.");
            return;
        };

        if xs.is_empty() || ys.is_empty() {
            ui.weak("Extracted spectrum is empty.");
            return;
        }

        if self.echelle_plot_x_axis_mode == EchellePlotXAxisMode::SampleIndex {
            xs = (0..ys.len()).map(|i| i as f64).collect();
        }
        let display_flux = if self.echelle_plot_smoothing_window > 1 {
            smooth_display_series(ys, self.echelle_plot_smoothing_window as usize)
        } else {
            ys.clone()
        };

        let points: Vec<[f64; 2]> = xs
            .iter()
            .copied()
            .zip(display_flux.iter().copied())
            .map(|(x, y)| [x, y])
            .collect();
        let line = Line::new(format!("{plot_label} flux"), PlotPoints::new(points));
        let selected_order_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview.orders.get(self.echelle_selected_order_plot)
        };
        let wavelength_lookup_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview
                .orders
                .get(self.echelle_selected_order_plot)
                .map(|o| o.wavelengths.as_slice())
        };
        let flux_lookup_for_hover = if self.echelle_show_merged_plot {
            None
        } else {
            preview
                .orders
                .get(self.echelle_selected_order_plot)
                .map(|o| o.flux.as_slice())
        };
        let mut hover_link = None;

        Plot::new("image_viewer_echelle_preview_plot")
            .height(180.0)
            .auto_bounds(egui::Vec2b::FALSE)
            .allow_scroll(false)
            .allow_drag(true)
            .allow_zoom(true)
            .x_axis_label(match self.echelle_plot_x_axis_mode {
                EchellePlotXAxisMode::Wavelength => unit,
                EchellePlotXAxisMode::SampleIndex => "sample",
            })
            .y_axis_label("counts")
            .show(ui, |plot_ui| {
                plot_ui.line(line);
                if let (Some(order), Some(w_lookup), Some(f_lookup)) = (
                    selected_order_for_hover,
                    wavelength_lookup_for_hover,
                    flux_lookup_for_hover,
                ) {
                    if let Some(pointer) = plot_ui.pointer_coordinate() {
                        let idx = match self.echelle_plot_x_axis_mode {
                            EchellePlotXAxisMode::SampleIndex => {
                                #[allow(
                                    clippy::cast_possible_truncation,
                                    clippy::cast_possible_wrap,
                                    clippy::cast_sign_loss
                                )]
                                let idx = pointer.x.round() as isize;
                                #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
                                let clamped = idx
                                    .clamp(0, (f_lookup.len().saturating_sub(1)) as isize)
                                    as usize;
                                clamped
                            }
                            EchellePlotXAxisMode::Wavelength => nearest_index_by_x(&xs, pointer.x),
                        };
                        if idx < w_lookup.len() && idx < f_lookup.len() {
                            hover_link = Some(EchellePlotHoverLink {
                                relative_index: order.relative_index,
                                sample_index: idx,
                                wavelength: w_lookup[idx],
                                flux: f_lookup[idx],
                            });
                        }
                    }
                }
            });
        self.echelle_plot_hover_link = hover_link;

        if let Some(order) = preview.orders.get(self.echelle_selected_order_plot) {
            #[allow(clippy::cast_precision_loss)]
            let mean_valid = if order.valid_fraction.is_empty() {
                0.0
            } else {
                order
                    .valid_fraction
                    .iter()
                    .map(|&v| f64::from(v))
                    .sum::<f64>()
                    / order.valid_fraction.len() as f64
            };
            ui.small(format!(
                "Coverage: {}/{} samples | mean valid frac: {:.2} | saturated samples: {}",
                order.covered_samples, order.total_samples, mean_valid, order.saturated_samples
            ));
            if let Some(link) = self.echelle_plot_hover_link {
                if link.relative_index == order.relative_index {
                    ui.small(format!(
                        "Hover: sample {} | λ={:.6} {} | flux={:.2}",
                        link.sample_index, link.wavelength, order.wavelength_unit, link.flux
                    ));
                }
            }
        }

        egui::CollapsingHeader::new("Diagnostics")
            .default_open(false)
            .show(ui, |ui| {
                ui.small(format!("extract runs: {}", self.echelle_extract_runs));
                ui.small(format!("extract errors: {}", self.echelle_extract_errors));
                ui.small(format!(
                    "decimation skips: {}",
                    self.echelle_extract_skipped_frames
                ));
                if let Some(ms) = self.echelle_last_extract_ms {
                    ui.small(format!("last extract: {:.2} ms", ms));
                }
                ui.small(format!(
                    "preview spectra objects: {}",
                    self.echelle_preview_measurements.len()
                ));
            });
    }
}

pub(super) fn smooth_display_series(values: &[f64], window: usize) -> Vec<f64> {
    if values.is_empty() || window <= 1 {
        return values.to_vec();
    }
    let radius = window / 2;
    let mut out = Vec::with_capacity(values.len());
    for i in 0..values.len() {
        let start = i.saturating_sub(radius);
        let end = (i + radius + 1).min(values.len());
        let sum: f64 = values[start..end].iter().sum();
        #[allow(clippy::cast_precision_loss)]
        let avg = sum / (end - start) as f64;
        out.push(avg);
    }
    out
}

pub(super) fn nearest_index_by_x(xs: &[f64], target: f64) -> usize {
    let mut best_idx = 0usize;
    let mut best_dist = f64::INFINITY;
    for (i, &x) in xs.iter().enumerate() {
        let d = (x - target).abs();
        if d < best_dist {
            best_dist = d;
            best_idx = i;
        }
    }
    best_idx
}
