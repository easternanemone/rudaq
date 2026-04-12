//! Quick Command Palette (Ctrl+K / Cmd+K) — modal overlay for navigating panels.
//!
//! Phase 1 MVP: navigation commands only (switching between panels).
//! Future: extend with device commands, script execution, etc.

use crate::app::Panel;
use crate::icons;

/// A single command that can be executed from the palette.
pub struct PaletteCommand {
    /// Display label shown in the palette list.
    pub label: String,
    /// Category shown as a muted prefix (e.g., "Navigation", "View").
    pub category: &'static str,
    /// What happens when the user selects this command.
    pub action: PaletteAction,
}

/// The action to perform when a command is selected.
pub enum PaletteAction {
    /// Switch focus to a specific panel tab.
    FocusPanel(Panel),
    // Future variants:
    // DeviceCommand { device_id: String, command: String },
    // ScriptRun { script_name: String },
}

/// Persistent state for the command palette overlay.
pub struct CommandPaletteState {
    /// Whether the palette is currently visible.
    pub open: bool,
    /// Current search query text.
    query: String,
    /// Index of the highlighted command in the filtered list.
    selected: usize,
    /// Static list of all available commands (built once at startup).
    commands: Vec<PaletteCommand>,
}

impl Default for CommandPaletteState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            selected: 0,
            commands: build_navigation_commands(),
        }
    }
}

impl CommandPaletteState {
    /// Toggle the palette open/closed. Resets query and selection on open.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.selected = 0;
        }
    }

    /// Close the palette.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Show the command palette overlay. Returns `Some(PaletteAction)` if the
    /// user selected a command this frame, `None` otherwise.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<PaletteAction> {
        if !self.open {
            return None;
        }

        let mut result: Option<usize> = None;

        // Semi-transparent backdrop -- clicking it closes the palette.
        #[allow(deprecated)] // screen_rect() deprecated in egui 0.34; content_rect() not yet stable
        let screen = ctx.screen_rect();
        let backdrop_response = egui::Area::new(egui::Id::new("command_palette_backdrop"))
            .fixed_pos(egui::Pos2::ZERO)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(128));
                response
            });

        if backdrop_response.inner.clicked() {
            self.close();
            return None;
        }

        // Compute filtered commands before showing the UI.
        let query_lower = self.query.to_lowercase();
        let filtered_indices: Vec<usize> = self
            .commands
            .iter()
            .enumerate()
            .filter(|(_, cmd)| {
                query_lower.is_empty()
                    || cmd.label.to_lowercase().contains(&query_lower)
                    || cmd.category.to_lowercase().contains(&query_lower)
            })
            .map(|(i, _)| i)
            .collect();

        // Clamp selection to filtered range.
        if self.selected >= filtered_indices.len() {
            self.selected = filtered_indices.len().saturating_sub(1);
        }

        // Palette dimensions.
        let palette_width = (screen.width() * 0.5).clamp(300.0, 600.0);
        let palette_x = (screen.width() - palette_width) / 2.0;
        let palette_y = screen.height() * 0.2;

        egui::Area::new(egui::Id::new("command_palette"))
            .fixed_pos(egui::pos2(palette_x, palette_y))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(12))
                    .corner_radius(egui::CornerRadius::same(8))
                    .show(ui, |ui| {
                        ui.set_width(palette_width);

                        // --- Search input ---
                        let text_edit = egui::TextEdit::singleline(&mut self.query)
                            .hint_text(format!("{} Type a command...", icons::MAGNIFYING_GLASS))
                            .desired_width(palette_width - 24.0);
                        let response = ui.add(text_edit);

                        // Auto-focus the text input on the first frame.
                        if !response.has_focus() {
                            response.request_focus();
                        }

                        // --- Keyboard navigation ---
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        let arrow_up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
                        let arrow_down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));

                        if escape {
                            self.close();
                            return;
                        }

                        if arrow_up && self.selected > 0 {
                            self.selected -= 1;
                        }
                        if arrow_down && self.selected + 1 < filtered_indices.len() {
                            self.selected += 1;
                        }

                        if enter && !filtered_indices.is_empty() {
                            result = Some(self.selected);
                        }

                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(4.0);

                        // --- Command list ---
                        if filtered_indices.is_empty() {
                            ui.colored_label(egui::Color32::GRAY, "No matching commands");
                        } else {
                            egui::ScrollArea::vertical()
                                .max_height(300.0)
                                .show(ui, |ui| {
                                    for (display_idx, &cmd_idx) in
                                        filtered_indices.iter().enumerate()
                                    {
                                        let cmd = &self.commands[cmd_idx];
                                        let is_selected = display_idx == self.selected;

                                        let response = ui.horizontal(|ui| {
                                            // Highlight background for selected item.
                                            if is_selected {
                                                let rect = ui.available_rect_before_wrap();
                                                ui.painter().rect_filled(
                                                    rect,
                                                    4.0,
                                                    ui.visuals()
                                                        .selection
                                                        .bg_fill
                                                        .linear_multiply(0.3),
                                                );
                                            }

                                            // Category badge.
                                            ui.label(
                                                egui::RichText::new(cmd.category)
                                                    .small()
                                                    .color(egui::Color32::GRAY),
                                            );

                                            // Command label.
                                            let label_text = if is_selected {
                                                egui::RichText::new(&cmd.label).strong()
                                            } else {
                                                egui::RichText::new(&cmd.label)
                                            };
                                            ui.label(label_text);
                                        });

                                        // Click to select.
                                        if response
                                            .response
                                            .interact(egui::Sense::click())
                                            .clicked()
                                        {
                                            result = Some(display_idx);
                                        }

                                        // Hover to highlight.
                                        if response.response.hovered() {
                                            self.selected = display_idx;
                                        }
                                    }
                                });
                        }

                        // --- Hint bar ---
                        ui.add_space(4.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.0;
                            ui.label(
                                egui::RichText::new("Enter to select")
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                            ui.label(
                                egui::RichText::new("Esc to close")
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                            ui.label(
                                egui::RichText::new("\u{2191}\u{2193} to navigate")
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                        });
                    });
            });

        // Extract the action if the user selected a command.
        if let Some(display_idx) = result
            && display_idx < filtered_indices.len()
        {
            let cmd_idx = filtered_indices[display_idx];
            self.close();
            // Extract the action by rebuilding (commands are cheap value types).
            return Some(take_action(&self.commands[cmd_idx]));
        }

        None
    }
}

