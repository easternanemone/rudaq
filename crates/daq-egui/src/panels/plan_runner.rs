//! Plan Runner panel for RunEngine control (bd-w14j.4)
//!
//! This panel provides a UI for:
//! - Queuing experiment plans (Count, LineScan, GridScan)
//! - Starting/pausing/resuming/aborting execution
//! - Monitoring engine status and queue length

use eframe::egui;

/// Plan Runner panel state
#[derive(Default)]
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
}

#[derive(Default, PartialEq)]
enum PlanType {
    #[default]
    Count,
    LineScan,
    GridScan,
}

impl PlanRunnerPanel {
    /// Render the Plan Runner panel
    pub fn ui(&mut self, ui: &mut egui::Ui, _client: Option<&mut crate::client::DaqClient>) {
        ui.heading("🎯 Plan Runner (RunEngine)");
        ui.separator();
        ui.add_space(8.0);

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
                ui.selectable_value(&mut self.selected_plan_type, PlanType::LineScan, "Line Scan");
                ui.selectable_value(&mut self.selected_plan_type, PlanType::GridScan, "Grid Scan");
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
                // TODO: Call RunEngineService.QueuePlan via gRPC
                self.queue_length += 1;
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
            ui.label("⏳ TODO: Connect to RunEngineServiceClient");
            ui.label("⏳ TODO: Implement gRPC calls (queue_plan, start, pause, resume, abort)");
            ui.label("⏳ TODO: Poll get_engine_status for status updates");
            ui.label("⏳ TODO: Validate plan parameters before queueing");
        });
    }
}
