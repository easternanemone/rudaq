//! Plan Runner panel for RunEngine control (bd-w14j.4)
//!
//! This panel provides a UI for:
//! - Queuing experiment plans (Count, LineScan, GridScan)
//! - Starting/pausing/resuming/aborting execution
//! - Monitoring engine status and queue length

use daq_client::DaqClient;
use eframe::egui;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// Result of an async action
enum ActionResult {
    QueuePlan {
        success: bool,
        error: Option<String>,
        run_uid: String,
        queue_position: u32,
    },
}

/// Pending action to execute
enum PendingAction {
    QueuePlan {
        plan_type: String,
        parameters: std::collections::HashMap<String, String>,
        device_mapping: std::collections::HashMap<String, String>,
        metadata: std::collections::HashMap<String, String>,
    },
}

/// Plan Runner panel state
pub struct PlanRunnerPanel {
    /// Selected plan type
    selected_plan_type: PlanType,

    /// Plan parameters (simple form)
    num_points: String,
    start_pos: String,
    end_pos: String,
    motor_name: String,
    detector_name: String,

    /// Engine state display
    engine_state: String,
    queue_length: usize,
    current_run_uid: String,

    /// Status message
    status: Option<String>,
    /// Error message
    error: Option<String>,

    /// Pending action
    pending_action: Option<PendingAction>,
    /// Async action result sender
    action_tx: mpsc::Sender<ActionResult>,
    /// Async action result receiver
    action_rx: mpsc::Receiver<ActionResult>,
    /// Number of in-flight async actions
    action_in_flight: usize,
}

#[derive(Default, PartialEq)]
enum PlanType {
    #[default]
    Count,
    LineScan,
    GridScan,
}

impl Default for PlanRunnerPanel {
    fn default() -> Self {
        let (action_tx, action_rx) = mpsc::channel(16);
        Self {
            selected_plan_type: PlanType::default(),
            num_points: "10".to_string(),
            start_pos: "0.0".to_string(),
            end_pos: "10.0".to_string(),
            motor_name: "motor".to_string(),
            detector_name: "detector".to_string(),
            engine_state: "Idle".to_string(),
            queue_length: 0,
            current_run_uid: String::new(),
            status: None,
            error: None,
            pending_action: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
        }
    }
}

