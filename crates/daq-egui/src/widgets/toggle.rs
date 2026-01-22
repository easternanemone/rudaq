//! Toggle switch widget for boolean values.
//!
//! This widget is available for integration but not currently used.
#![allow(dead_code)]

use egui::{Response, Sense, Ui, Vec2, Widget};
use tracing;

use crate::layout;

pub struct Toggle<'a> {
    value: &'a mut bool,
    label: Option<&'a str>,
    size: ToggleSize,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ToggleSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ToggleSize {
    fn dimensions(&self) -> (f32, f32) {
        match self {
            ToggleSize::Small => (28.0, 16.0),
            ToggleSize::Medium => (36.0, 20.0),
            ToggleSize::Large => (44.0, 24.0),
        }
    }
}

impl<'a> Toggle<'a> {
    pub fn new(value: &'a mut bool) -> Self {
        Self {
            value,
            label: None,
            size: ToggleSize::default(),
        }
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn size(mut self, size: ToggleSize) -> Self {
        self.size = size;
        self
    }

    pub fn small(mut self) -> Self {
        self.size = ToggleSize::Small;
        self
    }

    pub fn large(mut self) -> Self {
        self.size = ToggleSize::Large;
        self
    }
}

impl Widget for Toggle<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (width, height) = self.size.dimensions();
        let knob_radius = height / 2.0 - 2.0;
        let padding = 2.0;

        let total_width = if let Some(label) = self.label {
            let galley = ui.painter().layout_no_wrap(
                label.to_string(),
                egui::FontId::default(),
                ui.visuals().text_color(),
            );
            width + 8.0 + galley.size().x
        } else {
            width
        };

        let desired_size = Vec2::new(total_width, height);
        let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click());

        if response.clicked() {
            let old_value = *self.value;
            *self.value = !*self.value;
            tracing::info!("[Toggle] CLICKED! old={} new={}", old_value, *self.value);
            response.mark_changed();
        }

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let visuals = ui.visuals();

            let track_rect = egui::Rect::from_min_size(rect.min, Vec2::new(width, height));
            let rounding = height / 2.0;

            let (track_color, knob_color) = if *self.value {
                (layout::colors::ACCENT, egui::Color32::WHITE)
            } else {
                (
                    visuals.widgets.inactive.bg_fill,
                    visuals.widgets.inactive.fg_stroke.color,
                )
            };

            let track_color = if response.hovered() {
                if *self.value {
                    layout::colors::ACCENT_HOVER
                } else {
                    visuals.widgets.hovered.bg_fill
                }
            } else {
                track_color
            };

            painter.rect_filled(track_rect, rounding, track_color);

            let animation_t = ui.ctx().animate_bool_responsive(response.id, *self.value);

            let knob_x = egui::lerp(
                track_rect.left() + padding + knob_radius
                    ..=track_rect.right() - padding - knob_radius,
                animation_t,
            );
            let knob_center = egui::pos2(knob_x, track_rect.center().y);

            painter.circle_filled(knob_center, knob_radius, knob_color);

            if let Some(label) = self.label {
                let label_pos = egui::pos2(track_rect.right() + 8.0, rect.center().y);
                painter.text(
                    label_pos,
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::default(),
                    visuals.text_color(),
                );
            }
        }

        response
    }
}

pub fn toggle(value: &mut bool) -> Toggle<'_> {
    Toggle::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toggle_size_dimensions() {
        let (w, h) = ToggleSize::Small.dimensions();
        assert_eq!((w, h), (28.0, 16.0));

        let (w, h) = ToggleSize::Medium.dimensions();
        assert_eq!((w, h), (36.0, 20.0));

        let (w, h) = ToggleSize::Large.dimensions();
        assert_eq!((w, h), (44.0, 24.0));
    }
}
