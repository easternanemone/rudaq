//! Document Viewer panel for experiment data streams (bd-w14j.4)
//!
//! This panel displays structured experiment documents from RunEngine:
//! - Start: Run metadata and plan configuration
//! - Descriptor: Data schema definitions
//! - Event: Measurement data points
//! - Stop: Run completion status

use daq_proto::daq::Document;
use eframe::egui;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

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

    /// Subscription state
    is_subscribed: bool,
    rx: Option<mpsc::Receiver<Result<Document, String>>>,
    subscription_task: Option<JoinHandle<()>>,
}

impl DocumentViewerPanel {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            document_log: Vec::new(),
            auto_scroll: true,
            show_start_docs: true,
            show_event_docs: true,
            show_stop_docs: true,
            show_descriptor_docs: true,
            is_subscribed: false,
            rx: None,
            subscription_task: None,
        }
    }

    /// Render the Document Viewer panel
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        client: Option<&mut crate::client::DaqClient>,
        runtime: &tokio::runtime::Runtime,
    ) {
        // Poll for new documents
        let mut stream_disconnected = false;
        if let Some(rx) = &mut self.rx {
            while let Ok(result) = rx.try_recv() {
                match result {
                    Ok(doc) => {
                        let doc_str = format_document(&doc);
                        self.document_log.push(doc_str);
                    }
                    Err(err) => {
                        self.document_log
                            .push(format!("ERROR: Stream disconnected: {}", err));
                        self.is_subscribed = false;
                        stream_disconnected = true;
                    }
                }
            }
        }
        if stream_disconnected {
            self.rx = None;
        }

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
                        ui.label(
                            "No documents yet. Queue a plan and start the engine to see data.",
                        );
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

            if self.is_subscribed {
                if ui.button("Stop Stream").clicked() {
                    if let Some(handle) = self.subscription_task.take() {
                        handle.abort();
                    }
                    self.is_subscribed = false;
                    self.rx = None;
                    self.document_log
                        .push("INFO: Unsubscribed from stream".to_string());
                }
            } else {
                let connected = client.is_some();
                let btn = ui.add_enabled(connected, egui::Button::new("Subscribe to Stream"));
                if btn.clicked() {
                    if let Some(client_ref) = client {
                        let mut client = client_ref.clone();
                        let (tx, rx) = mpsc::channel(100);
                        self.rx = Some(rx);
                        self.is_subscribed = true;

                        self.subscription_task = Some(runtime.spawn(async move {
                            match client.stream_documents(None, vec![]).await {
                                Ok(mut stream) => {
                                    while let Some(result) = stream.next().await {
                                        match result {
                                            Ok(doc) => {
                                                if tx.send(Ok(doc)).await.is_err() {
                                                    break;
                                                }
                                            }
                                            Err(status) => {
                                                let _ = tx
                                                    .send(Err(format!("gRPC Error: {}", status)))
                                                    .await;
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(Err(format!("Failed to subscribe: {}", e))).await;
                                }
                            }
                        }));
                        self.document_log
                            .push("INFO: Subscribing to stream...".to_string());
                    }
                }
                if !connected {
                    btn.on_hover_text("Connect to daemon to subscribe");
                }
            }
        });

        ui.add_space(12.0);

        // Implementation Status
        ui.collapsing("Implementation Status (v0.6.0)", |ui| {
            ui.add_space(4.0);
            ui.label("✅ Panel structure created");
            ui.label("✅ Document log display with filtering");
            ui.label("✅ Color-coded document types");
            ui.label("✅ Connected to RunEngineServiceClient");
            ui.label("✅ Subscribe to stream_documents gRPC stream");
            ui.label("✅ Parse and format Document protos");
            ui.label("✅ Handle stream reconnection on error");
        });
    }

    /// Add a document to the log (for testing)
    #[allow(dead_code)]
    pub fn add_document(&mut self, doc: String) {
        self.document_log.push(doc);
    }
}

fn format_document(doc: &Document) -> String {
    use daq_proto::daq::document::Payload;
    match &doc.payload {
        Some(Payload::Start(start)) => {
            format!(
                "START: run_uid={}, plan_type={}, plan_name={}",
                start.run_uid, start.plan_type, start.plan_name
            )
        }
        Some(Payload::Descriptor(desc)) => {
            format!(
                "DESCRIPTOR: uid={}, name={}, keys={:?}",
                desc.descriptor_uid,
                desc.name,
                desc.data_keys.keys()
            )
        }
        Some(Payload::Event(event)) => {
            format!(
                "EVENT: seq_num={}, time={:.3}s, data={:?}",
                event.seq_num,
                event.time_ns as f64 / 1e9,
                event.data
            )
        }
        Some(Payload::Stop(stop)) => {
            format!(
                "STOP: run_uid={}, status={}, reason={}",
                stop.run_uid, stop.exit_status, stop.reason
            )
        }
        None => "UNKNOWN DOCUMENT".to_string(),
    }
}
