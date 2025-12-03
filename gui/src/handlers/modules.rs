//! Module handlers
//!
//! Handles module lifecycle callbacks: list types, create, configure, start/stop.

use super::common::spawn_rpc;
use crate::state::SharedState;
use crate::ui::{
    MainWindow, ModuleInstance, ModuleTypeInfo, SharedString, UiAdapter, VecModel, Weak,
};
use std::rc::Rc;
use tracing::{error, info};

/// Register module-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    let ui_weak = adapter.weak();
    register_list_module_types(ui, ui_weak.clone(), state.clone());
    register_list_modules(ui, ui_weak.clone(), state.clone());
    register_create_module(ui, ui_weak.clone(), state.clone());
    register_delete_module(ui, ui_weak.clone());
    register_configure_module(ui, ui_weak.clone(), state.clone());
    register_assign_device(ui, ui_weak.clone(), state.clone());
    register_unassign_device(ui, ui_weak.clone());
    register_start_module(ui, ui_weak.clone(), state.clone());
    register_pause_module(ui, ui_weak.clone(), state.clone());
    register_resume_module(ui, ui_weak.clone(), state.clone());
    register_stop_module(ui, ui_weak, state);
}

fn register_list_module_types(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_list_module_types(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                info!("Listing module types");

                match client.list_module_types().await {
                    Ok(types) => {
                        let slint_types: Vec<ModuleTypeInfo> = types
                            .into_iter()
                            .map(|t| ModuleTypeInfo {
                                type_id: SharedString::from(&t.type_id),
                                display_name: SharedString::from(&t.display_name),
                                description: SharedString::from(&t.description),
                                version: SharedString::from("1.0"),
                                required_roles: 0,
                                optional_roles: 0,
                                num_parameters: 0,
                                num_event_types: 0,
                                categories: SharedString::from(t.categories.join(",")),
                            })
                            .collect();

                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            let model = Rc::new(VecModel::from(slint_types));
                            ui.set_module_types(model.into());
                        });
                    }
                    Err(e) => {
                        error!("ListModuleTypes failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Module Types Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_list_modules(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_list_modules(move || {
        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            |client, ui_weak| async move {
                info!("Listing modules");

                match client.list_modules().await {
                    Ok(modules) => {
                        let slint_modules: Vec<ModuleInstance> = modules
                            .into_iter()
                            .map(|m| {
                                let state_str = match m.state {
                                    0 => "created",
                                    1 => "configured",
                                    2 => "running",
                                    3 => "paused",
                                    4 => "stopped",
                                    5 => "error",
                                    _ => "unknown",
                                };
                                ModuleInstance {
                                    module_id: SharedString::from(&m.module_id),
                                    type_id: SharedString::from(&m.type_id),
                                    instance_name: SharedString::from(&m.instance_name),
                                    state: SharedString::from(state_str),
                                    roles_filled: m.required_roles_filled as i32,
                                    roles_total: m.required_roles_total as i32,
                                    ready_to_start: m.ready_to_start,
                                    uptime_ms: (m.uptime_ns / 1_000_000) as i32,
                                    error_message: SharedString::from(""),
                                }
                            })
                            .collect();

                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            let model = Rc::new(VecModel::from(slint_modules));
                            ui.set_module_instances(model.into());
                        });
                    }
                    Err(e) => {
                        error!("ListModules failed: {}", e);
                    }
                }
            },
        );
    });
}

fn register_create_module(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_create_module(move |type_id| {
        let type_id = type_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Creating module of type: {}", type_id);

                let instance_name = format!("{}-instance", type_id);
                let initial_config = std::collections::HashMap::new();

                match client
                    .create_module(&type_id, &instance_name, initial_config)
                    .await
                {
                    Ok(module_id) => {
                        info!("Module created: {}", module_id);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.set_selected_module_id(SharedString::from(&module_id));
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Module Created"),
                                SharedString::from(format!("ID: {}", module_id)),
                            );
                            ui.invoke_list_modules();
                        });
                    }
                    Err(e) => {
                        error!("CreateModule failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Create Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_delete_module(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_delete_module(move |module_id| {
        let ui_weak = ui_weak.clone();
        let module_id = module_id.to_string();

        info!("Delete module requested: {}", module_id);

        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
            ui.invoke_show_toast(
                SharedString::from("warning"),
                SharedString::from("Not Implemented"),
                SharedString::from("Module deletion not yet available"),
            );
        });
    });
}

