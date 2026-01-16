use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::{Align2, Area, Color32, Context, Frame, Id, Order, Pos2, RichText, Vec2};

use crate::icons;
use crate::layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastLevel {
    fn icon(&self) -> &'static str {
        match self {
            ToastLevel::Info => icons::status::INFO,
            ToastLevel::Success => icons::status::SUCCESS,
            ToastLevel::Warning => icons::status::WARNING,
            ToastLevel::Error => icons::status::ERROR,
        }
    }

    fn color(&self) -> Color32 {
        match self {
            ToastLevel::Info => layout::colors::INFO,
            ToastLevel::Success => layout::colors::SUCCESS,
            ToastLevel::Warning => layout::colors::WARNING,
            ToastLevel::Error => layout::colors::ERROR,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub created_at: Instant,
    pub duration: Duration,
}

impl Toast {
    pub fn new(message: impl Into<String>, level: ToastLevel) -> Self {
        Self {
            message: message.into(),
            level,
            created_at: Instant::now(),
            duration: Duration::from_secs(4),
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Info)
    }

    pub fn success(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Success)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Warning)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Error)
    }

    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.duration
    }

    fn remaining_fraction(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_secs_f32();
        let total = self.duration.as_secs_f32();
        1.0 - (elapsed / total).clamp(0.0, 1.0)
    }
}

#[derive(Default)]
pub struct Toasts {
    toasts: VecDeque<Toast>,
    max_toasts: usize,
}

impl Toasts {
    pub fn new() -> Self {
        Self {
            toasts: VecDeque::new(),
            max_toasts: 5,
        }
    }

    pub fn max_toasts(mut self, max: usize) -> Self {
        self.max_toasts = max;
        self
    }

    pub fn add(&mut self, toast: Toast) {
        self.toasts.push_back(toast);
        while self.toasts.len() > self.max_toasts {
            self.toasts.pop_front();
        }
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.add(Toast::info(message));
    }

    pub fn success(&mut self, message: impl Into<String>) {
        self.add(Toast::success(message));
    }

    pub fn warning(&mut self, message: impl Into<String>) {
        self.add(Toast::warning(message));
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.add(Toast::error(message));
    }

    pub fn show(&mut self, ctx: &Context) {
        self.toasts.retain(|t| !t.is_expired());

        if self.toasts.is_empty() {
            return;
        }

        ctx.request_repaint();

        let screen_rect = ctx.input(|i| i.screen_rect());
        let margin = 16.0;
        let toast_width = 300.0;
        let toast_spacing = 8.0;

        let _base_pos = Pos2::new(
            screen_rect.right() - margin - toast_width,
            screen_rect.bottom() - margin - layout::STATUS_BAR_HEIGHT,
        );

        let mut y_offset = 0.0;
        let mut to_remove = Vec::new();

        for (i, toast) in self.toasts.iter().enumerate().rev() {
            let toast_id = Id::new("toast").with(i);

            let response = Area::new(toast_id)
                .order(Order::Foreground)
                .anchor(
                    Align2::RIGHT_BOTTOM,
                    Vec2::new(-margin, -margin - layout::STATUS_BAR_HEIGHT - y_offset),
                )
                .show(ctx, |ui| {
                    let opacity = toast.remaining_fraction().min(1.0);
                    let alpha = (opacity * 255.0) as u8;

                    Frame::popup(ui.style())
                        .fill(ui.visuals().window_fill.gamma_multiply(opacity))
                        .stroke(egui::Stroke::new(
                            1.0,
                            toast.level.color().gamma_multiply(opacity),
                        ))
                        .inner_margin(12.0)
                        .corner_radius(8.0)
                        .show(ui, |ui| {
                            ui.set_max_width(toast_width - 24.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(toast.level.icon())
                                        .color(Color32::from_rgba_unmultiplied(
                                            toast.level.color().r(),
                                            toast.level.color().g(),
                                            toast.level.color().b(),
                                            alpha,
                                        ))
                                        .size(18.0),
                                );
                                ui.label(RichText::new(&toast.message).color(
                                    Color32::from_rgba_unmultiplied(
                                        ui.visuals().text_color().r(),
                                        ui.visuals().text_color().g(),
                                        ui.visuals().text_color().b(),
                                        alpha,
                                    ),
                                ));

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("×").clicked() {
                                            to_remove.push(i);
                                        }
                                    },
                                );
                            });
                        });
                });

            y_offset += response.response.rect.height() + toast_spacing;
        }

        for i in to_remove.into_iter().rev() {
            self.toasts.remove(i);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.toasts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toast_creation() {
        let toast = Toast::info("Test message");
        assert_eq!(toast.level, ToastLevel::Info);
        assert_eq!(toast.message, "Test message");
    }

    #[test]
    fn test_toast_levels() {
        assert_eq!(Toast::success("").level, ToastLevel::Success);
        assert_eq!(Toast::warning("").level, ToastLevel::Warning);
        assert_eq!(Toast::error("").level, ToastLevel::Error);
    }

    #[test]
    fn test_toasts_max_limit() {
        let mut toasts = Toasts::new().max_toasts(3);
        toasts.info("1");
        toasts.info("2");
        toasts.info("3");
        toasts.info("4");
        assert_eq!(toasts.len(), 3);
    }

    #[test]
    fn test_toast_expiration() {
        let toast = Toast::info("Test").duration(Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        assert!(toast.is_expired());
    }
}
