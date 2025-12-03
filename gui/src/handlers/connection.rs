//! Connection handlers
//!
//! Handles connect, disconnect, and retry connection callbacks.

use crate::services::{state_sync, DaqClient};
use crate::state::SharedState;
use crate::ui::{DeviceInfo, MainWindow, SharedString, UiAdapter};
use std::sync::Arc;
use tracing::{error, info};

/// Register connection-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    register_connect(ui, adapter.clone(), state.clone());
    register_disconnect(ui, adapter.clone(), state);
    register_retry(ui, adapter);
}

fn register_connect(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    ui.on_connect(move |address| {
        let state = Arc::clone(&state);
        let adapter = adapter.clone();
        let address = address.to_string();

        tokio::spawn(async move {
            info!("Connecting to {}", address);

            adapter.set_connecting();

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

                    // Store client
                    {
                        let mut state_guard = state.lock().await;
                        state_guard.client = Some(client.clone());
                        state_guard.selected_device_ids.clear();
                    }

                    // Convert to Slint DeviceInfo
                    let slint_devices: Vec<DeviceInfo> = devices
                        .iter()
                        .map(|d| DeviceInfo {
                            id: SharedString::from(&d.id),
                            name: SharedString::from(&d.name),
                            driver_type: SharedString::from(&d.driver_type),
                            is_movable: d.is_movable,
                            is_readable: d.is_readable,
                            is_triggerable: d.is_triggerable,
                            is_frame_producer: d.is_frame_producer,
                            online: true,
                            selected: false,
                            // Metadata from daemon (bd-pwjo)
                            position_units: SharedString::from(
                                d.position_units.as_deref().unwrap_or("mm"),
                            ),
                            reading_units: SharedString::from(
                                d.reading_units.as_deref().unwrap_or("W"),
                            ),
                            min_position: d.min_position.unwrap_or(-100.0) as f32,
                            max_position: d.max_position.unwrap_or(100.0) as f32,
                        })
                        .collect();

                    // Collect device IDs for module assignment dropdowns
                    let device_ids: Vec<SharedString> =
                        devices.iter().map(|d| SharedString::from(&d.id)).collect();

                    let device_count = slint_devices.len();

                    // Update UI via adapter
                    adapter.set_connected(device_count);
                    adapter.update_devices(slint_devices);
                    adapter.update_available_device_ids(device_ids);
                    adapter.clear_selected_panels();

                    // Start state sync stream via service
                    state_sync::start_state_stream(state.clone(), client, adapter.clone()).await;
                }
                Err(e) => {
                    error!("Connection failed: {}", e);
                    let error_msg = e.to_string();
                    adapter.set_connection_error(&error_msg);
                    adapter.show_error("Connection Failed", &error_msg);
                }
            }
        });
    });
}

fn register_disconnect(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    ui.on_disconnect(move || {
        let state = Arc::clone(&state);
        let adapter = adapter.clone();

        tokio::spawn(async move {
            info!("Disconnecting");

            // Cancel any running streams and clear selection
            {
                let mut state_guard = state.lock().await;
                state_guard.clear_on_disconnect();
            }

            // Update UI via adapter
            adapter.set_disconnected();
            adapter.reset_all_models();
        });
    });
}

fn register_retry(ui: &MainWindow, adapter: UiAdapter) {
    let ui_weak = adapter.weak();

    ui.on_retry_connection(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let address = ui.get_server_address();
            ui.invoke_connect(address);
        }
    });
}
