//! Double slider widget for range selection.
//!
//! Provides a slider with two draggable handles for selecting a min/max range.
//! Useful for histogram clipping bounds, ROI coordinate ranges, time windows, etc.
//!
//! ## Features
//! - Two draggable handles on a single track
//! - Highlighted region between handles
//! - Optional value labels at handles
//! - Configurable step size and bounds
//! - Smooth animations using egui's animation system
//!
//! ## Keyboard Support
//! Currently supports mouse/touch interaction. Keyboard support (arrow keys when focused)
//! can be added in a future enhancement by implementing key event handling in the widget.

use egui::{pos2, Color32, PointerButton, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, Widget};

use crate::layout;

/// A double-handle slider for selecting a range [min, max].
///
/// # Example
/// ```ignore
/// let mut range = (0.0, 100.0);
/// let mut bounds = (0.0, 100.0);
/// ui.add(DoubleSlider::new(&mut range, &mut bounds));
/// ```
pub struct DoubleSlider<'a> {
    range: &'a mut (f64, f64),
    bounds: &'a mut (f64, f64),
    step: Option<f64>,
    show_labels: bool,
    label_format: Box<dyn Fn(f64) -> String + 'a>,
    width: Option<f32>,
    height: f32,
}

impl<'a> DoubleSlider<'a> {
    /// Create a new double slider.
    ///
    /// # Arguments
    /// * `range` - Mutable reference to the current (min, max) values
    /// * `bounds` - Mutable reference to the allowed (min, max) bounds
    pub fn new(range: &'a mut (f64, f64), bounds: &'a mut (f64, f64)) -> Self {
        Self {
            range,
            bounds,
            step: None,
            show_labels: true,
            label_format: Box::new(|v| format!("{:.2}", v)),
            width: None,
            height: 24.0,
        }
    }

    /// Set the step size for snapping values.
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Set whether to show value labels at the handles.
    pub fn show_labels(mut self, show: bool) -> Self {
        self.show_labels = show;
        self
    }

    /// Set a custom label formatter.
    pub fn label_format<F>(mut self, formatter: F) -> Self
    where
        F: Fn(f64) -> String + 'a,
    {
        self.label_format = Box::new(formatter);
        self
    }

    /// Set the slider width (defaults to available width).
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Set the slider height.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    fn snap_to_step(&self, value: f64) -> f64 {
        if let Some(step) = self.step {
            (value / step).round() * step
        } else {
            value
        }
    }

    fn normalize(&self, value: f64) -> f32 {
        let (min, max) = *self.bounds;
        if (max - min).abs() < f64::EPSILON {
            0.5
        } else {
            ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
        }
    }

    fn denormalize(&self, t: f32) -> f64 {
        let (min, max) = *self.bounds;
        min + (max - min) * t as f64
    }
}

