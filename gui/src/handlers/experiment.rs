//! Experiment handlers
//!
//! Handles RunEngine callbacks: plan browsing, queue management, engine control.
//! Part of GUI Phase 4 (bd-niy4)

use super::common::spawn_rpc;
use crate::state::SharedState;
use crate::ui::{
    EngineStatusInfo, MainWindow, PlanTypeInfo, SharedString, UiAdapter, VecModel, Weak,
};
use rust_daq::grpc::{EngineState, PlanTypeSummary};
use std::rc::Rc;
use tracing::{error, info};

/// Register experiment-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    let ui_weak = adapter.weak();
    register_list_plan_types(ui, ui_weak.clone(), state.clone());
    register_queue_plan(ui, ui_weak.clone(), state.clone());
    register_start_engine(ui, ui_weak.clone(), state.clone());
    register_pause_engine(ui, ui_weak.clone(), state.clone());
    register_resume_engine(ui, ui_weak.clone(), state.clone());
    register_abort_plan(ui, ui_weak.clone(), state.clone());
    register_halt_engine(ui, ui_weak.clone(), state.clone());
    register_get_engine_status(ui, ui_weak, state);
}

fn register_list_plan_types(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_list_plan_types(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                info!("Listing plan types");

                match client.list_plan_types().await {
                    Ok(plan_types) => {
                        let slint_plans: Vec<PlanTypeInfo> = plan_types
                            .into_iter()
                            .map(|p| summary_to_plan_type_info(&p))
                            .collect();

                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            let model = Rc::new(VecModel::from(slint_plans));
                            ui.set_plan_types(model.into());
                        });
                    }
                    Err(e) => {
                        error!("ListPlanTypes failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("List Plans Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_queue_plan(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_queue_plan(move |type_id| {
        let type_id = type_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Queueing plan: {}", type_id);

                // Queue with empty params (simplified - full implementation would have a dialog)
                match client
                    .queue_plan(
                        &type_id,
                        std::collections::HashMap::new(),
                        std::collections::HashMap::new(),
                        std::collections::HashMap::new(),
                    )
                    .await
                {
                    Ok((run_uid, position)) => {
                        info!("Plan queued: {} at position {}", run_uid, position);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Plan Queued"),
                                SharedString::from(format!(
                                    "Position {} ({})",
                                    position,
                                    &run_uid[..8]
                                )),
                            );
                            // Refresh status to update queue count
                            ui.invoke_get_engine_status();
                        });
                    }
                    Err(e) => {
                        error!("QueuePlan failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Queue Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_start_engine(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_start_engine(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                info!("Starting engine");

                match client.start_engine().await {
                    Ok(()) => {
                        info!("Engine started");
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Engine Started"),
                                SharedString::from("Processing queue"),
                            );
                            ui.invoke_get_engine_status();
                        });
                    }
                    Err(e) => {
                        error!("StartEngine failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Start Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_pause_engine(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_pause_engine(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                info!("Pausing engine");

                match client.pause_engine(false).await {
                    Ok(paused_at) => {
                        info!("Engine paused at: {}", paused_at);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("info"),
                                SharedString::from("Engine Paused"),
                                SharedString::from(paused_at),
                            );
                            ui.invoke_get_engine_status();
                        });
                    }
                    Err(e) => {
                        error!("PauseEngine failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Pause Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_resume_engine(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_resume_engine(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                info!("Resuming engine");

                match client.resume_engine().await {
                    Ok(()) => {
                        info!("Engine resumed");
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Engine Resumed"),
                                SharedString::from("Continuing execution"),
                            );
                            ui.invoke_get_engine_status();
                        });
                    }
                    Err(e) => {
                        error!("ResumeEngine failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Resume Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_abort_plan(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_abort_plan(move |run_uid| {
        let run_uid = run_uid.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Aborting plan: {}", run_uid);

                let uid = if run_uid.is_empty() {
                    None
                } else {
                    Some(run_uid.as_str())
                };

                match client.abort_plan(uid).await {
                    Ok(()) => {
                        info!("Plan aborted");
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("warning"),
                                SharedString::from("Plan Aborted"),
                                SharedString::from("Execution stopped"),
                            );
                            ui.invoke_get_engine_status();
                        });
                    }
                    Err(e) => {
                        error!("AbortPlan failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Abort Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_halt_engine(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_halt_engine(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                info!("HALTING engine");

                match client.halt_engine().await {
                    Ok(message) => {
                        info!("Engine halted: {}", message);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("warning"),
                                SharedString::from("ENGINE HALTED"),
                                SharedString::from(message),
                            );
                            ui.invoke_get_engine_status();
                        });
                    }
                    Err(e) => {
                        error!("HaltEngine failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Halt Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_get_engine_status(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_get_engine_status(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                match client.get_engine_status().await {
                    Ok(status) => {
                        let state_str = engine_state_to_string(status.state());
                        let slint_status = EngineStatusInfo {
                            state: SharedString::from(&state_str),
                            current_run_uid: SharedString::from(
                                status.current_run_uid.as_deref().unwrap_or(""),
                            ),
                            current_plan_type: SharedString::from(
                                status.current_plan_type.as_deref().unwrap_or(""),
                            ),
                            current_event: status.current_event_number.unwrap_or(0) as i32,
                            total_events: status.total_events_expected.unwrap_or(0) as i32,
                            queued_plans: status.queued_plans as i32,
                            elapsed_seconds: (status.elapsed_ns as f64 / 1_000_000_000.0) as f32,
                        };

                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.set_engine_status(slint_status);
                        });
                    }
                    Err(e) => {
                        error!("GetEngineStatus failed: {}", e);
                    }
                }
            },
        );
    });
}

/// Convert gRPC PlanTypeSummary to Slint PlanTypeInfo
fn summary_to_plan_type_info(summary: &PlanTypeSummary) -> PlanTypeInfo {
    PlanTypeInfo {
        type_id: SharedString::from(&summary.type_id),
        display_name: SharedString::from(&summary.display_name),
        description: SharedString::from(&summary.description),
        categories: SharedString::from(&summary.categories.join(", ")),
    }
}

/// Convert EngineState enum to display string
fn engine_state_to_string(state: EngineState) -> String {
    match state {
        EngineState::EngineIdle => "idle".to_string(),
        EngineState::EngineRunning => "running".to_string(),
        EngineState::EnginePaused => "paused".to_string(),
        EngineState::EngineAborting => "aborting".to_string(),
        EngineState::EngineHalted => "halted".to_string(),
        EngineState::Unspecified => "unknown".to_string(),
    }
}