fn register_configure_module(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_configure_module(move |module_id, param_name, param_value| {
        let module_id = module_id.to_string();
        let param_name = param_name.to_string();
        let param_value = param_value.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!(
                    "Configuring module {}: {} = {}",
                    module_id, param_name, param_value
                );

                let mut params = std::collections::HashMap::new();
                params.insert(param_name.clone(), param_value.clone());

                match client.configure_module(&module_id, params, true).await {
                    Ok(()) => {
                        info!("Module configured: {}", module_id);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Parameter Set"),
                                SharedString::from(format!("{} = {}", param_name, param_value)),
                            );
                        });
                    }
                    Err(e) => {
                        error!("ConfigureModule failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Configure Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_assign_device(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_assign_device_to_module(move |module_id, role_name, device_id| {
        let module_id = module_id.to_string();
        let role_name = role_name.to_string();
        let device_id = device_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!(
                    "Assigning {} to module {} role {}",
                    device_id, module_id, role_name
                );

                match client
                    .assign_device(&module_id, &role_name, &device_id)
                    .await
                {
                    Ok(ready) => {
                        info!("Device assigned, module ready: {}", ready);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Device Assigned"),
                                SharedString::from(format!("{} -> {}", device_id, role_name)),
                            );
                            ui.invoke_list_modules();
                        });
                    }
                    Err(e) => {
                        error!("AssignDevice failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Assign Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}

fn register_unassign_device(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_unassign_device_from_module(move |module_id, role_name| {
        let ui_weak = ui_weak.clone();
        let module_id = module_id.to_string();
        let role_name = role_name.to_string();

        info!(
            "Unassign device from module {} role {}",
            module_id, role_name
        );

        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
            ui.invoke_show_toast(
                SharedString::from("warning"),
                SharedString::from("Not Implemented"),
                SharedString::from("Device unassignment not yet available"),
            );
        });
    });
}

fn register_start_module(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_start_module(move |module_id| {
        let module_id = module_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Starting module: {}", module_id);

                match client.start_module(&module_id).await {
                    Ok(start_time) => {
                        info!("Module started at {}", start_time);
                        let module_id_clone = module_id.clone();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.set_selected_module_running(true);
                            ui.invoke_show_toast(
                                SharedString::from("success"),
                                SharedString::from("Module Started"),
                                SharedString::from(&module_id_clone),
                            );
                            ui.invoke_list_modules();
                        });
                    }
                    Err(e) => {
                        error!("StartModule failed: {}", e);
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

fn register_pause_module(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_pause_module(move |module_id| {
        let module_id = module_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Pausing module: {}", module_id);

                match client.pause_module(&module_id).await {
                    Ok(()) => {
                        info!("Module paused: {}", module_id);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_list_modules();
                        });
                    }
                    Err(e) => {
                        error!("PauseModule failed: {}", e);
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
            },
        );
    });
}

fn register_resume_module(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_resume_module(move |module_id| {
        let module_id = module_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Resuming module: {}", module_id);

                match client.resume_module(&module_id).await {
                    Ok(()) => {
                        info!("Module resumed: {}", module_id);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_list_modules();
                        });
                    }
                    Err(e) => {
                        error!("ResumeModule failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("warning"),
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

fn register_stop_module(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_stop_module(move |module_id| {
        let module_id = module_id.to_string();

        spawn_rpc(
            ui_weak.clone(),
            state.clone(),
            move |client, ui_weak| async move {
                info!("Stopping module: {}", module_id);

                match client.stop_module(&module_id, false).await {
                    Ok(()) => {
                        info!("Module stopped: {}", module_id);
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.set_selected_module_running(false);
                            ui.invoke_list_modules();
                        });
                    }
                    Err(e) => {
                        error!("StopModule failed: {}", e);
                        let error_msg = e.to_string();
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.invoke_show_toast(
                                SharedString::from("error"),
                                SharedString::from("Stop Failed"),
                                SharedString::from(error_msg),
                            );
                        });
                    }
                }
            },
        );
    });
}
