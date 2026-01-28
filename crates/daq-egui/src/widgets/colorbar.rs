//! Interactive colorbar widget for colormap visualization and adjustment
//!
//! Provides:
//! - Vertical or horizontal colorbar showing current colormap gradient
//! - Draggable midpoint handle for gamma-like intensity adjustment
//! - Min/max value labels with units
//! - Double-click to reset to linear mapping (midpoint = 0.5)
//! - Percentage indicator during drag
//! - Optional logarithmic scale toggle

use eframe::egui::{self, StrokeKind};

/// Orientation of the colorbar
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorbarOrientation {
    Vertical,
    Horizontal,
}

/// Interactive colorbar with draggable midpoint control
///
/// The midpoint (0.0-1.0) acts as a gamma-like control:
/// - 0.5 = linear mapping (default)
/// - < 0.5 = darkens midtones (emphasizes bright features)
/// - > 0.5 = brightens midtones (reveals dark features)
#[derive(Debug, Clone)]
pub struct Colorbar {
    /// Midpoint position (0.0-1.0) for non-linear intensity mapping
    pub midpoint: f32,
    /// Orientation (vertical or horizontal)
    pub orientation: ColorbarOrientation,
    /// Minimum data value (for label display)
    pub min_value: f64,
    /// Maximum data value (for label display)
    pub max_value: f64,
    /// Unit string (e.g., "counts", "%", "AU")
    pub units: String,
    /// Use logarithmic scale for value axis
    pub log_scale: bool,
    /// Currently dragging the midpoint handle
    dragging: bool,
}

impl Default for Colorbar {
    fn default() -> Self {
        Self {
            midpoint: 0.5, // Linear mapping by default
            orientation: ColorbarOrientation::Vertical,
            min_value: 0.0,
            max_value: 1.0,
            units: String::new(),
            log_scale: false,
            dragging: false,
        }
    }
}

impl Colorbar {
    /// Create a new colorbar with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Set orientation
    pub fn orientation(mut self, orientation: ColorbarOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Set data value range
    pub fn value_range(mut self, min: f64, max: f64) -> Self {
        self.min_value = min;
        self.max_value = max;
        self
    }

    /// Set unit string
    pub fn units(mut self, units: impl Into<String>) -> Self {
        self.units = units.into();
        self
    }

    /// Reset midpoint to linear (0.5)
    pub fn reset_midpoint(&mut self) {
        self.midpoint = 0.5;
    }

    /// Apply the midpoint adjustment to a normalized value (0.0-1.0)
    ///
    /// Uses a power function to remap values based on midpoint:
    /// - midpoint = 0.5 → linear (gamma = 1.0)
    /// - midpoint < 0.5 → gamma > 1.0 (darkens midtones)
    /// - midpoint > 0.5 → gamma < 1.0 (brightens midtones)
    #[inline]
    pub fn apply_adjustment(&self, value: f32) -> f32 {
        if self.midpoint == 0.5 {
            // Fast path for linear mapping
            value
        } else {
            // Convert midpoint to gamma:
            // gamma = -log(0.5) / log(midpoint)
            // This ensures: adjusted(midpoint) = 0.5
            let gamma = if self.midpoint > 0.0 && self.midpoint < 1.0 {
                -0.693147 / self.midpoint.ln() // -ln(0.5) = 0.693147
            } else {
                1.0
            };
            value.powf(gamma).clamp(0.0, 1.0)
        }
    }

    /// Show the colorbar widget
    ///
    /// Returns true if the midpoint changed (requires texture regeneration)
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        colormap: &impl ColormapTrait,
        size: egui::Vec2,
    ) -> bool {
        let mut changed = false;

        match self.orientation {
            ColorbarOrientation::Vertical => {
                changed = self.show_vertical(ui, colormap, size);
            }
            ColorbarOrientation::Horizontal => {
                changed = self.show_horizontal(ui, colormap, size);
            }
        }

        changed
    }