/// Reconstruct the action from a command reference. Panel variants are Copy so
/// this is trivially cheap.
fn take_action(cmd: &PaletteCommand) -> PaletteAction {
    match &cmd.action {
        PaletteAction::FocusPanel(panel) => PaletteAction::FocusPanel(*panel),
    }
}

/// Build the static list of navigation commands (one per non-device Panel variant).
fn build_navigation_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "Getting Started".into(),
            category: "Navigation",
            action: PaletteAction::FocusPanel(Panel::GettingStarted),
        },
        PaletteCommand {
            label: "Instruments".into(),
            category: "Navigation",
            action: PaletteAction::FocusPanel(Panel::Instruments),
        },
        PaletteCommand {
            label: "Image Viewer".into(),
            category: "Visualization",
            action: PaletteAction::FocusPanel(Panel::ImageViewer),
        },
        PaletteCommand {
            label: "Signal Plotter".into(),
            category: "Visualization",
            action: PaletteAction::FocusPanel(Panel::SignalPlotter),
        },
        PaletteCommand {
            label: "Scripts".into(),
            category: "Experiment",
            action: PaletteAction::FocusPanel(Panel::Scripts),
        },
        PaletteCommand {
            label: "Scan Builder".into(),
            category: "Experiment",
            action: PaletteAction::FocusPanel(Panel::ScanBuilder),
        },
        PaletteCommand {
            label: "Experiment Designer".into(),
            category: "Experiment",
            action: PaletteAction::FocusPanel(Panel::ExperimentDesigner),
        },
        PaletteCommand {
            label: "Plan Runner".into(),
            category: "Experiment",
            action: PaletteAction::FocusPanel(Panel::PlanRunner),
        },
        PaletteCommand {
            label: "Storage".into(),
            category: "Data",
            action: PaletteAction::FocusPanel(Panel::Storage),
        },
        PaletteCommand {
            label: "Run History".into(),
            category: "Data",
            action: PaletteAction::FocusPanel(Panel::RunHistory),
        },
        PaletteCommand {
            label: "Documents".into(),
            category: "Data",
            action: PaletteAction::FocusPanel(Panel::DocumentViewer),
        },
        PaletteCommand {
            label: "Modules".into(),
            category: "System",
            action: PaletteAction::FocusPanel(Panel::Modules),
        },
        PaletteCommand {
            label: "Logs".into(),
            category: "System",
            action: PaletteAction::FocusPanel(Panel::Logs),
        },
    ]
}
