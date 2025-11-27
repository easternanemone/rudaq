//! rust-daq GUI Application
//!
//! Slint-based GUI for remote control of the rust-daq daemon via gRPC.

mod grpc_client;

use anyhow::Result;
use grpc_client::DaqClient;
use slint::{ComponentHandle, SharedString, VecModel};
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

slint::include_modules!();

/// Application state shared between UI and background tasks
struct AppState {
    client: Option<DaqClient>,
    power_stream_handle: Option<tokio::task::JoinHandle<()>>,
    position_stream_handle: Option<tokio::task::JoinHandle<()>>,
    /// ID of the currently selected movable device (for stage operations)
    selected_stage_id: Option<String>,
    /// ID of the currently selected readable device (for power meter operations)
    selected_power_meter_id: Option<String>,
}

impl AppState {
    fn new() -> Self {
        Self {
            client: None,
            power_stream_handle: None,
            position_stream_handle: None,
            selected_stage_id: None,
            selected_power_meter_id: None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rust_daq_gui=info".parse().unwrap())
                .add_directive("tonic=warn".parse().unwrap()),
        )
        .init();

    info!("Starting rust-daq GUI");

    // Create the UI
    let ui = MainWindow::new()?;
    let ui_weak = ui.as_weak();

    // Shared state
    let state = Arc::new(Mutex::new(AppState::new()));

    // Initialize with empty device model
    let empty_devices: Rc<VecModel<DeviceInfo>> = Rc::new(VecModel::default());
    ui.set_devices(empty_devices.into());

    // =========================================================================
    // Connect callback
    // =========================================================================
    {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        ui.on_connect(move |address| {
            let state = Arc::clone(&state);
            let ui_weak = ui_weak.clone();
            let address = address.to_string();

            tokio::spawn(async move {
                info!("Connecting to {}", address);

                let ui_weak_clone = ui_weak.clone();
                ui_weak
                    .upgrade_in_event_loop(move |ui| {
                        ui.set_connection_status(SharedString::from("Connecting..."));
                    })
                    .ok();

                match DaqClient::connect(&address).await {
                    Ok(client) => {
                        info!("Connected to daemon");

                        // Fetch devices
                        let devices = match client.list_devices().await {
                            Ok(d) => d,
                            Err(e) => {
                                error!("Failed to list devices: {}", e);
                                vec![]
                            }
                        };

                        // Find first movable device for stage operations
                        let first_movable = devices.iter().find(|d| d.is_movable).map(|d| d.id.clone());
                        // Find first readable device for power meter operations
                        let first_readable = devices.iter().find(|d| d.is_readable).map(|d| d.id.clone());

                        if let Some(ref id) = first_movable {
                            info!("Selected stage device: {}", id);
                        }
                        if let Some(ref id) = first_readable {
                            info!("Selected power meter device: {}", id);
                        }

                        // Store client and selected device IDs
                        {
                            let mut state_guard = state.lock().await;
                            state_guard.client = Some(client);
                            state_guard.selected_stage_id = first_movable;
                            state_guard.selected_power_meter_id = first_readable;
                        }

                        // Convert to Slint DeviceInfo (must be done before moving to UI thread)
                        let slint_devices: Vec<DeviceInfo> = devices
                            .into_iter()
                            .map(|d| DeviceInfo {
                                id: SharedString::from(d.id),
                                name: SharedString::from(d.name),
                                driver_type: SharedString::from(d.driver_type),
                                is_movable: d.is_movable,
                                is_readable: d.is_readable,
                                is_triggerable: d.is_triggerable,
                                is_frame_producer: d.is_frame_producer,
                                online: true,
                            })
                            .collect();

                        let device_count = slint_devices.len();

                        // Update UI - create new model inside event loop
                        ui_weak_clone
                            .upgrade_in_event_loop(move |ui| {
                                ui.set_connected(true);
                                ui.set_connection_status(SharedString::from(format!(
                                    "Connected ({} devices)",
                                    device_count
                                )));

                                // Create fresh model and set it
                                let model = Rc::new(VecModel::from(slint_devices));
                                ui.set_devices(model.into());
                            })
                            .ok();
                    }
                    Err(e) => {
                        error!("Connection failed: {}", e);
                        ui_weak_clone
                            .upgrade_in_event_loop(move |ui| {
                                ui.set_connected(false);
                                ui.set_connection_status(SharedString::from(format!(
                                    "Failed: {}",
                                    e
                                )));
                            })
                            .ok();
                    }
                }
            });
        });
    }

    // =========================================================================
    // Disconnect callback
    // =========================================================================
    {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        ui.on_disconnect(move || {
            let state = Arc::clone(&state);
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                info!("Disconnecting");

                // Cancel any running streams
                {
                    let mut state_guard = state.lock().await;
                    if let Some(handle) = state_guard.power_stream_handle.take() {
                        handle.abort();
                    }
                    if let Some(handle) = state_guard.position_stream_handle.take() {
                        handle.abort();
                    }
                    state_guard.client = None;
                }

                // Update UI - create empty model
                ui_weak
                    .upgrade_in_event_loop(move |ui| {
                        ui.set_connected(false);
                        ui.set_connection_status(SharedString::from("Disconnected"));
                        let empty_model: Rc<VecModel<DeviceInfo>> = Rc::new(VecModel::default());
                        ui.set_devices(empty_model.into());
                    })
                    .ok();
            });
        });
    }

    // =========================================================================
    // Move stage absolute
    // =========================================================================
    {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        ui.on_move_stage_absolute(move |position| {
            let state = Arc::clone(&state);
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let state_guard = state.lock().await;
                let stage_id = match &state_guard.selected_stage_id {
                    Some(id) => id.clone(),
                    None => {
                        error!("No movable device available");
                        return;
                    }
                };
                let client = match &state_guard.client {
                    Some(c) => c.clone(),
                    None => return,
                };
                drop(state_guard); // Release lock before async operations

                info!("Moving {} to {}", stage_id, position);

                ui_weak
                    .upgrade_in_event_loop(|ui| {
                        ui.set_stage_moving(true);
                    })
                    .ok();

                match client.move_absolute(&stage_id, position as f64).await {
                    Ok(final_pos) => {
                        info!("Move complete, position: {}", final_pos);
                        ui_weak
                            .upgrade_in_event_loop(move |ui| {
                                ui.set_stage_position(final_pos as f32);
                                ui.set_stage_moving(false);
                            })
                            .ok();
                    }
                    Err(e) => {
                        error!("Move failed: {}", e);
                        ui_weak
                            .upgrade_in_event_loop(|ui| {
                                ui.set_stage_moving(false);
                            })
                            .ok();
                    }
                }
            });
        });
    }

    // =========================================================================
    // Move stage relative
    // =========================================================================
    {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        ui.on_move_stage_relative(move |delta| {
            let state = Arc::clone(&state);
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let state_guard = state.lock().await;
                let stage_id = match &state_guard.selected_stage_id {
                    Some(id) => id.clone(),
                    None => {
                        error!("No movable device available");
                        return;
                    }
                };
                let client = match &state_guard.client {
                    Some(c) => c.clone(),
                    None => return,
                };
                drop(state_guard); // Release lock before async operations

                info!("Moving {} relative by {}", stage_id, delta);

                ui_weak
                    .upgrade_in_event_loop(|ui| {
                        ui.set_stage_moving(true);
                    })
                    .ok();

                match client.move_relative(&stage_id, delta as f64).await {
                    Ok(final_pos) => {
                        info!("Move complete, position: {}", final_pos);
                        ui_weak
                            .upgrade_in_event_loop(move |ui| {
                                ui.set_stage_position(final_pos as f32);
                                ui.set_stage_moving(false);
                            })
                            .ok();
                    }
                    Err(e) => {
                        error!("Move failed: {}", e);
                        ui_weak
                            .upgrade_in_event_loop(|ui| {
                                ui.set_stage_moving(false);
                            })
                            .ok();
                    }
                }
            });
        });
    }

    // =========================================================================
    // Stop stage
    // =========================================================================
    {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        ui.on_stop_stage(move || {
            let state = Arc::clone(&state);
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let state_guard = state.lock().await;
                let stage_id = match &state_guard.selected_stage_id {
                    Some(id) => id.clone(),
                    None => {
                        error!("No movable device available");
                        return;
                    }
                };
                let client = match &state_guard.client {
                    Some(c) => c.clone(),
                    None => return,
                };
                drop(state_guard); // Release lock before async operations

                info!("Stopping stage {}", stage_id);

                match client.stop_motion(&stage_id).await {
                    Ok(pos) => {
                        info!("Stage stopped at {}", pos);
                        ui_weak
                            .upgrade_in_event_loop(move |ui| {
                                ui.set_stage_position(pos as f32);
                                ui.set_stage_moving(false);
                            })
                            .ok();
                    }
                    Err(e) => {
                        error!("Stop failed: {}", e);
                    }
                }
            });
        });
    }

    // =========================================================================
    // Start power stream
    // =========================================================================
    {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        ui.on_start_power_stream(move || {
            let state = Arc::clone(&state);
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let mut state_guard = state.lock().await;
                let power_meter_id = match &state_guard.selected_power_meter_id {
                    Some(id) => id.clone(),
                    None => {
                        error!("No readable device available for power streaming");
                        return;
                    }
                };
                let client = match &state_guard.client {
                    Some(c) => c.clone(),
                    None => return,
                };

                info!("Starting power meter stream for {}", power_meter_id);

                let ui_weak_clone = ui_weak.clone();
                ui_weak
                    .upgrade_in_event_loop(|ui| {
                        ui.set_power_streaming(true);
                    })
                    .ok();

                let handle = tokio::spawn(async move {
                    match client.stream_values(&power_meter_id, 10).await {
                        Ok(mut stream) => {
                            while let Some(update) = stream.recv().await {
                                let value = update.value as f32;
                                ui_weak_clone
                                    .upgrade_in_event_loop(move |ui| {
                                        ui.set_power_reading(value);
                                    })
                                    .ok();
                            }
                        }
                        Err(e) => {
                            error!("Stream error: {}", e);
                        }
                    }

                    // Stream ended
                    ui_weak_clone
                        .upgrade_in_event_loop(|ui| {
                            ui.set_power_streaming(false);
                        })
                        .ok();
                });

                state_guard.power_stream_handle = Some(handle);
            });
        });
    }

    // =========================================================================
    // Stop power stream
    // =========================================================================
    {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        ui.on_stop_power_stream(move || {
            let state = Arc::clone(&state);
            let ui_weak = ui_weak.clone();

            tokio::spawn(async move {
                let mut state_guard = state.lock().await;
                if let Some(handle) = state_guard.power_stream_handle.take() {
                    info!("Stopping power meter stream");
                    handle.abort();
                }

                ui_weak
                    .upgrade_in_event_loop(|ui| {
                        ui.set_power_streaming(false);
                    })
                    .ok();
            });
        });
    }

    // =========================================================================
    // Scan callbacks (placeholders for now)
    // =========================================================================
    ui.on_create_scan(move |_config| {
        info!("Create scan requested");
    });

    ui.on_start_scan(move || {
        info!("Start scan requested");
    });

    ui.on_stop_scan(move || {
        info!("Stop scan requested");
    });

    ui.on_pause_scan(move || {
        info!("Pause scan requested");
    });

    // =========================================================================
    // Run the UI
    // =========================================================================
    info!("GUI ready, running event loop");
    ui.run()?;

    info!("GUI closed");
    Ok(())
}