    /// Show vertical colorbar
    fn show_vertical(
        &mut self,
        ui: &mut egui::Ui,
        colormap: &impl ColormapTrait,
        size: egui::Vec2,
    ) -> bool {
        let mut changed = false;

        ui.vertical(|ui| {
            // Max value label
            ui.label(egui::RichText::new(self.format_value(self.max_value)).small());

            // Colorbar gradient
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(size.x, size.y), egui::Sense::click_and_drag());

            // Draw gradient
            if ui.is_rect_visible(rect) {
                self.draw_gradient_vertical(ui, colormap, rect);

                // Draw midpoint handle
                let handle_y = rect.top() + (1.0 - self.midpoint) * rect.height();
                self.draw_handle_vertical(ui, rect, handle_y);
            }

            // Handle interaction
            if response.clicked() || response.double_clicked() {
                if response.double_clicked() {
                    // Reset to linear
                    self.reset_midpoint();
                    changed = true;
                } else if let Some(pos) = response.interact_pointer_pos() {
                    // Jump to clicked position
                    let normalized = (pos.y - rect.top()) / rect.height();
                    self.midpoint = (1.0 - normalized).clamp(0.0, 1.0);
                    changed = true;
                }
            }

            if response.dragged() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let normalized = (pos.y - rect.top()) / rect.height();
                    self.midpoint = (1.0 - normalized).clamp(0.0, 1.0);
                    self.dragging = true;
                    changed = true;
                }
            }

            if response.drag_stopped() {
                self.dragging = false;
            }

            // Show percentage while dragging
            if self.dragging {
                ui.label(
                    egui::RichText::new(format!("{:.0}%", self.midpoint * 100.0))
                        .small()
                        .color(egui::Color32::YELLOW),
                );
            } else {
                ui.label(egui::RichText::new(format!("{:.0}%", self.midpoint * 100.0)).small());
            }

