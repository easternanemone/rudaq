//! Camera handlers
//!
//! Handles camera exposure control and frame streaming callbacks.

use super::common::spawn_rpc;
use crate::state::SharedState;
use crate::ui::{MainWindow, SelectedCamera, SharedString, UiAdapter, VecModel, Weak};
use slint::Model;
use tracing::{error, info};

/// Register camera-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    let ui_weak = adapter.weak();
    register_set_exposure(ui, ui_weak.clone(), state.clone());
    register_start_stream(ui, ui_weak.clone(), state.clone());
    register_stop_stream(ui, ui_weak, state);
}

fn register_set_exposure(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_set_camera_exposure(move |device_id, exposure_ms| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Setting exposure for {} to {} ms", device_id, exposure_ms);

                match client.set_exposure(&device_id, exposure_ms as f64).await {
                    Ok(actual_exposure) => {
                        info!("Exposure set to {} ms", actual_exposure);
                        let device_id_clone = device_id.clone();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            let cameras = ui.get_selected_cameras();
                            for i in 0..cameras.row_count() {
                                if let Some(mut c) = cameras.row_data(i) {
                                    if c.device_id.as_str() == device_id_clone {
                                        c.exposure_ms = actual_exposure as f32;
                                        if let Some(vm) = cameras
                                            .as_any()
                                            .downcast_ref::<VecModel<SelectedCamera>>()
                                        {
                                            vm.set_row_data(i, c);
                                        }
                                        break;
                                    }
                                }
                            }
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Exposure Set"),
                                SharedString::from(format!("{:.2} ms", actual_exposure)),
                            );
                        });
                    }
                    Err(e) => {
                        error!("SetExposure failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Exposure Failed"),
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
    ui.on_start_camera_stream(move |device_id| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Starting camera stream for {}", device_id);

                // Update UI to show streaming state
                let device_id_clone = device_id.clone();
                let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                    let cameras = ui.get_selected_cameras();
                    for i in 0..cameras.row_count() {
                        if let Some(mut c) = cameras.row_data(i) {
                            if c.device_id.as_str() == device_id_clone {
                                c.streaming = true;
                                c.frame_count = 0;
                                if let Some(vm) =
                                    cameras.as_any().downcast_ref::<VecModel<SelectedCamera>>()
                                {
                                    vm.set_row_data(i, c);
                                }
                                break;
                            }
                        }
                    }
                });

                // Start the frame stream on the device
                match client.start_frame_stream(&device_id, None).await {
                    Ok(()) => {
                        info!("Camera stream started for {}", device_id);

                        // Start receiving frames (metadata only for now)
                        if let Ok(mut frame_rx) = client.stream_frames(&device_id, false).await {
                            let ui_weak_stream = ui_weak.clone();
                            let device_id_stream = device_id.clone();

                            tokio::spawn(async move {
                                while let Some(frame) = frame_rx.recv().await {
                                    let device_id_clone = device_id_stream.clone();
                                    let frame_num = frame.frame_number as i32;

                                    let _ = ui_weak_stream.upgrade_in_event_loop(move |ui| {
                                        let cameras = ui.get_selected_cameras();
                                        for i in 0..cameras.row_count() {
                                            if let Some(mut c) = cameras.row_data(i) {
                                                if c.device_id.as_str() == device_id_clone {
                                                    c.frame_count = frame_num;
                                                    if let Some(vm) = cameras
                                                        .as_any()
                                                        .downcast_ref::<VecModel<SelectedCamera>>()
                                                    {
                                                        vm.set_row_data(i, c);
                                                    }
                                                    break;
                                                }
                                            }
                                        }
                                    });
                                }
                                tracing::debug!("Frame stream ended for {}", device_id_stream);
                            });
                        }
                    }
                    Err(e) => {
                        error!("StartFrameStream failed: {}", e);
                        let device_id_clone = device_id.clone();
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            // Reset streaming state
                            let cameras = ui.get_selected_cameras();
                            for i in 0..cameras.row_count() {
                                if let Some(mut c) = cameras.row_data(i) {
                                    if c.device_id.as_str() == device_id_clone {
                                        c.streaming = false;
                                        if let Some(vm) = cameras
                                            .as_any()
                                            .downcast_ref::<VecModel<SelectedCamera>>()
                                        {
                                            vm.set_row_data(i, c);
                                        }
                                        break;
                                    }
                                }
                            }
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Stream Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_stop_stream(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_stop_camera_stream(move |device_id| {
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Stopping camera stream for {}", device_id);

                match client.stop_frame_stream(&device_id).await {
                    Ok(frames_captured) => {
                        info!("Camera stream stopped, {} frames captured", frames_captured);
                        let device_id_clone = device_id.clone();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            let cameras = ui.get_selected_cameras();
                            for i in 0..cameras.row_count() {
                                if let Some(mut c) = cameras.row_data(i) {
                                    if c.device_id.as_str() == device_id_clone {
                                        c.streaming = false;
                                        if let Some(vm) = cameras
                                            .as_any()
                                            .downcast_ref::<VecModel<SelectedCamera>>()
                                        {
                                            vm.set_row_data(i, c);
                                        }
                                        break;
                                    }
                                }
                            }
                            ui.invoke_show_toast(
                                SharedString::from("info"),
                                SharedString::from("Stream Stopped"),
                                SharedString::from(format!("{} frames captured", frames_captured)),
                            );
                        });
                    }
                    Err(e) => {
                        error!("StopFrameStream failed: {}", e);
                        // Still update UI to clear streaming state
                        let device_id_clone = device_id.clone();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            let cameras = ui.get_selected_cameras();
                            for i in 0..cameras.row_count() {
                                if let Some(mut c) = cameras.row_data(i) {
                                    if c.device_id.as_str() == device_id_clone {
                                        c.streaming = false;
                                        if let Some(vm) = cameras
                                            .as_any()
                                            .downcast_ref::<VecModel<SelectedCamera>>()
                                        {
                                            vm.set_row_data(i, c);
                                        }
                                        break;
                                    }
                                }
                            }
                        });
                    }
                }
            },
        );
    });
}