impl Widget for DoubleSlider<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let width = self.width.unwrap_or(ui.available_width());
        let handle_radius = self.height / 2.0 - 2.0;
        let track_height = 4.0;

        // Calculate total height including labels
        let total_height = if self.show_labels {
            self.height + 20.0 // Extra space for labels
        } else {
            self.height
        };

        let desired_size = Vec2::new(width, total_height);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

        // Track area (centered vertically in the handle area)
        let track_y = rect.min.y + (self.height - track_height) / 2.0;
        let track_rect = Rect::from_min_size(
            pos2(rect.min.x + handle_radius, track_y),
            Vec2::new(width - 2.0 * handle_radius, track_height),
        );

        let mut changed = false;

        // Handle dragging
        if response.dragged_by(PointerButton::Primary) {
            if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
                let x = pointer_pos.x.clamp(track_rect.min.x, track_rect.max.x);
                let t = (x - track_rect.min.x) / track_rect.width();
                let value = self.snap_to_step(self.denormalize(t));

                // Determine which handle is closest
                let (min_val, max_val) = *self.range;
                let min_t = self.normalize(min_val);
                let max_t = self.normalize(max_val);
                let min_x = track_rect.min.x + min_t * track_rect.width();
                let max_x = track_rect.min.x + max_t * track_rect.width();

                let dist_to_min = (x - min_x).abs();
                let dist_to_max = (x - max_x).abs();

                if dist_to_min < dist_to_max {
                    // Dragging min handle
                    self.range.0 = value.min(self.range.1).clamp(self.bounds.0, self.bounds.1);
                } else {
                    // Dragging max handle
                    self.range.1 = value.max(self.range.0).clamp(self.bounds.0, self.bounds.1);
                }
                changed = true;
            }
        }

        // Render if visible
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let visuals = ui.visuals();

            // Draw track background
            let track_bg_color = visuals.widgets.inactive.bg_fill;
            painter.rect_filled(track_rect, 2.0, track_bg_color);

            // Draw selected range
            let (min_val, max_val) = *self.range;
            let min_t = self.normalize(min_val);
            let max_t = self.normalize(max_val);
            let range_rect = Rect::from_min_max(
                pos2(
                    track_rect.min.x + min_t * track_rect.width(),
                    track_rect.min.y,
                ),
                pos2(
                    track_rect.min.x + max_t * track_rect.width(),
                    track_rect.max.y,
                ),
            );
            painter.rect_filled(range_rect, 2.0, layout::colors::ACCENT);

            // Draw handles
            let handle_color = if response.hovered() {
                Color32::WHITE
            } else {
                visuals.widgets.inactive.fg_stroke.color
            };
            let handle_stroke = Stroke::new(2.0, layout::colors::ACCENT);

            let min_center = pos2(
                track_rect.min.x + min_t * track_rect.width(),
                rect.min.y + self.height / 2.0,
            );
            let max_center = pos2(
                track_rect.min.x + max_t * track_rect.width(),
                rect.min.y + self.height / 2.0,
            );

            painter.circle_filled(min_center, handle_radius, handle_color);
            painter.circle_stroke(min_center, handle_radius, handle_stroke);

            painter.circle_filled(max_center, handle_radius, handle_color);
            painter.circle_stroke(max_center, handle_radius, handle_stroke);

            // Draw labels
            if self.show_labels {
                let text_color = visuals.text_color();
                let font_id = egui::FontId::proportional(12.0);

                let min_label = (self.label_format)(min_val);
                let max_label = (self.label_format)(max_val);

                let label_y = rect.min.y + self.height + 4.0;

                painter.text(
                    pos2(min_center.x, label_y),
                    egui::Align2::CENTER_TOP,
                    min_label,
                    font_id.clone(),
                    text_color,
                );

                painter.text(
                    pos2(max_center.x, label_y),
                    egui::Align2::CENTER_TOP,
                    max_label,
                    font_id,
                    text_color,
                );
            }
        }

        if changed {
            response.mark_changed();
            ui.ctx().request_repaint();
        }

        response
    }
}

/// Convenience function to create a double slider.
pub fn double_slider<'a>(
    range: &'a mut (f64, f64),
    bounds: &'a mut (f64, f64),
) -> DoubleSlider<'a> {
    DoubleSlider::new(range, bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        let mut range = (25.0, 75.0);
        let mut bounds = (0.0, 100.0);
        let slider = DoubleSlider::new(&mut range, &mut bounds);

        assert!((slider.normalize(0.0) - 0.0).abs() < 1e-6);
        assert!((slider.normalize(50.0) - 0.5).abs() < 1e-6);
        assert!((slider.normalize(100.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_denormalize() {
        let mut range = (25.0, 75.0);
        let mut bounds = (0.0, 100.0);
        let slider = DoubleSlider::new(&mut range, &mut bounds);

        assert!((slider.denormalize(0.0) - 0.0).abs() < 1e-6);
        assert!((slider.denormalize(0.5) - 50.0).abs() < 1e-6);
        assert!((slider.denormalize(1.0) - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_snap_to_step() {
        let mut range = (25.0, 75.0);
        let mut bounds = (0.0, 100.0);
        let slider = DoubleSlider::new(&mut range, &mut bounds).step(10.0);

        assert_eq!(slider.snap_to_step(23.0), 20.0);
        assert_eq!(slider.snap_to_step(27.0), 30.0);
        assert_eq!(slider.snap_to_step(25.0), 30.0);
    }

    #[test]
    fn test_snap_to_step_none() {
        let mut range = (25.0, 75.0);
        let mut bounds = (0.0, 100.0);
        let slider = DoubleSlider::new(&mut range, &mut bounds);

        assert_eq!(slider.snap_to_step(23.456), 23.456);
    }

    #[test]
    fn test_normalize_zero_range() {
        let mut range = (50.0, 50.0);
        let mut bounds = (50.0, 50.0);
        let slider = DoubleSlider::new(&mut range, &mut bounds);

        // Should not panic, should return 0.5
        assert_eq!(slider.normalize(50.0), 0.5);
    }
}
