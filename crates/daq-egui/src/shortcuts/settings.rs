//! Keyboard shortcuts settings panel

use eframe::egui;

use super::action::ShortcutAction;
use super::manager::{KeyBinding, ShortcutManager};

/// Settings panel for customizing keyboard shortcuts
#[derive(Default)]
pub struct ShortcutSettingsPanel {
    /// Action currently being edited
    editing_action: Option<ShortcutAction>,
    /// Temporary binding for the action being edited
    temp_binding: Option<KeyBinding>,
    /// Conflict warning message
    conflict_message: Option<String>,
}

impl ShortcutSettingsPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Render the settings panel
    pub fn ui(&mut self, ui: &mut egui::Ui, shortcuts: &mut ShortcutManager) {
        ui.heading("Keyboard Shortcuts");
        ui.separator();

        ui.label("Customize keyboard shortcuts for the application.");
        ui.label("Click on a shortcut to change it, then press the new key combination.");
        ui.add_space(8.0);

        // Reset to defaults button
        ui.horizontal(|ui| {
            if ui.button("Reset to Defaults").clicked() {
                shortcuts.reset_to_defaults();
                self.editing_action = None;
                self.temp_binding = None;
                self.conflict_message = None;
            }
        });

        ui.add_space(12.0);

        // Group shortcuts by context
        let actions_by_context = shortcuts.all_actions_by_context();

        // Display contexts in order
        let contexts = vec![
            super::action::ShortcutContext::Global,
            super::action::ShortcutContext::ImageViewer,
            super::action::ShortcutContext::SignalPlotter,
        ];

        for context in contexts {
            if let Some(actions) = actions_by_context.get(&context) {
                if !actions.is_empty() {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new(context.label()).heading().strong());
                        ui.separator();

                        for action in actions {
                            self.render_shortcut_row(ui, shortcuts, *action);
                        }
                    });

                    ui.add_space(12.0);
                }
            }
        }

        // Show conflict warning if present
        if let Some(ref msg) = self.conflict_message {
            ui.colored_label(egui::Color32::RED, msg);
        }
    }

    /// Render a single shortcut configuration row
    fn render_shortcut_row(
        &mut self,
        ui: &mut egui::Ui,
        shortcuts: &mut ShortcutManager,
        action: ShortcutAction,
    ) {
        ui.horizontal(|ui| {
            // Action description
            ui.label(action.description());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let current_binding = shortcuts.get_binding(action);

                if self.editing_action == Some(action) {
                    // Editing mode - capture key press
                    ui.label(egui::RichText::new("Press a key...").italics());

                    // Capture key input
                    if let Some(new_binding) = self.capture_key_press(ui.ctx()) {
                        self.temp_binding = Some(new_binding);

                        // Check for conflicts
                        if let Some(conflict) = shortcuts.find_conflict(action, new_binding) {
                            self.conflict_message = Some(format!(
                                "Key {} is already bound to {}",
                                new_binding.label(),
                                conflict.description()
                            ));
                        } else {
                            // No conflict - apply the binding
                            shortcuts.set_binding(action, new_binding);
                            self.editing_action = None;
                            self.temp_binding = None;
                            self.conflict_message = None;
                        }
                    }

                    // Cancel button
                    if ui.small_button("Cancel").clicked() {
                        self.editing_action = None;
                        self.temp_binding = None;
                        self.conflict_message = None;
                    }
                } else if let Some(binding) = current_binding {
                    // Display mode - show current binding
                    let button_text = binding.label();
                    if ui
                        .button(egui::RichText::new(button_text).monospace())
                        .clicked()
                    {
                        self.editing_action = Some(action);
                        self.temp_binding = None;
                        self.conflict_message = None;
                    }
                } else {
                    // No binding set
                    if ui.button("Set...").clicked() {
                        self.editing_action = Some(action);
                        self.temp_binding = None;
                        self.conflict_message = None;
                    }
                }
            });
        });
    }

    /// Capture a key press and return the binding
    fn capture_key_press(&self, ctx: &egui::Context) -> Option<KeyBinding> {
        ctx.input(|i| {
            // Check for any key press
            for key in &[
                egui::Key::Space,
                egui::Key::Enter,
                egui::Key::Escape,
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
                egui::Key::Num0,
                egui::Key::Plus,
                egui::Key::Minus,
                egui::Key::F,
                egui::Key::R,
                egui::Key::C,
                egui::Key::H,
                egui::Key::M,
                egui::Key::S,
                egui::Key::Comma,
                egui::Key::Slash,
            ] {
                if i.key_pressed(*key) {
                    return Some(KeyBinding {
                        key: *key,
                        ctrl: i.modifiers.ctrl,
                        shift: i.modifiers.shift,
                        alt: i.modifiers.alt,
                    });
                }
            }
            None
        })
    }
}
