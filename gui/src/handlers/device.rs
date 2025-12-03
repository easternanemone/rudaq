//! Device handlers
//!
//! Handles device selection, motion control, and readable streaming callbacks.

use super::common::spawn_rpc;
use crate::state::SharedState;
use crate::ui::{
    DeviceInfo, MainWindow, SelectedCamera, SelectedLaser, SelectedMovable, SelectedReadable,
    SharedString, UiAdapter, VecModel, Weak,
};
use slint::Model;
use std::rc::Rc;
use std::sync::Arc;
use tracing::{error, info};

/// Register device-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    let ui_weak = adapter.weak();
    register_select_device(ui, ui_weak.clone());
    register_toggle_device(ui, ui_weak.clone(), state.clone());
    register_move_absolute(ui, ui_weak.clone(), state.clone());
    register_move_relative(ui, ui_weak.clone(), state.clone());
    register_stop_motion(ui, ui_weak.clone(), state.clone());
    register_home_device(ui, ui_weak.clone(), state.clone());
    register_start_stream(ui, ui_weak.clone(), state.clone());
    register_stop_stream(ui, ui_weak.clone());
    // Laser control handlers (bd-pwjo)
    register_laser_shutter(ui, ui_weak.clone(), state.clone());
    register_laser_wavelength(ui, ui_weak.clone(), state.clone());
    register_laser_emission(ui, ui_weak.clone(), state.clone());
    register_laser_stream(ui, ui_weak, state);
}

fn register_select_device(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_select_device(move |idx| {
        let ui_weak = ui_weak.clone();
        let idx = idx as usize;

        if let Some(ui) = ui_weak.upgrade() {
            let devices = ui.get_devices();
            if idx < devices.row_count() {
                if let Some(d) = devices.row_data(idx) {
                    let caps = match (d.is_movable, d.is_readable, d.is_frame_producer) {
                        (true, true, true) => "movable + readable + camera",
                        (true, true, false) => "movable + readable",
                        (true, false, true) => "movable + camera",
                        (false, true, true) => "readable + camera",
                        (true, false, false) => "movable",
                        (false, true, false) => "readable",
                        (false, false, true) => "camera",
                        (false, false, false) => "no capabilities",
                    };
                    ui.invoke_show_toast(
                        SharedString::from("info"),
                        SharedString::from(format!("{}", d.name)),
                        SharedString::from(format!("{} ({})", d.driver_type, caps)),
                    );
                }
            }
        }
    });
}

fn register_toggle_device(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_toggle_device(move |device_id, selected| {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();
        let device_id = device_id.to_string();

        tokio::spawn(async move {
            // Update selection state
            {
                let mut state_guard = state.lock().await;
                if selected {
                    state_guard.selected_device_ids.insert(device_id.clone());
                    info!("Device selected: {}", device_id);
                } else {
                    state_guard.selected_device_ids.remove(&device_id);
                    info!("Device deselected: {}", device_id);
                }
            }

            // Update UI
            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                let devices = ui.get_devices();

                // Update device's selected flag
                for i in 0..devices.row_count() {
                    if let Some(mut d) = devices.row_data(i) {
                        if d.id.as_str() == device_id {
                            d.selected = selected;
                            if let Some(vm) =
                                devices.as_any().downcast_ref::<VecModel<DeviceInfo>>()
                            {
                                vm.set_row_data(i, d.clone());
                            }
                            break;
                        }
                    }
                }

                // Rebuild selected panels from device list
                rebuild_selected_panels(&ui, &devices);
            });
        });
    });
}

