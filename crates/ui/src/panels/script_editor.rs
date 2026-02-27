//! Script editor panel for editing Rhai scripts directly.
//!
//! This panel is used after "ejecting" from the visual graph editor.
//! Changes to the script do NOT sync back to the graph (one-way export).

use egui_code_editor::{CodeEditor, ColorTheme, Syntax};
#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;

/// Panel for editing Rhai scripts directly (ejected from visual mode).
pub struct ScriptEditorPanel {
    /// Script content
    code: String,
    /// Current file path (if saved)
    file_path: Option<std::path::PathBuf>,
    /// Color theme
    theme: ColorTheme,
    /// Whether content has unsaved changes
    dirty: bool,
    /// Status message
    status: Option<String>,
}

impl ScriptEditorPanel {
    /// Create new panel with initial code from graph
    pub fn from_graph_code(code: String, source_graph: Option<std::path::PathBuf>) -> Self {
        Self {
            code,
            file_path: None, // New script, not saved yet
            theme: ColorTheme::GRUVBOX_DARK,
            dirty: true, // Start as dirty since it's unsaved
            status: source_graph.map(|p| format!("Ejected from {}", p.display())),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // Toolbar
        ui.horizontal(|ui| {
            ui.heading("Script Editor");

            ui.separator();

            if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                self.save();
            }

            if ui.button("Save As...").clicked() {
                self.save_as();
            }

            if ui
                .button("Run")
                .on_hover_text("Execute this script")
                .clicked()
            {
                // TODO: Connect to scripting engine execution
                self.status = Some("Run not yet implemented".to_string());
            }

            ui.separator();

            // Theme selector
            egui::ComboBox::from_label("Theme")
                .selected_text(self.theme_name())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.theme, ColorTheme::GRUVBOX_DARK, "Gruvbox Dark");
                    ui.selectable_value(&mut self.theme, ColorTheme::GRUVBOX, "Gruvbox Light");
                    ui.selectable_value(&mut self.theme, ColorTheme::AYU_DARK, "Ayu Dark");
                });

            ui.separator();

            // File info
            if let Some(path) = &self.file_path {
                let dirty_marker = if self.dirty { "*" } else { "" };
                ui.label(format!(
                    "{}{}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    dirty_marker
                ));
            } else {
                ui.label("Unsaved*");
            }
        });

        // Status message
        if let Some(status) = &self.status {
            ui.label(status);
        }

        ui.separator();

        // Code editor (editable)
        let response = CodeEditor::default()
            .id_source("script_editor")
            .with_rows(40)
            .with_fontsize(13.0)
            .with_theme(self.theme)
            .with_syntax(Syntax::rust())
            .with_numlines(true)
            .show(ui, &mut self.code);

        // Track dirty state
        if response.response.changed() {
            self.dirty = true;
        }
    }

    fn save(&mut self) {
        if let Some(path) = &self.file_path {
            match std::fs::write(path, &self.code) {
                Ok(()) => {
                    self.dirty = false;
                    self.status = Some(format!("Saved to {}", path.display()));
                }
                Err(e) => {
                    self.status = Some(format!("Save failed: {}", e));
                }
            }
        } else {
            self.save_as();
        }
    }

    fn save_as(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = FileDialog::new()
            .add_filter("Rhai Script", &["rhai"])
            .set_file_name("script.rhai")
            .save_file()
        {
            self.file_path = Some(path.clone());
            self.save();
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.status = Some("File dialogs are not available in the browser".to_string());
        }
    }

    fn theme_name(&self) -> &'static str {
        self.theme.name
    }
}