impl PlanRunnerPanel {
    /// Poll for completed async operations
    fn poll_async_results(&mut self, ctx: &egui::Context) {
        let mut updated = false;
        loop {
            match self.action_rx.try_recv() {
                Ok(result) => {
                    self.action_in_flight = self.action_in_flight.saturating_sub(1);
                    match result {
                        ActionResult::QueuePlan {
                            success,
                            error,
                            run_uid,
                            queue_position,
                        } => {
                            if success {
                                self.status = Some(format!(
                                    "Plan queued: {} (Position: {})",
                                    run_uid, queue_position
                                ));
                                self.error = None;
                                self.queue_length += 1; // Basic local update
                            } else {
                                self.error = error;
                            }
                        }
                    }
                    updated = true;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        if self.action_in_flight > 0 || updated {
            ctx.request_repaint();
        }
    }

    /// Render the Plan Runner panel
    pub fn ui(&mut self, ui: &mut egui::Ui, client: Option<&mut DaqClient>, runtime: &Runtime) {
        self.poll_async_results(ui.ctx());

        // Clear pending action at start of frame
        self.pending_action = None;

        ui.heading("🎯 Plan Runner (RunEngine)");
        ui.separator();
        ui.add_space(8.0);

        // Show status/error
        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }
        if let Some(status) = &self.status {
            ui.colored_label(egui::Color32::GREEN, status);
        }

        // Status Display
        ui.group(|ui| {
            ui.heading("Engine Status");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("State:");
                ui.label(&self.engine_state);
            });

            ui.horizontal(|ui| {
                ui.label("Queue Length:");
                ui.label(self.queue_length.to_string());
            });

            if !self.current_run_uid.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Current Run:");
                    ui.monospace(&self.current_run_uid);
                });
            }
        });

        ui.add_space(12.0);

        // Plan Creation Form
        ui.group(|ui| {
            ui.heading("Queue New Plan");
            ui.add_space(4.0);

            // Plan type selector
            ui.horizontal(|ui| {
                ui.label("Plan Type:");
                ui.selectable_value(&mut self.selected_plan_type, PlanType::Count, "Count");
                ui.selectable_value(
                    &mut self.selected_plan_type,
                    PlanType::LineScan,
                    "Line Scan",
                );
                ui.selectable_value(
                    &mut self.selected_plan_type,
                    PlanType::GridScan,
                    "Grid Scan",
                );
            });

            ui.add_space(8.0);

            // Parameters based on plan type
            match self.selected_plan_type {
                PlanType::Count => {
                    ui.horizontal(|ui| {
                        ui.label("Number of Points:");
                        ui.text_edit_singleline(&mut self.num_points);
                    });

                    ui.horizontal(|ui| {
                        ui.label("Detector:");
                        ui.text_edit_singleline(&mut self.detector_name);
                    });
                }
                PlanType::LineScan => {
                    ui.horizontal(|ui| {
                        ui.label("Motor:");
                        ui.text_edit_singleline(&mut self.motor_name);
                    });

                    ui.horizontal(|ui| {
                        ui.label("Start:");
                        ui.text_edit_singleline(&mut self.start_pos);
                        ui.label("End:");
                        ui.text_edit_singleline(&mut self.end_pos);
                        ui.label("Points:");
                        ui.text_edit_singleline(&mut self.num_points);
                    });

                    ui.horizontal(|ui| {
                        ui.label("Detector:");
                        ui.text_edit_singleline(&mut self.detector_name);
                    });
                }
                PlanType::GridScan => {
                    ui.label("Grid scan parameters (TODO: implement 2D form)");
                }
            }

            ui.add_space(8.0);

            if ui.button("Queue Plan").clicked() {
                let mut parameters = std::collections::HashMap::new();
                let mut device_mapping = std::collections::HashMap::new();

                let plan_type_str = match self.selected_plan_type {
                    PlanType::Count => {
                        parameters.insert("num_points".to_string(), self.num_points.clone());
                        device_mapping.insert("detector".to_string(), self.detector_name.clone());
                        "count".to_string()
                    }
                    PlanType::LineScan => {
                        parameters.insert("start_position".to_string(), self.start_pos.clone());
                        parameters.insert("stop_position".to_string(), self.end_pos.clone());
                        parameters.insert("num_points".to_string(), self.num_points.clone());
                        device_mapping.insert("motor".to_string(), self.motor_name.clone());
                        device_mapping.insert("detector".to_string(), self.detector_name.clone());
                        "line_scan".to_string()
                    }
                    PlanType::GridScan => {
                        // TODO: Add grid scan parameters
                        "grid_scan".to_string()
                    }
                };

                self.pending_action = Some(PendingAction::QueuePlan {
                    plan_type: plan_type_str,
                    parameters,
                    device_mapping,
                    metadata: std::collections::HashMap::new(),
                });
            }
        });

        ui.add_space(12.0);

        // Control Buttons
        ui.group(|ui| {
            ui.heading("Engine Controls");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button("▶ Start").clicked() {
                    // TODO: Call RunEngineService.StartEngine
                    self.engine_state = "Running".to_string();
                }

                if ui.button("⏸ Pause").clicked() {
                    // TODO: Call RunEngineService.PauseEngine
                    self.engine_state = "Paused".to_string();
                }

                if ui.button("▶ Resume").clicked() {
                    // TODO: Call RunEngineService.ResumeEngine
                    self.engine_state = "Running".to_string();
                }

                if ui.button("⏹ Abort").clicked() {
                    // TODO: Call RunEngineService.AbortPlan
                    self.engine_state = "Idle".to_string();
                }
            });
        });

        ui.add_space(12.0);

        // Implementation Status
        ui.collapsing("Implementation Status (v0.6.0)", |ui| {
            ui.add_space(4.0);
            ui.label("✅ Panel structure created");
            ui.label("✅ UI controls laid out");
            ui.label("✅ Connected to RunEngineServiceClient");
            ui.label("✅ Implemented gRPC call for QueuePlan");
            ui.label("⏳ TODO: Implement start, pause, resume, abort");
            ui.label("⏳ TODO: Poll get_engine_status for status updates");
            ui.label("⏳ TODO: Validate plan parameters before queueing");
        });

        // Execute pending action
        if let Some(action) = self.pending_action.take() {
            self.execute_action(action, client, runtime);
        }
    }

    fn execute_action(
        &mut self,
        action: PendingAction,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    ) {
        match action {
            PendingAction::QueuePlan {
                plan_type,
                parameters,
                device_mapping,
                metadata,
            } => {
                let Some(client) = client else {
                    self.error = Some("Not connected to daemon".to_string());
                    return;
                };

                let mut client = client.clone();
                let tx = self.action_tx.clone();
                self.action_in_flight = self.action_in_flight.saturating_add(1);

                runtime.spawn(async move {
                    let result = client
                        .queue_plan(&plan_type, parameters, device_mapping, metadata)
                        .await;

                    let action_result = match result {
                        Ok(response) => ActionResult::QueuePlan {
                            success: response.success,
                            error: if response.success {
                                None
                            } else {
                                Some(response.error_message)
                            },
                            run_uid: response.run_uid,
                            queue_position: response.queue_position,
                        },
                        Err(e) => ActionResult::QueuePlan {
                            success: false,
                            error: Some(e.to_string()),
                            run_uid: String::new(),
                            queue_position: 0,
                        },
                    };
                    let _ = tx.send(action_result).await;
                });
            }
        }
    }
}