/// Rebuild the selected movables, readables, cameras, and lasers panels from device selection
fn rebuild_selected_panels(ui: &MainWindow, devices: &slint::ModelRc<DeviceInfo>) {
    let mut movables: Vec<SelectedMovable> = Vec::new();
    let mut readables: Vec<SelectedReadable> = Vec::new();
    let mut cameras: Vec<SelectedCamera> = Vec::new();
    let mut lasers: Vec<SelectedLaser> = Vec::new();

    for i in 0..devices.row_count() {
        if let Some(device) = devices.row_data(i) {
            if !device.selected {
                continue;
            }

            // Check if this is a laser device (MaiTai or similar)
            // Use case-insensitive matching for driver type
            let driver_lower = device.driver_type.as_str().to_lowercase();
            let is_laser = driver_lower == "maitai"
                || driver_lower.contains("laser")
                || driver_lower.contains("ti:sapphire");

            if device.is_movable {
                movables.push(SelectedMovable {
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    position: 0.0,
                    // Use units from device metadata (bd-pwjo)
                    units: device.position_units.clone(),
                    is_moving: false,
                    min_position: device.min_position,
                    max_position: device.max_position,
                });
            }

            // For lasers, add to laser panel instead of readable panel
            if is_laser {
                lasers.push(SelectedLaser {
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    power_reading: 0.0,
                    power_units: device.reading_units.clone(),
                    wavelength_nm: 800.0, // Default for MaiTai
                    min_wavelength_nm: 690.0,
                    max_wavelength_nm: 1040.0,
                    shutter_open: false,
                    emission_enabled: false,
                    streaming: false,
                });
            } else if device.is_readable {
                readables.push(SelectedReadable {
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    value: 0.0,
                    // Use units from device metadata (bd-pwjo)
                    units: device.reading_units.clone(),
                    streaming: false,
                });
            }

            if device.is_frame_producer {
                cameras.push(SelectedCamera {
                    device_id: device.id.clone(),
                    device_name: device.name.clone(),
                    width: 512,
                    height: 512,
                    exposure_ms: 100.0,
                    min_exposure_ms: 0.1,
                    max_exposure_ms: 10000.0,
                    streaming: false,
                    frame_count: 0,
                });
            }
        }
    }

    ui.set_selected_movables(Rc::new(VecModel::from(movables)).into());
    ui.set_selected_readables(Rc::new(VecModel::from(readables)).into());
    ui.set_selected_cameras(Rc::new(VecModel::from(cameras)).into());
    ui.set_selected_lasers(Rc::new(VecModel::from(lasers)).into());
}

fn register_move_absolute(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_move_absolute(move |device_id, position| {
        let device_id = device_id.to_string();
        let ui_weak_clone = ui_weak.clone();
        let device_id_for_spawn = device_id.clone();

        // Set moving state
        set_moving_state(&ui_weak, &device_id, true);

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Moving {} to {}", device_id_for_spawn, position);

                match client
                    .move_absolute(&device_id_for_spawn, position as f64)
                    .await
                {
                    Ok(final_pos) => {
                        info!("Move complete, {} at {}", device_id_for_spawn, final_pos);
                        update_movable_position(
                            &ui_weak,
                            &device_id_for_spawn,
                            final_pos as f32,
                            false,
                        );
                    }
                    Err(e) => {
                        error!("Move failed: {}", e);
                        set_moving_state(&ui_weak, &device_id_for_spawn, false);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Move Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );

        // Also set moving state immediately in the spawning context
        set_moving_state(&ui_weak_clone, &device_id, true);
    });
}

fn register_move_relative(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_move_relative(move |device_id, delta| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Moving {} relative by {}", device_id, delta);

                set_moving_state(&ui_weak, &device_id, true);

                match client.move_relative(&device_id, delta as f64).await {
                    Ok(final_pos) => {
                        update_movable_position(&ui_weak, &device_id, final_pos as f32, false);
                    }
                    Err(e) => {
                        error!("Move failed: {}", e);
                        set_moving_state(&ui_weak, &device_id, false);
                    }
                }
            },
        );
    });
}

fn register_stop_motion(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_stop_motion(move |device_id| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Stopping {}", device_id);

                match client.stop_motion(&device_id).await {
                    Ok(pos) => {
                        info!("{} stopped at {}", device_id, pos);
                        update_movable_position(&ui_weak, &device_id, pos as f32, false);
                    }
                    Err(e) => {
                        error!("Stop failed: {}", e);
                    }
                }
            },
        );
    });
}

