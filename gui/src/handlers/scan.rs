//! Scan handlers
//!
//! Handles scan creation, execution, and progress streaming callbacks.

use super::common::spawn_with_state;
use crate::state::SharedState;
use crate::ui::{MainWindow, ScanStatus, SharedString, UiAdapter, Weak};
use std::sync::Arc;
use tracing::{error, info};

/// Register scan-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    let ui_weak = adapter.weak();
    register_create_scan(ui, ui_weak.clone(), state.clone());
    register_start_scan(ui, ui_weak.clone(), state.clone());
    register_pause_scan(ui, ui_weak.clone(), state.clone());
    register_stop_scan(ui, ui_weak, state);
}

fn register_create_scan(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_create_scan(move |config| {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();
        let device_id = config.device_id.to_string();
        let start = config.start as f64;
        let end = config.end as f64;
        let num_points = config.points as u32;

        spawn_with_state(ui_weak, state, move |state, ui_weak| async move {
            let mut state_guard = state.lock().await;
            let client = match &state_guard.client {
                Some(c) => c.clone(),
                None => {
                    error!("No client connection for scan");
                    return;
                }
            };

            info!(
                "Creating scan: {} from {} to {} ({} pts)",
                device_id, start, end, num_points
            );

            match client.create_scan(&device_id, start, end, num_points).await {
                Ok((scan_id, total_points)) => {
                    info!("Scan created: {} with {} points", scan_id, total_points);
                    state_guard.current_scan_id = Some(scan_id.clone());

                    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                        ui.set_scan_status(ScanStatus {
                            scan_id: SharedString::from(&scan_id),
                            state: SharedString::from("created"),
                            current_point: 0,
                            total_points: total_points as i32,
                            progress: 0.0,
                        });
                    });
                }
                Err(e) => {
                    error!("CreateScan failed: {}", e);
                    let error_msg = e.to_string();
                    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                        ui.invoke_show_toast(
                            SharedString::from("error"),
                            SharedString::from("Scan Failed"),
                            SharedString::from(error_msg),
                        );
                    });
                }
            }
        });
    });
}

fn register_start_scan(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_start_scan(move || {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        spawn_with_state(
            ui_weak.clone(),
            state.clone(),
            move |state, ui_weak| async move {
                let state_guard = state.lock().await;
                let scan_id = match &state_guard.current_scan_id {
                    Some(id) => id.clone(),
                    None => {
                        error!("No scan created to start");
                        return;
                    }
                };
                let client = match &state_guard.client {
                    Some(c) => c.clone(),
                    None => return,
                };
                drop(state_guard);

                info!("Starting scan: {}", scan_id);

                // Set running state
                let _ = ui_weak.upgrade_in_event_loop(|ui| {
                    ui.set_scan_running(true);
                });

                match client.start_scan(&scan_id).await {
                    Ok(_start_time) => {
                        info!("Scan started: {}", scan_id);

                        // Start progress streaming
                        if let Ok(mut progress_rx) =
                            client.stream_scan_progress(&scan_id, false).await
                        {
                            let ui_weak_stream = ui_weak.clone();
                            let state_stream = state.clone();

                            let handle = tokio::spawn(async move {
                                while let Some(progress) = progress_rx.recv().await {
                                    let scan_id_clone = scan_id.clone();
                                    let state_str = match progress.state {
                                        0 => "created",
                                        1 => "running",
                                        2 => "paused",
                                        3 => "completed",
                                        4 => "failed",
                                        5 => "cancelled",
                                        _ => "unknown",
                                    };

                                    let pct = if progress.total_points > 0 {
                                        (progress.point_index as f32 / progress.total_points as f32)
                                            * 100.0
                                    } else {
                                        0.0
                                    };

                                    let state_copy = state_str.to_string();
                                    let point = progress.point_index as i32;
                                    let total = progress.total_points as i32;

                                    let _ = ui_weak_stream.upgrade_in_event_loop(move |ui| {
                                        ui.set_scan_status(ScanStatus {
                                            scan_id: SharedString::from(&scan_id_clone),
                                            state: SharedString::from(&state_copy),
                                            current_point: point,
                                            total_points: total,
                                            progress: pct,
                                        });

                                        // Check if scan finished
                                        if state_copy == "completed"
                                            || state_copy == "failed"
                                            || state_copy == "cancelled"
                                        {
                                            ui.set_scan_running(false);
                                        }
                                    });
                                }

                                // Stream ended
                                let mut guard = state_stream.lock().await;
                                guard.scan_progress_handle = None;

                                let _ = ui_weak_stream.upgrade_in_event_loop(|ui| {
                                    ui.set_scan_running(false);
                                });
                            });

                            let mut guard = state.lock().await;
                            guard.scan_progress_handle = Some(handle);
                        }
                    }
                    Err(e) => {
                        error!("StartScan failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.set_scan_running(false);
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Start Scan Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_pause_scan(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_pause_scan(move || {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        tokio::spawn(async move {
            let state_guard = state.lock().await;
            let scan_id = match &state_guard.current_scan_id {
                Some(id) => id.clone(),
                None => return,
            };
            let client = match &state_guard.client {
                Some(c) => c.clone(),
                None => return,
            };
            drop(state_guard);

            info!("Pausing scan: {}", scan_id);

            match client.pause_scan(&scan_id).await {
                Ok(paused_at) => {
                    info!("Scan paused at point {}", paused_at);
                }
                Err(e) => {
                    error!("PauseScan failed: {}", e);
                    let error_msg = e.to_string();
                    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                        ui.invoke_show_toast(
                            SharedString::from("warning"),
                            SharedString::from("Pause Failed"),
                            SharedString::from(error_msg),
                        );
                    });
                }
            }
        });
    });
}

fn register_stop_scan(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_stop_scan(move || {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        tokio::spawn(async move {
            let mut state_guard = state.lock().await;
            let scan_id = match &state_guard.current_scan_id {
                Some(id) => id.clone(),
                None => return,
            };
            let client = match &state_guard.client {
                Some(c) => c.clone(),
                None => return,
            };

            // Cancel progress stream
            if let Some(handle) = state_guard.scan_progress_handle.take() {
                handle.abort();
            }
            drop(state_guard);

            info!("Stopping scan: {}", scan_id);

            match client.stop_scan(&scan_id).await {
                Ok(stopped_at) => {
                    info!("Scan stopped at point {}", stopped_at);
                    let _ = ui_weak.upgrade_in_event_loop(|ui| {
                        ui.set_scan_running(false);
                    });
                }
                Err(e) => {
                    error!("StopScan failed: {}", e);
                    let error_msg = e.to_string();
                    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                        ui.set_scan_running(false);
                        ui.invoke_show_toast(
                            SharedString::from("error"),
                            SharedString::from("Stop Failed"),
                            SharedString::from(error_msg),
                        );
                    });
                }
            }
        });
    });
}
