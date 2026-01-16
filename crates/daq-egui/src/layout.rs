use egui::{Color32, CornerRadius, Stroke, Vec2};

pub const NAV_PANEL_WIDTH: f32 = 200.0;
pub const SIDE_PANEL_WIDTH: f32 = 300.0;
pub const STATUS_BAR_HEIGHT: f32 = 24.0;

pub const ITEM_SPACING: Vec2 = Vec2::new(6.0, 8.0);
pub const SECTION_SPACING: f32 = 16.0;
pub const PANEL_PADDING: f32 = 8.0;

pub const ICON_SIZE_INLINE: f32 = 16.0;
pub const ICON_SIZE_BUTTON: f32 = 20.0;
pub const ICON_SIZE_LARGE: f32 = 32.0;

pub const CARD_ROUNDING: CornerRadius = CornerRadius::same(4);
pub const BUTTON_ROUNDING: CornerRadius = CornerRadius::same(4);

pub mod colors {
    use super::*;

    pub const SUCCESS: Color32 = Color32::from_rgb(34, 197, 94);
    pub const ERROR: Color32 = Color32::from_rgb(239, 68, 68);
    pub const WARNING: Color32 = Color32::from_rgb(234, 179, 8);
    pub const INFO: Color32 = Color32::from_rgb(59, 130, 246);

    pub const CONNECTED: Color32 = SUCCESS;
    pub const DISCONNECTED: Color32 = Color32::from_rgb(156, 163, 175);
    pub const CONNECTING: Color32 = WARNING;
    pub const RECONNECTING: Color32 = WARNING;

    pub const ACCENT: Color32 = Color32::from_rgb(99, 102, 241);
    pub const ACCENT_HOVER: Color32 = Color32::from_rgb(129, 140, 248);

    pub const MUTED: Color32 = Color32::from_rgb(107, 114, 128);
    pub const BORDER: Color32 = Color32::from_rgb(55, 65, 81);
}

pub fn card_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .corner_radius(CARD_ROUNDING)
        .inner_margin(PANEL_PADDING)
        .stroke(Stroke::new(1.0, colors::BORDER))
}

pub fn section_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(CARD_ROUNDING)
        .inner_margin(PANEL_PADDING)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_constants() {
        assert_eq!(NAV_PANEL_WIDTH, 200.0);
        assert_eq!(SIDE_PANEL_WIDTH, 300.0);
        assert_eq!(STATUS_BAR_HEIGHT, 24.0);
    }

    #[test]
    fn test_spacing_constants() {
        assert_eq!(ITEM_SPACING, Vec2::new(6.0, 8.0));
        assert_eq!(SECTION_SPACING, 16.0);
        assert_eq!(PANEL_PADDING, 8.0);
    }

    #[test]
    fn test_icon_sizes() {
        assert!(ICON_SIZE_INLINE < ICON_SIZE_BUTTON);
        assert!(ICON_SIZE_BUTTON < ICON_SIZE_LARGE);
    }

    #[test]
    fn test_status_colors_distinct() {
        assert_ne!(colors::SUCCESS, colors::ERROR);
        assert_ne!(colors::WARNING, colors::INFO);
        assert_ne!(colors::SUCCESS, colors::WARNING);
    }

    #[test]
    fn test_connection_colors() {
        assert_eq!(colors::CONNECTED, colors::SUCCESS);
        assert_eq!(colors::CONNECTING, colors::WARNING);
        assert_eq!(colors::RECONNECTING, colors::WARNING);
    }
}