fn register_home_device(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_home_device(move |device_id| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Homing {}", device_id);

                match client.move_absolute(&device_id, 0.0).await {
                    Ok(final_pos) => {
                        update_movable_position(&ui_weak, &device_id, final_pos as f32, false);
                    }
                    Err(e) => {
                        error!("Home failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Home Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_start_stream(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_start_stream(move |device_id| {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();
        let device_id = device_id.to_string();

        tokio::spawn(async move {
            {
                let state_guard = state.lock().await;
                if state_guard.client.is_none() {
                    return;
                }
            }

            info!("Starting stream for {}", device_id);
            set_streaming_state(&ui_weak, &device_id, true);
        });
    });
}

fn register_stop_stream(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_stop_stream(move |device_id| {
        let device_id = device_id.to_string();
        set_streaming_state(&ui_weak, &device_id, false);
    });
}

// Helper functions

fn set_moving_state(ui_weak: &Weak<MainWindow>, device_id: &str, is_moving: bool) {
    let device_id = device_id.to_string();
    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        let movables = ui.get_selected_movables();
        for i in 0..movables.row_count() {
            if let Some(mut m) = movables.row_data(i) {
                if m.device_id.as_str() == device_id {
                    m.is_moving = is_moving;
                    if let Some(vm) = movables
                        .as_any()
                        .downcast_ref::<VecModel<SelectedMovable>>()
                    {
                        vm.set_row_data(i, m);
                    }
                    break;
                }
            }
        }
    });
}

fn update_movable_position(
    ui_weak: &Weak<MainWindow>,
    device_id: &str,
    position: f32,
    is_moving: bool,
) {
    let device_id = device_id.to_string();
    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        let movables = ui.get_selected_movables();
        for i in 0..movables.row_count() {
            if let Some(mut m) = movables.row_data(i) {
                if m.device_id.as_str() == device_id {
                    m.position = position;
                    m.is_moving = is_moving;
                    if let Some(vm) = movables
                        .as_any()
                        .downcast_ref::<VecModel<SelectedMovable>>()
                    {
                        vm.set_row_data(i, m);
                    }
                    break;
                }
            }
        }
    });
}

fn set_streaming_state(ui_weak: &Weak<MainWindow>, device_id: &str, streaming: bool) {
    let device_id = device_id.to_string();
    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        let readables = ui.get_selected_readables();
        for i in 0..readables.row_count() {
            if let Some(mut r) = readables.row_data(i) {
                if r.device_id.as_str() == device_id {
                    r.streaming = streaming;
                    if let Some(vm) = readables
                        .as_any()
                        .downcast_ref::<VecModel<SelectedReadable>>()
                    {
                        vm.set_row_data(i, r);
                    }
                    break;
                }
            }
        }
    });
}

// =============================================================================
// Laser Control Handlers (bd-pwjo)
// =============================================================================

