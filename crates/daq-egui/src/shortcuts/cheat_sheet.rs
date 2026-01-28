//! Keyboard shortcuts cheat sheet panel

use eframe::egui;

use super::action::ShortcutContext;
use super::manager::ShortcutManager;

/// Cheat sheet panel showing all available keyboard shortcuts
#[derive(Default)]
pub struct CheatSheetPanel {
    /// Currently selected context filter
    selected_context: Option<ShortcutContext>,
}

impl CheatSheetPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Render the cheat sheet as a window
    pub fn show(&mut self, ctx: &egui::Context, open: &mut bool, shortcuts: &ShortcutManager) {
        egui::Window::new("⌨ Keyboard Shortcuts")
            .open(open)
            .default_width(500.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.ui(ui, shortcuts);
            });
    }

    /// Render the cheat sheet UI
    pub fn ui(&mut self, ui: &mut egui::Ui, shortcuts: &ShortcutManager) {
        ui.heading("Keyboard Shortcuts");
        ui.separator();

        // Context filter
        ui.horizontal(|ui| {
            ui.label("Show shortcuts for:");
            if ui
                .selectable_label(self.selected_context.is_none(), "All")
                .clicked()
            {
                self.selected_context = None;
            }
            if ui
                .selectable_label(
                    self.selected_context == Some(ShortcutContext::Global),
                    "Global",
                )
                .clicked()
            {
                self.selected_context = Some(ShortcutContext::Global);
            }
            if ui
                .selectable_label(
                    self.selected_context == Some(ShortcutContext::ImageViewer),
                    "Image Viewer",
                )
                .clicked()
            {
                self.selected_context = Some(ShortcutContext::ImageViewer);
            }
        });

        ui.add_space(8.0);

        // Group shortcuts by context
        let actions_by_context = shortcuts.all_actions_by_context();

        // Display contexts in order
        let contexts_to_show = if let Some(ctx) = self.selected_context {
            vec![ctx]
        } else {
            vec![
                ShortcutContext::Global,
                ShortcutContext::ImageViewer,
                ShortcutContext::SignalPlotter,
            ]
        };

        for context in contexts_to_show {
            if let Some(actions) = actions_by_context.get(&context) {
                if !actions.is_empty() {
                    ui.group(|ui| {
                        ui.label(egui::RichText::new(context.label()).heading().strong());
                        ui.separator();

                        // Table of shortcuts
                        egui::Grid::new(format!("shortcuts_grid_{:?}", context))
                            .num_columns(2)
                            .spacing([20.0, 8.0])
                            .striped(true)
                            .show(ui, |ui| {
                                for action in actions {
                                    if let Some(binding) = shortcuts.get_binding(*action) {
                                        // Key binding (with monospace font)
                                        ui.label(
                                            egui::RichText::new(binding.label())
                                                .monospace()
                                                .strong(),
                                        );

                                        // Action description
                                        ui.label(action.description());

                                        ui.end_row();
                                    }
                                }
                            });
                    });

                    ui.add_space(12.0);
                }
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("💡 Tip:");
            ui.label("Press Shift+? again to close this window");
        });
    }
}
