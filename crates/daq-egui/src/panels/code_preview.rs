//! Code preview panel for displaying generated Rhai scripts.

use crate::graph::{graph_to_rhai_script, ExperimentNode};
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};
use egui_snarl::Snarl;

/// Panel for displaying generated Rhai code from the experiment graph.
pub struct CodePreviewPanel {
    /// Generated Rhai code (cached)
    code: String,
    /// Whether the panel is visible
    visible: bool,
    /// Hash of graph for change detection
    last_graph_version: u64,
    /// Color theme for code display
    theme: ColorTheme,
}

impl Default for CodePreviewPanel {
    fn default() -> Self {
        Self {
            code: String::new(),
            visible: false,
            last_graph_version: 0,
            theme: ColorTheme::GRUVBOX_DARK,
        }
    }
}

impl CodePreviewPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle panel visibility
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Check if panel is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Update generated code from graph if changed
    pub fn update(&mut self, graph: &Snarl<ExperimentNode>, graph_version: u64) {
        if !self.visible {
            return; // Don't regenerate when hidden
        }

        if graph_version != self.last_graph_version {
            self.code = graph_to_rhai_script(graph, None);
            self.last_graph_version = graph_version;
        }
    }

    /// Render the code preview panel as a right side panel
    ///
    /// DEPRECATED: Use ui_inside() for proper integration with tabs/docks.
    /// This method renders at the window level and may be cut off in tabbed layouts.
    #[allow(dead_code)]
    #[deprecated(
        since = "0.1.0",
        note = "Use ui_inside() for proper tab/dock integration"
    )]
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.visible {
            return;
        }

        egui::SidePanel::right("code_preview_panel")
            .resizable(true)
            .default_width(400.0)
            .min_width(250.0)
            .show(ctx, |ui| {
                ui.heading("Generated Rhai Code");

                ui.horizontal(|ui| {
                    if ui
                        .button("Copy")
                        .on_hover_text("Copy to clipboard")
                        .clicked()
                    {
                        ui.ctx().copy_text(self.code.clone());
                    }

                    // Theme selector
                    egui::ComboBox::from_label("Theme")
                        .selected_text(self.theme_name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.theme,
                                ColorTheme::GRUVBOX_DARK,
                                "Gruvbox Dark",
                            );
                            ui.selectable_value(
                                &mut self.theme,
                                ColorTheme::GRUVBOX_LIGHT,
                                "Gruvbox Light",
                            );
                            ui.selectable_value(
                                &mut self.theme,
                                ColorTheme::GITHUB_DARK,
                                "GitHub Dark",
                            );
                        });
                });

                ui.separator();

                // Scrollable code area - fill remaining vertical space
                let available_height = ui.available_height();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(available_height)
                    .show(ui, |ui| {
                        // Use egui_code_editor for syntax highlighting
                        // Note: CodeEditor requires mutable string but we ignore changes (read-only)
                        let mut code_copy = self.code.clone();
                        // Calculate rows based on available height (approximate 14px per line)
                        let rows = ((available_height / 14.0) as usize).max(10);
                        CodeEditor::default()
                            .id_source("rhai_preview")
                            .with_rows(rows)
                            .with_fontsize(12.0)
                            .with_theme(self.theme)
                            .with_syntax(Syntax::rust()) // Rhai similar to Rust
                            .with_numlines(true)
                            .show(ui, &mut code_copy);
                        // Discard any edits (read-only preview)
                    });
            });
    }

    /// Render the code preview panel inside a given UI context (for tab/dock integration)
    pub fn ui_inside(&mut self, ui: &mut egui::Ui) {
        if !self.visible {
            return;
        }

        egui::SidePanel::right("code_preview_panel")
            .resizable(true)
            .default_width(400.0)
            .min_width(250.0)
            .show_inside(ui, |ui| {
                ui.heading("Generated Rhai Code");

                ui.horizontal(|ui| {
                    if ui
                        .button("Copy")
                        .on_hover_text("Copy to clipboard")
                        .clicked()
                    {
                        ui.ctx().copy_text(self.code.clone());
                    }

                    // Theme selector
                    egui::ComboBox::from_label("Theme")
                        .selected_text(self.theme_name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.theme,
                                ColorTheme::GRUVBOX_DARK,
                                "Gruvbox Dark",
                            );
                            ui.selectable_value(
                                &mut self.theme,
                                ColorTheme::GRUVBOX_LIGHT,
                                "Gruvbox Light",
                            );
                            ui.selectable_value(
                                &mut self.theme,
                                ColorTheme::GITHUB_DARK,
                                "GitHub Dark",
                            );
                        });
                });

                ui.separator();

                // Scrollable code area - fill remaining vertical space
                let available_height = ui.available_height();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(available_height)
                    .show(ui, |ui| {
                        // Use egui_code_editor for syntax highlighting
                        // Note: CodeEditor requires mutable string but we ignore changes (read-only)
                        let mut code_copy = self.code.clone();
                        // Calculate rows based on available height (approximate 14px per line)
                        let rows = ((available_height / 14.0) as usize).max(10);
                        CodeEditor::default()
                            .id_source("rhai_preview")
                            .with_rows(rows)
                            .with_fontsize(12.0)
                            .with_theme(self.theme)
                            .with_syntax(Syntax::rust()) // Rhai similar to Rust
                            .with_numlines(true)
                            .show(ui, &mut code_copy);
                        // Discard any edits (read-only preview)
                    });
            });
    }

    fn theme_name(&self) -> &'static str {
        self.theme.name
    }

    /// Get the current generated code (for export)
    #[allow(dead_code)]
    pub fn code(&self) -> &str {
        &self.code
    }
}