fn register_laser_shutter(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_set_laser_shutter(move |device_id, open| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!(
                    "Setting shutter {} to {}",
                    device_id,
                    if open { "open" } else { "closed" }
                );

                match client.set_shutter(&device_id, open).await {
                    Ok(is_open) => {
                        info!(
                            "Shutter {} now {}",
                            device_id,
                            if is_open { "open" } else { "closed" }
                        );
                        update_laser_shutter(&ui_weak, &device_id, is_open);
                    }
                    Err(e) => {
                        error!("SetShutter failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Shutter Control Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_laser_wavelength(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_set_laser_wavelength(move |device_id, wavelength_nm| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Setting wavelength {} to {} nm", device_id, wavelength_nm);

                match client
                    .set_wavelength(&device_id, wavelength_nm as f64)
                    .await
                {
                    Ok(actual_wl) => {
                        info!("Wavelength {} now {} nm", device_id, actual_wl);
                        update_laser_wavelength(&ui_weak, &device_id, actual_wl as f32);
                    }
                    Err(e) => {
                        error!("SetWavelength failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Wavelength Control Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_laser_emission(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_set_laser_emission(move |device_id, enabled| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!(
                    "Setting emission {} to {}",
                    device_id,
                    if enabled { "ON" } else { "OFF" }
                );

                match client.set_emission(&device_id, enabled).await {
                    Ok(is_enabled) => {
                        info!(
                            "Emission {} now {}",
                            device_id,
                            if is_enabled { "ON" } else { "OFF" }
                        );
                        update_laser_emission(&ui_weak, &device_id, is_enabled);
                    }
                    Err(e) => {
                        error!("SetEmission failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Emission Control Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_laser_stream(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    // Start laser power reading stream
    let ui_weak_start = ui_weak.clone();
    let state_start = state.clone();
    ui.on_start_laser_stream(move |device_id| {
        let state = Arc::clone(&state_start);
        let ui_weak = ui_weak_start.clone();
        let device_id = device_id.to_string();

        tokio::spawn(async move {
            {
                let state_guard = state.lock().await;
                if state_guard.client.is_none() {
                    return;
                }
            }

            info!("Starting laser power stream for {}", device_id);
            set_laser_streaming(&ui_weak, &device_id, true);
        });
    });

    // Stop laser power reading stream
    ui.on_stop_laser_stream(move |device_id| {
        let device_id = device_id.to_string();
        info!("Stopping laser power stream for {}", device_id);
        set_laser_streaming(&ui_weak, &device_id, false);
    });
}

// Laser helper functions

fn update_laser_shutter(ui_weak: &Weak<MainWindow>, device_id: &str, is_open: bool) {
    let device_id = device_id.to_string();
    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        let lasers = ui.get_selected_lasers();
        for i in 0..lasers.row_count() {
            if let Some(mut laser) = lasers.row_data(i) {
                if laser.device_id.as_str() == device_id {
                    laser.shutter_open = is_open;
                    if let Some(vm) = lasers.as_any().downcast_ref::<VecModel<SelectedLaser>>() {
                        vm.set_row_data(i, laser);
                    }
                    break;
                }
            }
        }
    });
}

fn update_laser_wavelength(ui_weak: &Weak<MainWindow>, device_id: &str, wavelength_nm: f32) {
    let device_id = device_id.to_string();
    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        let lasers = ui.get_selected_lasers();
        for i in 0..lasers.row_count() {
            if let Some(mut laser) = lasers.row_data(i) {
                if laser.device_id.as_str() == device_id {
                    laser.wavelength_nm = wavelength_nm;
                    if let Some(vm) = lasers.as_any().downcast_ref::<VecModel<SelectedLaser>>() {
                        vm.set_row_data(i, laser);
                    }
                    break;
                }
            }
        }
    });
}

fn update_laser_emission(ui_weak: &Weak<MainWindow>, device_id: &str, is_enabled: bool) {
    let device_id = device_id.to_string();
    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        let lasers = ui.get_selected_lasers();
        for i in 0..lasers.row_count() {
            if let Some(mut laser) = lasers.row_data(i) {
                if laser.device_id.as_str() == device_id {
                    laser.emission_enabled = is_enabled;
                    if let Some(vm) = lasers.as_any().downcast_ref::<VecModel<SelectedLaser>>() {
                        vm.set_row_data(i, laser);
                    }
                    break;
                }
            }
        }
    });
}

fn set_laser_streaming(ui_weak: &Weak<MainWindow>, device_id: &str, streaming: bool) {
    let device_id = device_id.to_string();
    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        let lasers = ui.get_selected_lasers();
        for i in 0..lasers.row_count() {
            if let Some(mut laser) = lasers.row_data(i) {
                if laser.device_id.as_str() == device_id {
                    laser.streaming = streaming;
                    if let Some(vm) = lasers.as_any().downcast_ref::<VecModel<SelectedLaser>>() {
                        vm.set_row_data(i, laser);
                    }
                    break;
                }
            }
        }
    });
}
