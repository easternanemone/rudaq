//! Handles keyboard shortcuts for the GUI.
use eframe::egui;

/// Represents an action triggered by a keyboard shortcut.
pub enum ShortcutAction {
    ToggleInstrumentWindow,
}

/// Handles keyboard shortcuts and returns an action if a shortcut is triggered.
pub fn handle_shortcuts(ctx: &egui::Context) -> Option<ShortcutAction> {
    if ctx.input(|i| i.key_pressed(egui::Key::I) && i.modifiers.ctrl) {
        return Some(ShortcutAction::ToggleInstrumentWindow);
    }

    None
}
