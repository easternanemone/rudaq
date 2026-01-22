use egui::{Color32, Context, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemePreference {
    Light,
    #[default]
    Dark,
    System,
}

impl ThemePreference {
    pub fn cycle(&mut self) {
        *self = match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
            Self::System => Self::Light,
        };
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Light => crate::icons::SUN,
            Self::Dark => crate::icons::MOON,
            Self::System => crate::icons::DESKTOP,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::System => "System",
        }
    }

    pub fn is_dark(&self) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            Self::System => {
                // Detect system preference - default to dark if unavailable
                #[cfg(feature = "dark-light")]
                {
                    dark_light::detect().map_or(true, |mode| mode == dark_light::Mode::Dark)
                }
                #[cfg(not(feature = "dark-light"))]
                {
                    // Fall back to dark mode when dark-light crate not available
                    true
                }
            }
        }
    }
}

pub fn apply_theme(ctx: &Context, preference: ThemePreference) {
    let visuals = if preference.is_dark() {
        dark_visuals()
    } else {
        light_visuals()
    };
    ctx.set_visuals(visuals);

    // Disable debug visualization in debug builds
    #[cfg(debug_assertions)]
    ctx.style_mut(|style| {
        style.debug.debug_on_hover = false;
        style.debug.debug_on_hover_with_all_modifiers = false;
        style.debug.show_unaligned = false;
    });
}

fn dark_visuals() -> Visuals {
    let mut visuals = Visuals::dark();

    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(30, 30, 35);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(45, 45, 55);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(60, 60, 75);
    visuals.widgets.active.bg_fill = Color32::from_rgb(75, 75, 95);

    visuals.selection.bg_fill = Color32::from_rgb(99, 102, 241);
    visuals.selection.stroke.color = Color32::WHITE;

    visuals.extreme_bg_color = Color32::from_rgb(20, 20, 25);
    visuals.faint_bg_color = Color32::from_rgb(35, 35, 42);

    visuals.window_fill = Color32::from_rgb(25, 25, 30);
    visuals.panel_fill = Color32::from_rgb(25, 25, 30);

    visuals
}

fn light_visuals() -> Visuals {
    let mut visuals = Visuals::light();

    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(245, 245, 250);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(235, 235, 242);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(225, 225, 235);
    visuals.widgets.active.bg_fill = Color32::from_rgb(210, 210, 225);

    visuals.selection.bg_fill = Color32::from_rgb(99, 102, 241);
    visuals.selection.stroke.color = Color32::WHITE;

    visuals.extreme_bg_color = Color32::from_rgb(255, 255, 255);
    visuals.faint_bg_color = Color32::from_rgb(250, 250, 252);

    visuals.window_fill = Color32::from_rgb(250, 250, 252);
    visuals.panel_fill = Color32::from_rgb(250, 250, 252);

    visuals
}

pub fn theme_toggle_button(ui: &mut egui::Ui, preference: &mut ThemePreference) -> bool {
    let response = ui.add(
        egui::Button::new(
            egui::RichText::new(preference.icon()).size(crate::layout::ICON_SIZE_BUTTON),
        )
        .frame(false),
    );

    let changed = response.clicked();
    if changed {
        preference.cycle();
    }

    response.on_hover_text(format!("Theme: {} (click to change)", preference.label()));

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_preference_default() {
        let pref = ThemePreference::default();
        assert_eq!(pref, ThemePreference::Dark);
    }

    #[test]
    fn test_theme_preference_cycle() {
        let mut pref = ThemePreference::Light;
        pref.cycle();
        assert_eq!(pref, ThemePreference::Dark);
        pref.cycle();
        assert_eq!(pref, ThemePreference::System);
        pref.cycle();
        assert_eq!(pref, ThemePreference::Light);
    }

    #[test]
    fn test_theme_preference_labels() {
        assert_eq!(ThemePreference::Light.label(), "Light");
        assert_eq!(ThemePreference::Dark.label(), "Dark");
        assert_eq!(ThemePreference::System.label(), "System");
    }

    #[test]
    fn test_theme_is_dark() {
        assert!(!ThemePreference::Light.is_dark());
        assert!(ThemePreference::Dark.is_dark());
    }

    #[test]
    fn test_dark_visuals_colors() {
        let visuals = dark_visuals();
        assert_eq!(visuals.window_fill, Color32::from_rgb(25, 25, 30));
        assert_eq!(visuals.extreme_bg_color, Color32::from_rgb(20, 20, 25));
    }

    #[test]
    fn test_light_visuals_colors() {
        let visuals = light_visuals();
        assert_eq!(visuals.window_fill, Color32::from_rgb(250, 250, 252));
        assert_eq!(visuals.extreme_bg_color, Color32::from_rgb(255, 255, 255));
    }

    #[test]
    fn test_theme_serialization() {
        let pref = ThemePreference::Dark;
        let json = serde_json::to_string(&pref).unwrap();
        assert_eq!(json, "\"Dark\"");

        let deserialized: ThemePreference = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, pref);
    }
}
