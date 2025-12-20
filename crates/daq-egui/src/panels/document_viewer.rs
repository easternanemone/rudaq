//! Document Viewer panel for experiment data streams (bd-w14j.4)
//!
//! This panel displays structured experiment documents from RunEngine:
//! - Start: Run metadata and plan configuration
//! - Descriptor: Data schema definitions
//! - Event: Measurement data points
//! - Stop: Run completion status

use eframe::egui;

/// Document Viewer panel state
#[derive(Default)]
pub struct DocumentViewerPanel {
    /// Document log (text display)
    document_log: Vec<String>,

    /// Auto-scroll to bottom
    auto_scroll: bool,

    /// Filter settings
    show_start_docs: bool,
    show_event_docs: bool,
    show_stop_docs: bool,
    show_descriptor_docs: bool,
}

impl DocumentViewerPanel {
    pub fn new() -> Self {
        Self {
            document_log: Vec::new(),
            auto_scroll: true,
            show_start_docs: true,
            show_event_docs: true,
            show_stop_docs: true,
            show_descriptor_docs: true,
        }
    }

    /// Render the Document Viewer panel
    pub fn ui(&mut self, ui: &mut egui::Ui, _client: Option<&mut crate::client::DaqClient>) {
        ui.heading("📊 Document Viewer (Experiment Data Stream)");
        ui.separator();
        ui.add_space(8.0);

        // Filter Controls
        ui.horizontal(|ui| {
            ui.label("Show:");
            ui.checkbox(&mut self.show_start_docs, "Start");
            ui.checkbox(&mut self.show_event_docs, "Event");
            ui.checkbox(&mut self.show_stop_docs, "Stop");
            ui.checkbox(&mut self.show_descriptor_docs, "Descriptor");
            ui.separator();
            ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
        });

        ui.add_space(8.0);

        // Document Stream Display
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(self.auto_scroll)
            .show(ui, |ui| {
                ui.group(|ui| {
                    if self.document_log.is_empty() {
                        ui.label("No documents yet. Queue a plan and start the engine to see data.");
                    } else {
                        for doc in &self.document_log {
                            ui.horizontal(|ui| {
                                // Color-code by document type
                                if doc.starts_with("START") {
                                    if self.show_start_docs {
                                        ui.colored_label(egui::Color32::GREEN, "●");
                                        ui.monospace(doc);
                                    }
                                } else if doc.starts_with("EVENT") {
                                    if self.show_event_docs {
                                        ui.colored_label(egui::Color32::LIGHT_BLUE, "●");
                                        ui.monospace(doc);
                                    }
                                } else if doc.starts_with("STOP") {
                                    if self.show_stop_docs {
                                        ui.colored_label(egui::Color32::RED, "●");
                                        ui.monospace(doc);
                                    }
                                } else if doc.starts_with("DESCRIPTOR") {
                                    if self.show_descriptor_docs {
                                        ui.colored_label(egui::Color32::YELLOW, "●");
                                        ui.monospace(doc);
                                    }
                                } else {
                                    ui.monospace(doc);
                                }
                            });
                        }
                    }
                });
            });

        ui.add_space(12.0);

        // Control buttons
        ui.horizontal(|ui| {
            if ui.button("Clear Log").clicked() {
                self.document_log.clear();
            }

            if ui.button("Subscribe to Stream").clicked() {
                // TODO: Call RunEngineService.StreamDocuments
                self.document_log.push("START: run_uid=abc123, plan_type=count, num_points=5".to_string());
            }
        });

        ui.add_space(12.0);

        // Implementation Status
        ui.collapsing("Implementation Status (v0.6.0)", |ui| {
            ui.add_space(4.0);
            ui.label("✅ Panel structure created");
            ui.label("✅ Document log display with filtering");
            ui.label("✅ Color-coded document types");
            ui.label("⏳ TODO: Connect to RunEngineServiceClient");
            ui.label("⏳ TODO: Subscribe to stream_documents gRPC stream");
            ui.label("⏳ TODO: Parse and format Document protos");
            ui.label("⏳ TODO: Handle stream reconnection on error");
        });
    }

    /// Add a document to the log (for testing)
    #[allow(dead_code)]
    pub fn add_document(&mut self, doc: String) {
        self.document_log.push(doc);
    }
}