            // Min value label
            ui.label(egui::RichText::new(self.format_value(self.min_value)).small());
        });

        changed
    }

    /// Show horizontal colorbar
    fn show_horizontal(
        &mut self,
        ui: &mut egui::Ui,
        colormap: &impl ColormapTrait,
        size: egui::Vec2,
    ) -> bool {
        let mut changed = false;

        ui.horizontal(|ui| {
            // Min value label
            ui.label(egui::RichText::new(self.format_value(self.min_value)).small());

            // Colorbar gradient
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(size.x, size.y), egui::Sense::click_and_drag());

            // Draw gradient
            if ui.is_rect_visible(rect) {
                self.draw_gradient_horizontal(ui, colormap, rect);

                // Draw midpoint handle
                let handle_x = rect.left() + self.midpoint * rect.width();
                self.draw_handle_horizontal(ui, rect, handle_x);
            }

            // Handle interaction
            if response.clicked() || response.double_clicked() {
                if response.double_clicked() {
                    // Reset to linear
                    self.reset_midpoint();
                    changed = true;
                } else if let Some(pos) = response.interact_pointer_pos() {
                    // Jump to clicked position
                    let normalized = (pos.x - rect.left()) / rect.width();
                    self.midpoint = normalized.clamp(0.0, 1.0);
                    changed = true;
                }
            }

            if response.dragged() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let normalized = (pos.x - rect.left()) / rect.width();
                    self.midpoint = normalized.clamp(0.0, 1.0);
                    self.dragging = true;
                    changed = true;
                }
            }

            if response.drag_stopped() {
                self.dragging = false;
            }

            // Show percentage while dragging
            if self.dragging {
                ui.label(
                    egui::RichText::new(format!("{:.0}%", self.midpoint * 100.0))
                        .small()
                        .color(egui::Color32::YELLOW),
                );
            } else {
                ui.label(egui::RichText::new(format!("{:.0}%", self.midpoint * 100.0)).small());
            }

            // Max value label
            ui.label(egui::RichText::new(self.format_value(self.max_value)).small());
        });

        changed
    }

    /// Draw vertical gradient
    fn draw_gradient_vertical(
        &self,
        ui: &egui::Ui,
        colormap: &impl ColormapTrait,
        rect: egui::Rect,
    ) {
        let painter = ui.painter();
        let height = rect.height();
        let samples = 64; // Number of gradient steps

        for i in 0..samples {
            let y_start = rect.top() + (i as f32 / samples as f32) * height;
            let y_end = rect.top() + ((i + 1) as f32 / samples as f32) * height;

            // Value goes from 1.0 (top) to 0.0 (bottom)
            let value = 1.0 - (i as f32 / samples as f32);

            let color = colormap.apply(value);
            let egui_color = egui::Color32::from_rgb(color[0], color[1], color[2]);

            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), y_start),
                    egui::pos2(rect.right(), y_end),
                ),
                0.0,
                egui_color,
            );
        }

        // Border
        painter.rect_stroke(rect, 0.0, (1.0, egui::Color32::GRAY), StrokeKind::Outside);
    }

    /// Draw horizontal gradient
    fn draw_gradient_horizontal(
        &self,
        ui: &egui::Ui,
        colormap: &impl ColormapTrait,
        rect: egui::Rect,
    ) {
        let painter = ui.painter();
        let width = rect.width();
        let samples = 64; // Number of gradient steps

        for i in 0..samples {
            let x_start = rect.left() + (i as f32 / samples as f32) * width;
            let x_end = rect.left() + ((i + 1) as f32 / samples as f32) * width;

            // Value goes from 0.0 (left) to 1.0 (right)
            let value = i as f32 / samples as f32;

            let color = colormap.apply(value);
            let egui_color = egui::Color32::from_rgb(color[0], color[1], color[2]);

            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x_start, rect.top()),
                    egui::pos2(x_end, rect.bottom()),
                ),
                0.0,
                egui_color,
            );
        }

        // Border
        painter.rect_stroke(rect, 0.0, (1.0, egui::Color32::GRAY), StrokeKind::Outside);
    }

    /// Draw vertical midpoint handle (triangle)
    fn draw_handle_vertical(&self, ui: &egui::Ui, rect: egui::Rect, handle_y: f32) {
        let painter = ui.painter();
        let handle_size = 8.0;

        // Left triangle
        let points_left = vec![
            egui::pos2(rect.left() - handle_size, handle_y),
            egui::pos2(rect.left(), handle_y - handle_size / 2.0),
            egui::pos2(rect.left(), handle_y + handle_size / 2.0),
        ];

        // Right triangle
        let points_right = vec![
            egui::pos2(rect.right() + handle_size, handle_y),
            egui::pos2(rect.right(), handle_y - handle_size / 2.0),
            egui::pos2(rect.right(), handle_y + handle_size / 2.0),
        ];

        let handle_color = if self.dragging {
            egui::Color32::YELLOW
        } else {
            egui::Color32::WHITE
        };

        painter.add(egui::Shape::convex_polygon(
            points_left,
            handle_color,
            (1.0, egui::Color32::BLACK),
        ));

        painter.add(egui::Shape::convex_polygon(
            points_right,
            handle_color,
            (1.0, egui::Color32::BLACK),
        ));

        // Connecting line
        painter.line_segment(
            [
                egui::pos2(rect.left(), handle_y),
                egui::pos2(rect.right(), handle_y),
            ],
            (2.0, handle_color),
        );
    }

    /// Draw horizontal midpoint handle (triangle)
    fn draw_handle_horizontal(&self, ui: &egui::Ui, rect: egui::Rect, handle_x: f32) {
        let painter = ui.painter();
        let handle_size = 8.0;

        // Top triangle
        let points_top = vec![
            egui::pos2(handle_x, rect.top() - handle_size),
            egui::pos2(handle_x - handle_size / 2.0, rect.top()),
            egui::pos2(handle_x + handle_size / 2.0, rect.top()),
        ];

        // Bottom triangle
        let points_bottom = vec![
            egui::pos2(handle_x, rect.bottom() + handle_size),
            egui::pos2(handle_x - handle_size / 2.0, rect.bottom()),
            egui::pos2(handle_x + handle_size / 2.0, rect.bottom()),
        ];

        let handle_color = if self.dragging {
            egui::Color32::YELLOW
        } else {
            egui::Color32::WHITE
        };

        painter.add(egui::Shape::convex_polygon(
            points_top,
            handle_color,
            (1.0, egui::Color32::BLACK),
        ));

        painter.add(egui::Shape::convex_polygon(
            points_bottom,
            handle_color,
            (1.0, egui::Color32::BLACK),
        ));

        // Connecting line
        painter.line_segment(
            [
                egui::pos2(handle_x, rect.top()),
                egui::pos2(handle_x, rect.bottom()),
            ],
            (2.0, handle_color),
        );
    }

    /// Format value for display
    fn format_value(&self, value: f64) -> String {
        if self.units.is_empty() {
            format!("{:.2}", value)
        } else {
            format!("{:.2} {}", value, self.units)
        }
    }
}

