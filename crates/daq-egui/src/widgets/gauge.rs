//! Radial and linear gauge widgets for value visualization.
//!
//! These widgets are available for integration but not currently used.
#![allow(dead_code)]

use egui::{Color32, Pos2, Response, Sense, Stroke, Ui, Vec2, Widget};

use crate::layout;

#[derive(Debug, Clone)]
pub struct GaugeThresholds {
    pub warning: f32,
    pub critical: f32,
}

impl Default for GaugeThresholds {
    fn default() -> Self {
        Self {
            warning: 0.7,
            critical: 0.9,
        }
    }
}

pub struct Gauge<'a> {
    value: f32,
    min: f32,
    max: f32,
    label: Option<&'a str>,
    unit: Option<&'a str>,
    size: f32,
    thresholds: Option<GaugeThresholds>,
}

impl<'a> Gauge<'a> {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            min: 0.0,
            max: 100.0,
            label: None,
            unit: None,
            size: 60.0,
            thresholds: None,
        }
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = Some(unit);
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn thresholds(mut self, thresholds: GaugeThresholds) -> Self {
        self.thresholds = Some(thresholds);
        self
    }

    fn normalized_value(&self) -> f32 {
        ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    fn value_color(&self) -> Color32 {
        let norm = self.normalized_value();
        match &self.thresholds {
            Some(t) if norm >= t.critical => layout::colors::ERROR,
            Some(t) if norm >= t.warning => layout::colors::WARNING,
            _ => layout::colors::SUCCESS,
        }
    }
}

impl Widget for Gauge<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let desired_size = Vec2::splat(self.size);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let center = rect.center();
            let radius = self.size / 2.0 - 4.0;

            let bg_color = ui.visuals().widgets.noninteractive.bg_fill;
            let track_color = ui.visuals().widgets.inactive.bg_fill;
            let value_color = self.value_color();

            painter.circle_filled(center, radius, bg_color);

            let stroke_width = 4.0;
            let track_radius = radius - stroke_width / 2.0;

            painter.circle_stroke(center, track_radius, Stroke::new(stroke_width, track_color));

            let norm = self.normalized_value();
            if norm > 0.0 {
                let start_angle = -std::f32::consts::FRAC_PI_2;
                let sweep = norm * std::f32::consts::TAU;

                let segments = (sweep / 0.1).ceil() as usize;
                let mut points = Vec::with_capacity(segments + 1);

                for i in 0..=segments {
                    let t = i as f32 / segments as f32;
                    let angle = start_angle + t * sweep;
                    let x = center.x + track_radius * angle.cos();
                    let y = center.y + track_radius * angle.sin();
                    points.push(Pos2::new(x, y));
                }

                if points.len() >= 2 {
                    painter.add(egui::Shape::line(
                        points,
                        Stroke::new(stroke_width, value_color),
                    ));
                }
            }

            let value_text = if self.value.abs() < 10.0 {
                format!("{:.1}", self.value)
            } else {
                format!("{:.0}", self.value)
            };

            let text = match self.unit {
                Some(u) => format!("{}{}", value_text, u),
                None => value_text,
            };

            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                &text,
                egui::FontId::proportional(self.size / 4.0),
                ui.visuals().text_color(),
            );

            if let Some(label) = self.label {
                let label_pos = Pos2::new(center.x, rect.bottom() + 2.0);
                painter.text(
                    label_pos,
                    egui::Align2::CENTER_TOP,
                    label,
                    egui::FontId::proportional(10.0),
                    layout::colors::MUTED,
                );
            }
        }

        response
    }
}

pub struct LinearGauge<'a> {
    value: f32,
    min: f32,
    max: f32,
    label: Option<&'a str>,
    width: f32,
    height: f32,
    thresholds: Option<GaugeThresholds>,
}

impl<'a> LinearGauge<'a> {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            min: 0.0,
            max: 100.0,
            label: None,
            width: 100.0,
            height: 8.0,
            thresholds: None,
        }
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn thresholds(mut self, thresholds: GaugeThresholds) -> Self {
        self.thresholds = Some(thresholds);
        self
    }

    fn normalized_value(&self) -> f32 {
        ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    fn value_color(&self) -> Color32 {
        let norm = self.normalized_value();
        match &self.thresholds {
            Some(t) if norm >= t.critical => layout::colors::ERROR,
            Some(t) if norm >= t.warning => layout::colors::WARNING,
            _ => layout::colors::SUCCESS,
        }
    }
}

impl Widget for LinearGauge<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let total_height = if self.label.is_some() {
            self.height + 14.0
        } else {
            self.height
        };
        let desired_size = Vec2::new(self.width, total_height);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();

            let bar_rect = if self.label.is_some() {
                egui::Rect::from_min_size(rect.min, Vec2::new(self.width, self.height))
            } else {
                rect
            };

            let rounding = self.height / 2.0;
            let track_color = ui.visuals().widgets.inactive.bg_fill;
            painter.rect_filled(bar_rect, rounding, track_color);

            let norm = self.normalized_value();
            if norm > 0.0 {
                let fill_width = bar_rect.width() * norm;
                let fill_rect = egui::Rect::from_min_size(
                    bar_rect.min,
                    Vec2::new(fill_width, bar_rect.height()),
                );
                painter.rect_filled(fill_rect, rounding, self.value_color());
            }

            if let Some(label) = self.label {
                let label_pos = Pos2::new(rect.left(), bar_rect.bottom() + 2.0);
                painter.text(
                    label_pos,
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(10.0),
                    layout::colors::MUTED,
                );
            }
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gauge_normalized_value() {
        let gauge = Gauge::new(50.0).range(0.0, 100.0);
        assert!((gauge.normalized_value() - 0.5).abs() < 0.001);

        let gauge = Gauge::new(25.0).range(0.0, 100.0);
        assert!((gauge.normalized_value() - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_gauge_clamping() {
        let gauge = Gauge::new(150.0).range(0.0, 100.0);
        assert!((gauge.normalized_value() - 1.0).abs() < 0.001);

        let gauge = Gauge::new(-10.0).range(0.0, 100.0);
        assert!(gauge.normalized_value().abs() < 0.001);
    }

    #[test]
    fn test_gauge_color_thresholds() {
        let thresholds = GaugeThresholds {
            warning: 0.7,
            critical: 0.9,
        };

        let gauge = Gauge::new(50.0)
            .range(0.0, 100.0)
            .thresholds(thresholds.clone());
        assert_eq!(gauge.value_color(), layout::colors::SUCCESS);

        let gauge = Gauge::new(75.0)
            .range(0.0, 100.0)
            .thresholds(thresholds.clone());
        assert_eq!(gauge.value_color(), layout::colors::WARNING);

        let gauge = Gauge::new(95.0).range(0.0, 100.0).thresholds(thresholds);
        assert_eq!(gauge.value_color(), layout::colors::ERROR);
    }
}
