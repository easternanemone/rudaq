//! Preset handlers
//!
//! Handles preset lifecycle callbacks: list, save, load, delete.
//! Part of GUI Phase 3 (bd-i1c5)

use super::common::spawn_rpc;
use crate::state::SharedState;
use crate::ui::{MainWindow, PresetInfo, SharedString, UiAdapter, VecModel, Weak};
use rust_daq::grpc::{Preset, PresetMetadata};
use std::rc::Rc;
use tracing::{error, info};

/// Register preset-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    let ui_weak = adapter.weak();
    register_list_presets(ui, ui_weak.clone(), state.clone());
    register_save_preset(ui, ui_weak.clone(), state.clone());
    register_load_preset(ui, ui_weak.clone(), state.clone());
    register_delete_preset(ui, ui_weak, state);
}

fn register_list_presets(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_list_presets(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                info!("Listing presets");

                match client.list_presets().await {
                    Ok(presets) => {
                        let slint_presets: Vec<PresetInfo> = presets
                            .into_iter()
                            .map(|p| metadata_to_preset_info(&p))
                            .collect();

                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            let model = Rc::new(VecModel::from(slint_presets));
                            ui.set_presets(model.into());
                        });
                    }
                    Err(e) => {
                        error!("ListPresets failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("List Presets Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_save_preset(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_save_preset(move |name, description| {
        let name = name.to_string();
        let description = description.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Saving preset: {}", name);

                // Create preset with metadata
                let preset = Preset {
                    meta: Some(PresetMetadata {
                        preset_id: String::new(), // Server assigns ID
                        name: name.clone(),
                        description: description.clone(),
                        author: "GUI User".to_string(),
                        created_at_ns: 0,
                        updated_at_ns: 0,
                        schema_version: 1,
                    }),
                    device_configs_json: std::collections::HashMap::new(), // Server captures current state
                    scan_template_json: String::new(),
                };

                match client.save_preset(preset, false).await {
                    Ok(message) => {
                        info!("Preset saved: {}", message);
                        let name_clone = name.clone();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Preset Saved"),
                                SharedString::from(&name_clone),
                            );
                            // Refresh the list
                            ui.invoke_list_presets();
                        });
                    }
                    Err(e) => {
                        error!("SavePreset failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Save Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_load_preset(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_load_preset(move |preset_id| {
        let preset_id = preset_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Loading preset: {}", preset_id);

                match client.load_preset(&preset_id).await {
                    Ok(message) => {
                        info!("Preset loaded: {}", message);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Preset Loaded"),
                                SharedString::from("Device configurations applied"),
                            );
                            // Refresh device states
                            ui.invoke_refresh_devices();
                        });
                    }
                    Err(e) => {
                        error!("LoadPreset failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Load Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_delete_preset(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_delete_preset(move |preset_id| {
        let preset_id = preset_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Deleting preset: {}", preset_id);

                match client.delete_preset(&preset_id).await {
                    Ok(message) => {
                        info!("Preset deleted: {}", message);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("info"),
                                SharedString::from("Preset Deleted"),
                                SharedString::from("Preset removed"),
                            );
                            // Refresh the list
                            ui.invoke_list_presets();
                        });
                    }
                    Err(e) => {
                        error!("DeletePreset failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Delete Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

/// Convert gRPC PresetMetadata to Slint PresetInfo
fn metadata_to_preset_info(meta: &PresetMetadata) -> PresetInfo {
    // Convert nanoseconds to human-readable date
    let created_str = format_timestamp_ns(meta.created_at_ns);
    let updated_str = format_timestamp_ns(meta.updated_at_ns);

    PresetInfo {
        preset_id: SharedString::from(&meta.preset_id),
        name: SharedString::from(&meta.name),
        description: SharedString::from(&meta.description),
        author: SharedString::from(&meta.author),
        created_at: SharedString::from(&created_str),
        updated_at: SharedString::from(&updated_str),
        device_count: 0, // Would need to fetch full preset to count
    }
}

/// Format a nanosecond timestamp to a human-readable string
fn format_timestamp_ns(ns: u64) -> String {
    if ns == 0 {
        return "Unknown".to_string();
    }

    use std::time::{Duration, UNIX_EPOCH};
    let duration = Duration::from_nanos(ns);
    let datetime = UNIX_EPOCH + duration;

    // Simple formatting (could use chrono for better formatting)
    match datetime.elapsed() {
        Ok(elapsed) => {
            let secs = elapsed.as_secs();
            if secs < 60 {
                "Just now".to_string()
            } else if secs < 3600 {
                format!("{} min ago", secs / 60)
            } else if secs < 86400 {
                format!("{} hours ago", secs / 3600)
            } else {
                format!("{} days ago", secs / 86400)
            }
        }
        Err(_) => "Future".to_string(),
    }
}