/// Trait for colormaps to work with the colorbar widget
///
/// Implemented by anything that can map a normalized value (0.0-1.0) to RGB
pub trait ColormapTrait {
    /// Apply colormap to a normalized value (0.0-1.0) returning RGB
    fn apply(&self, value: f32) -> [u8; 3];
}

// Implement for the existing Colormap enum (will be in image_viewer.rs)
// This will be implemented in the integration code

#[cfg(test)]
mod tests {
    use super::*;

    struct TestColormap;

    impl ColormapTrait for TestColormap {
        fn apply(&self, value: f32) -> [u8; 3] {
            let v = (value * 255.0) as u8;
            [v, v, v]
        }
    }

    #[test]
    fn test_linear_midpoint() {
        let colorbar = Colorbar::new();
        assert_eq!(colorbar.midpoint, 0.5);

        // Linear mapping: input = output
        assert!((colorbar.apply_adjustment(0.0) - 0.0).abs() < 1e-6);
        assert!((colorbar.apply_adjustment(0.5) - 0.5).abs() < 1e-6);
        assert!((colorbar.apply_adjustment(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_midpoint_adjustment_darkens() {
        let mut colorbar = Colorbar::new();
        colorbar.midpoint = 0.3; // Darkens midtones

        // At midpoint=0.3, gamma > 1.0, so midtones are darker
        let mid = colorbar.apply_adjustment(0.5);
        assert!(mid < 0.5, "Midtones should be darker");

        // But adjusted(midpoint) should still be ~0.5
        let at_midpoint = colorbar.apply_adjustment(0.3);
        assert!((at_midpoint - 0.5).abs() < 0.1, "adjusted(midpoint) ≈ 0.5");
    }

    #[test]
    fn test_midpoint_adjustment_brightens() {
        let mut colorbar = Colorbar::new();
        colorbar.midpoint = 0.7; // Brightens midtones

        // At midpoint=0.7, gamma < 1.0, so midtones are brighter
        let mid = colorbar.apply_adjustment(0.5);
        assert!(mid > 0.5, "Midtones should be brighter");

        // But adjusted(midpoint) should still be ~0.5
        let at_midpoint = colorbar.apply_adjustment(0.7);
        assert!((at_midpoint - 0.5).abs() < 0.1, "adjusted(midpoint) ≈ 0.5");
    }

    #[test]
    fn test_reset_midpoint() {
        let mut colorbar = Colorbar::new();
        colorbar.midpoint = 0.8;
        colorbar.reset_midpoint();
        assert_eq!(colorbar.midpoint, 0.5);
    }

    #[test]
    fn test_value_formatting() {
        let colorbar = Colorbar::new().value_range(0.0, 100.0).units("counts");

        assert_eq!(colorbar.format_value(50.0), "50.00 counts");
    }

    #[test]
    fn test_value_formatting_no_units() {
        let colorbar = Colorbar::new().value_range(0.0, 1.0);

        assert_eq!(colorbar.format_value(0.5), "0.50");
    }
}
