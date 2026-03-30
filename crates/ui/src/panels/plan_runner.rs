//! Plan Runner panel for RunEngine control (bd-w14j.4)
//!
//! This panel provides a UI for:
//! - Queuing experiment plans (Count, LineScan, GridScan)
//! - Starting/pausing/resuming/aborting execution
//! - Monitoring engine status and queue length

use crate::runtime::Runtime;
use client::DaqClient;
use eframe::egui;
use tokio::sync::mpsc;

/// Result of an async action
enum ActionResult {
    QueuePlan {
        success: bool,
        error: Option<String>,
        run_uid: String,
        queue_position: u32,
    },
    StartEngine {
        success: bool,
        error: Option<String>,
    },
    PauseEngine {
        success: bool,
        paused_at: Option<String>,
    },
    ResumeEngine {
        success: bool,
        error: Option<String>,
    },
    AbortPlan {
        success: bool,
        error: Option<String>,
    },
    EngineStatus {
        state: String,
        queued_plans: u32,
        current_run_uid: Option<String>,
        current_plan_type: Option<String>,
        current_event: Option<u32>,
        total_events: Option<u32>,
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
    StartEngine,
    PauseEngine {
        defer: bool,
    },
    ResumeEngine,
    AbortPlan {
        run_uid: Option<String>,
    },
    PollStatus,
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

    /// Grid scan parameters
    grid_x_motor: String,
    grid_y_motor: String,
    grid_x_start: String,
    grid_x_end: String,
    grid_x_points: String,
    grid_y_start: String,
    grid_y_end: String,
    grid_y_points: String,
    grid_detector: String,

    /// Engine state display
    engine_state: String,
    queue_length: usize,
    current_run_uid: String,
    /// Current plan type (from status polling)
    current_plan_type: String,
    /// Current event / total events for progress display
    current_event: Option<u32>,
    total_events: Option<u32>,

    /// Status message
    status: Option<String>,
    /// Error message
    error: Option<String>,
    /// Validation errors for the current plan form
    validation_errors: Vec<String>,

    /// Pending action
    pending_action: Option<PendingAction>,
    /// Async action result sender
    action_tx: mpsc::Sender<ActionResult>,
    /// Async action result receiver
    action_rx: mpsc::Receiver<ActionResult>,
    /// Number of in-flight async actions
    action_in_flight: usize,

    /// Last time we polled engine status
    last_status_poll: Option<std::time::Instant>,
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
            grid_x_motor: "x_motor".to_string(),
            grid_y_motor: "y_motor".to_string(),
            grid_x_start: "0.0".to_string(),
            grid_x_end: "10.0".to_string(),
            grid_x_points: "10".to_string(),
            grid_y_start: "0.0".to_string(),
            grid_y_end: "10.0".to_string(),
            grid_y_points: "10".to_string(),
            grid_detector: "detector".to_string(),
            engine_state: "Idle".to_string(),
            queue_length: 0,
            current_run_uid: String::new(),
            current_plan_type: String::new(),
            current_event: None,
            total_events: None,
            status: None,
            error: None,
            validation_errors: Vec::new(),
            pending_action: None,
            action_tx,
            action_rx,
            action_in_flight: 0,
            last_status_poll: None,
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
                        ActionResult::StartEngine { success, error } => {
                            if success {
                                self.status = Some("Engine started".to_string());
                                self.error = None;
                                self.engine_state = "Running".to_string();
                            } else {
                                self.error = error;
                            }
                        }
                        ActionResult::PauseEngine { success, paused_at } => {
                            if success {
                                self.status = paused_at
                                    .map(|at| format!("Engine paused at: {}", at))
                                    .or_else(|| Some("Engine paused".to_string()));
                                self.error = None;
                                self.engine_state = "Paused".to_string();
                            } else {
                                self.error = Some("Failed to pause engine".to_string());
                            }
                        }
                        ActionResult::ResumeEngine { success, error } => {
                            if success {
                                self.status = Some("Engine resumed".to_string());
                                self.error = None;
                                self.engine_state = "Running".to_string();
                            } else {
                                self.error = error;
                            }
                        }
                        ActionResult::AbortPlan { success, error } => {
                            if success {
                                self.status = Some("Plan aborted".to_string());
                                self.error = None;
                                self.engine_state = "Idle".to_string();
                            } else {
                                self.error = error;
                            }
                        }
                        ActionResult::EngineStatus {
                            state,
                            queued_plans,
                            current_run_uid,
                            current_plan_type,
                            current_event,
                            total_events,
                        } => {
                            self.engine_state = state;
                            self.queue_length = queued_plans as usize;
                            if let Some(uid) = current_run_uid {
                                self.current_run_uid = uid;
                            } else {
                                self.current_run_uid.clear();
                            }
                            if let Some(pt) = current_plan_type {
                                self.current_plan_type = pt;
                            } else {
                                self.current_plan_type.clear();
                            }
                            self.current_event = current_event;
                            self.total_events = total_events;
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
                let state_color = match self.engine_state.as_str() {
                    "Running" => egui::Color32::GREEN,
                    "Paused" => egui::Color32::YELLOW,
                    "Aborting" | "Halted" => egui::Color32::RED,
                    _ => ui.visuals().text_color(),
                };
                ui.colored_label(state_color, &self.engine_state);
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

            if !self.current_plan_type.is_empty() {
                ui.horizontal(|ui| {
                    ui.label("Plan Type:");
                    ui.label(&self.current_plan_type);
                });
            }

            if let (Some(current), Some(total)) = (self.current_event, self.total_events) {
                ui.horizontal(|ui| {
                    ui.label("Progress:");
                    let progress = if total > 0 {
                        #[allow(clippy::cast_precision_loss)]
                        // SAFETY: precision loss acceptable for progress bar display
                        {
                            current as f32 / total as f32
                        }
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .text(format!("{}/{}", current, total))
                            .animate(self.engine_state == "Running"),
                    );
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
                    ui.label(egui::RichText::new("X Axis (fast)").strong());
                    ui.horizontal(|ui| {
                        ui.label("Motor:");
                        ui.text_edit_singleline(&mut self.grid_x_motor);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Start:");
                        ui.text_edit_singleline(&mut self.grid_x_start);
                        ui.label("End:");
                        ui.text_edit_singleline(&mut self.grid_x_end);
                        ui.label("Points:");
                        ui.text_edit_singleline(&mut self.grid_x_points);
                    });

                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Y Axis (slow)").strong());
                    ui.horizontal(|ui| {
                        ui.label("Motor:");
                        ui.text_edit_singleline(&mut self.grid_y_motor);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Start:");
                        ui.text_edit_singleline(&mut self.grid_y_start);
                        ui.label("End:");
                        ui.text_edit_singleline(&mut self.grid_y_end);
                        ui.label("Points:");
                        ui.text_edit_singleline(&mut self.grid_y_points);
                    });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Detector:");
                        ui.text_edit_singleline(&mut self.grid_detector);
                    });
                }
            }

            ui.add_space(8.0);

            // Validate parameters before showing the Queue button
            self.validation_errors = self.validate_plan_parameters();

            // Show validation errors
            for err in &self.validation_errors {
                ui.colored_label(egui::Color32::from_rgb(255, 140, 0), err);
            }

            let can_queue = self.validation_errors.is_empty();
            ui.add_enabled_ui(can_queue, |ui| {
                if ui.button("Queue Plan").clicked() {
                    let mut parameters = std::collections::HashMap::new();
                    let mut device_mapping = std::collections::HashMap::new();

                    let plan_type_str = match self.selected_plan_type {
                        PlanType::Count => {
                            parameters
                                .insert("num_points".to_string(), self.num_points.clone());
                            device_mapping
                                .insert("detector".to_string(), self.detector_name.clone());
                            "count".to_string()
                        }
                        PlanType::LineScan => {
                            parameters
                                .insert("start_position".to_string(), self.start_pos.clone());
                            parameters
                                .insert("stop_position".to_string(), self.end_pos.clone());
                            parameters
                                .insert("num_points".to_string(), self.num_points.clone());
                            device_mapping
                                .insert("motor".to_string(), self.motor_name.clone());
                            device_mapping
                                .insert("detector".to_string(), self.detector_name.clone());
                            "line_scan".to_string()
                        }
                        PlanType::GridScan => {
                            parameters
                                .insert("x_start".to_string(), self.grid_x_start.clone());
                            parameters
                                .insert("x_end".to_string(), self.grid_x_end.clone());
                            parameters
                                .insert("x_points".to_string(), self.grid_x_points.clone());
                            parameters
                                .insert("y_start".to_string(), self.grid_y_start.clone());
                            parameters
                                .insert("y_end".to_string(), self.grid_y_end.clone());
                            parameters
                                .insert("y_points".to_string(), self.grid_y_points.clone());
                            device_mapping
                                .insert("x_motor".to_string(), self.grid_x_motor.clone());
                            device_mapping
                                .insert("y_motor".to_string(), self.grid_y_motor.clone());
                            device_mapping
                                .insert("detector".to_string(), self.grid_detector.clone());
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
        });

        ui.add_space(12.0);

        // Control Buttons
        ui.group(|ui| {
            ui.heading("Engine Controls");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button("▶ Start").clicked() {
                    self.pending_action = Some(PendingAction::StartEngine);
                }

                if ui.button("⏸ Pause").clicked() {
                    self.pending_action = Some(PendingAction::PauseEngine { defer: false });
                }

                if ui.button("▶ Resume").clicked() {
                    self.pending_action = Some(PendingAction::ResumeEngine);
                }

                if ui.button("⏹ Abort").clicked() {
                    self.pending_action = Some(PendingAction::AbortPlan { run_uid: None });
                }
            });
        });

        ui.add_space(12.0);

        // Poll engine status every 2 seconds when connected
        if client.is_some() {
            let should_poll = match self.last_status_poll {
                Some(last) => last.elapsed() > std::time::Duration::from_secs(2),
                None => true,
            };
            if should_poll && self.pending_action.is_none() {
                self.pending_action = Some(PendingAction::PollStatus);
                self.last_status_poll = Some(std::time::Instant::now());
            }
        }

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
        let Some(client) = client else {
            self.error = Some("Not connected to daemon".to_string());
            return;
        };

        let mut client = client.clone();
        let tx = self.action_tx.clone();
        self.action_in_flight = self.action_in_flight.saturating_add(1);

        match action {
            PendingAction::QueuePlan {
                plan_type,
                parameters,
                device_mapping,
                metadata,
            } => {
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
            PendingAction::StartEngine => {
                runtime.spawn(async move {
                    let result = client.start_engine().await;

                    let action_result = match result {
                        Ok(response) => ActionResult::StartEngine {
                            success: response.success,
                            error: if response.success {
                                None
                            } else {
                                Some(response.error_message)
                            },
                        },
                        Err(e) => ActionResult::StartEngine {
                            success: false,
                            error: Some(e.to_string()),
                        },
                    };
                    let _ = tx.send(action_result).await;
                });
            }
            PendingAction::PauseEngine { defer } => {
                runtime.spawn(async move {
                    let result = client.pause_engine(defer).await;

                    let action_result = match result {
                        Ok(response) => ActionResult::PauseEngine {
                            success: response.success,
                            paused_at: if response.success && !response.paused_at.is_empty() {
                                Some(response.paused_at)
                            } else {
                                None
                            },
                        },
                        Err(e) => ActionResult::PauseEngine {
                            success: false,
                            paused_at: Some(format!("Error: {}", e)),
                        },
                    };
                    let _ = tx.send(action_result).await;
                });
            }
            PendingAction::ResumeEngine => {
                runtime.spawn(async move {
                    let result = client.resume_engine().await;

                    let action_result = match result {
                        Ok(response) => ActionResult::ResumeEngine {
                            success: response.success,
                            error: if response.success {
                                None
                            } else {
                                Some(response.error_message)
                            },
                        },
                        Err(e) => ActionResult::ResumeEngine {
                            success: false,
                            error: Some(e.to_string()),
                        },
                    };
                    let _ = tx.send(action_result).await;
                });
            }
            PendingAction::AbortPlan { run_uid } => {
                runtime.spawn(async move {
                    let result = client.abort_plan(run_uid.as_deref()).await;

                    let action_result = match result {
                        Ok(response) => ActionResult::AbortPlan {
                            success: response.success,
                            error: if response.success {
                                None
                            } else {
                                Some(response.error_message)
                            },
                        },
                        Err(e) => ActionResult::AbortPlan {
                            success: false,
                            error: Some(e.to_string()),
                        },
                    };
                    let _ = tx.send(action_result).await;
                });
            }
        }
    }
}
