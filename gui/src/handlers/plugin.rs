//! Plugin handlers
//!
//! Handles plugin device callbacks for dynamic UI rendering.
//! Part of GUI Phase bd-22si.6.2
//!
//! NOTE: This is an initial implementation. Full functionality requires
//! the PluginService gRPC backend to be wired up.

use crate::state::SharedState;
use crate::ui::{
    MainWindow, PluginDeviceInfo, PluginUiElementInfo, SharedString, UiAdapter, VecModel, Weak,
};
use std::rc::Rc;
use tracing::{error, info, warn};

/// Send-safe UI element data for transferring across threads
#[derive(Clone)]
struct UiElementData {
    element_type: String,
    label: String,
    target: String,
    source: String,
    action: String,
    current_value: f32,
    current_string: String,
    unit: String,
    min_value: f32,
    max_value: f32,
    options: Vec<String>,
    is_loading: bool,
    child_count: i32,
}

/// Register plugin-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter, state: SharedState) {
    let ui_weak = adapter.weak();
    register_refresh_plugins(ui, ui_weak.clone(), state.clone());
    register_slider_changed(ui, ui_weak.clone(), state.clone());
    register_readout_refresh(ui, ui_weak.clone(), state.clone());
    register_toggle_changed(ui, ui_weak.clone(), state.clone());
    register_action_triggered(ui, ui_weak.clone(), state.clone());
    register_dropdown_changed(ui, ui_weak.clone(), state.clone());

    // Initialize with example plugin devices for development
    initialize_example_plugins(ui);
}

fn initialize_example_plugins(ui: &MainWindow) {
    // Provide example plugin devices for UI development/testing
    // In production, this would query the PluginService gRPC endpoint
    let example_plugins = vec![
        PluginDeviceInfo {
            device_id: SharedString::from("ell14-stage-01"),
            device_name: SharedString::from("Thorlabs ELL14 Rotator"),
            plugin_id: SharedString::from("thorlabs-ell14"),
            connected: true,
            mock_mode: true,
            has_readable: true,
            has_movable: true,
            has_settable: false,
            has_switchable: false,
            has_actionable: true,
            ui_elements: Rc::new(VecModel::from(vec![
                PluginUiElementInfo {
                    element_type: SharedString::from("slider"),
                    label: SharedString::from("Position"),
                    target: SharedString::from("position"),
                    source: SharedString::new(),
                    action: SharedString::new(),
                    current_value: 45.0,
                    current_string: SharedString::new(),
                    unit: SharedString::from("deg"),
                    min_value: 0.0,
                    max_value: 360.0,
                    options: Rc::new(VecModel::default()).into(),
                    is_loading: false,
                    child_count: 0,
                },
                PluginUiElementInfo {
                    element_type: SharedString::from("readout"),
                    label: SharedString::from("Current Position"),
                    target: SharedString::new(),
                    source: SharedString::from("position"),
                    action: SharedString::new(),
                    current_value: 45.0,
                    current_string: SharedString::new(),
                    unit: SharedString::from("deg"),
                    min_value: 0.0,
                    max_value: 360.0,
                    options: Rc::new(VecModel::default()).into(),
                    is_loading: false,
                    child_count: 0,
                },
                PluginUiElementInfo {
                    element_type: SharedString::from("button"),
                    label: SharedString::from("Home"),
                    target: SharedString::new(),
                    source: SharedString::new(),
                    action: SharedString::from("home"),
                    current_value: 0.0,
                    current_string: SharedString::new(),
                    unit: SharedString::new(),
                    min_value: 0.0,
                    max_value: 0.0,
                    options: Rc::new(VecModel::default()).into(),
                    is_loading: false,
                    child_count: 0,
                },
            ]))
            .into(),
        },
        PluginDeviceInfo {
            device_id: SharedString::from("power-meter-01"),
            device_name: SharedString::from("Newport 1830-C Power Meter"),
            plugin_id: SharedString::from("newport-1830c"),
            connected: true,
            mock_mode: true,
            has_readable: true,
            has_movable: false,
            has_settable: true,
            has_switchable: true,
            has_actionable: false,
            ui_elements: Rc::new(VecModel::from(vec![
                PluginUiElementInfo {
                    element_type: SharedString::from("readout"),
                    label: SharedString::from("Power"),
                    target: SharedString::new(),
                    source: SharedString::from("power"),
                    action: SharedString::new(),
                    current_value: 0.000_042,
                    current_string: SharedString::new(),
                    unit: SharedString::from("W"),
                    min_value: 0.0,
                    max_value: 100.0,
                    options: Rc::new(VecModel::default()).into(),
                    is_loading: false,
                    child_count: 0,
                },
                PluginUiElementInfo {
                    element_type: SharedString::from("dropdown"),
                    label: SharedString::from("Wavelength"),
                    target: SharedString::from("wavelength"),
                    source: SharedString::new(),
                    action: SharedString::new(),
                    current_value: 0.0,
                    current_string: SharedString::from("800nm"),
                    unit: SharedString::new(),
                    min_value: 0.0,
                    max_value: 0.0,
                    options: Rc::new(VecModel::from(vec![
                        SharedString::from("532nm"),
                        SharedString::from("633nm"),
                        SharedString::from("800nm"),
                        SharedString::from("1064nm"),
                    ]))
                    .into(),
                    is_loading: false,
                    child_count: 0,
                },
                PluginUiElementInfo {
                    element_type: SharedString::from("toggle"),
                    label: SharedString::from("Auto-Range"),
                    target: SharedString::from("auto_range"),
                    source: SharedString::new(),
                    action: SharedString::new(),
                    current_value: 1.0, // 1.0 = on
                    current_string: SharedString::new(),
                    unit: SharedString::new(),
                    min_value: 0.0,
                    max_value: 1.0,
                    options: Rc::new(VecModel::default()).into(),
                    is_loading: false,
                    child_count: 0,
                },
            ]))
            .into(),
        },
    ];

    let model = Rc::new(VecModel::from(example_plugins));
    ui.set_plugin_devices(model.into());
}

fn register_refresh_plugins(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_refresh_plugins(move || {
        info!("Refreshing plugin devices");

        let ui_weak_clone = ui_weak.clone();
        let state_clone = state.clone();

        tokio::spawn(async move {
            // Get client from state
            let client = {
                let app_state = state_clone.lock().await;
                app_state.get_client()
            };

            let Some(client) = client else {
                error!("Cannot refresh plugins: not connected to daemon");
                let _ = ui_weak_clone.upgrade_in_event_loop(|ui| {
                    ui.invoke_show_toast(
                        SharedString::from("error"),
                        SharedString::from("Connection Error"),
                        SharedString::from("Not connected to daemon"),
                    );
                });
                return;
            };

            // Fetch plugin instances
            let instances = match client.list_plugin_instances().await {
                Ok(instances) => instances,
                Err(e) => {
                    let error_msg = e.to_string();
                    error!("Failed to list plugin instances: {}", error_msg);
                    let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                        ui.invoke_show_toast(
                            SharedString::from("error"),
                            SharedString::from("Plugin Refresh Failed"),
                            SharedString::from(format!("Failed to list plugins: {}", error_msg)),
                        );
                    });
                    return;
                }
            };

            // Build plugin device info for each instance
            // We collect Send-safe data first, then build PluginDeviceInfo in the event loop
            #[derive(Clone)]
            struct PluginDeviceData {
                device_id: String,
                device_name: String,
                plugin_id: String,
                connected: bool,
                mock_mode: bool,
                has_readable: bool,
                has_movable: bool,
                has_settable: bool,
                has_switchable: bool,
                has_actionable: bool,
                ui_elements: Vec<UiElementData>,
            }

            let mut plugin_data_list = Vec::new();
            for instance in instances {
                // Get plugin info to fetch UI layout and capabilities
                let plugin_info = match client.get_plugin_info(&instance.plugin_id).await {
                    Ok(info) => info,
                    Err(e) => {
                        warn!(
                            "Failed to get plugin info for {}: {}",
                            instance.plugin_id, e
                        );
                        continue;
                    }
                };

                // Convert UI elements from proto - just extract data, don't build Slint types
                let ui_elements: Vec<UiElementData> = plugin_info
                    .ui_layout
                    .into_iter()
                    .flat_map(|elem| convert_ui_element_data(&elem))
                    .collect();

                // Extract capability flags
                let caps = plugin_info.capabilities.as_ref();
                let has_readable = caps.map_or(false, |c| !c.readable.is_empty());
                let has_movable = caps.map_or(false, |c| c.movable.is_some());
                let has_settable = caps.map_or(false, |c| !c.settable.is_empty());
                let has_switchable = caps.map_or(false, |c| !c.switchable.is_empty());
                let has_actionable = caps.map_or(false, |c| !c.actionable.is_empty());

                plugin_data_list.push(PluginDeviceData {
                    device_id: instance.device_id,
                    device_name: plugin_info.name,
                    plugin_id: instance.plugin_id,
                    connected: instance.connected,
                    mock_mode: instance.mock_mode,
                    has_readable,
                    has_movable,
                    has_settable,
                    has_switchable,
                    has_actionable,
                    ui_elements,
                });
            }

            let count = plugin_data_list.len();

            // Update UI - build PluginDeviceInfo with Rc inside event loop
            let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                let plugin_devices: Vec<PluginDeviceInfo> = plugin_data_list
                    .into_iter()
                    .map(|data| {
                        let ui_elements: Vec<PluginUiElementInfo> = data
                            .ui_elements
                            .into_iter()
                            .map(|elem| PluginUiElementInfo {
                                element_type: SharedString::from(&elem.element_type),
                                label: SharedString::from(&elem.label),
                                target: SharedString::from(&elem.target),
                                source: SharedString::from(&elem.source),
                                action: SharedString::from(&elem.action),
                                current_value: elem.current_value,
                                current_string: SharedString::from(&elem.current_string),
                                unit: SharedString::from(&elem.unit),
                                min_value: elem.min_value,
                                max_value: elem.max_value,
                                options: Rc::new(VecModel::from(
                                    elem.options
                                        .into_iter()
                                        .map(|s| SharedString::from(&s))
                                        .collect::<Vec<_>>(),
                                ))
                                .into(),
                                is_loading: elem.is_loading,
                                child_count: elem.child_count,
                            })
                            .collect();

                        PluginDeviceInfo {
                            device_id: SharedString::from(&data.device_id),
                            device_name: SharedString::from(&data.device_name),
                            plugin_id: SharedString::from(&data.plugin_id),
                            connected: data.connected,
                            mock_mode: data.mock_mode,
                            has_readable: data.has_readable,
                            has_movable: data.has_movable,
                            has_settable: data.has_settable,
                            has_switchable: data.has_switchable,
                            has_actionable: data.has_actionable,
                            ui_elements: Rc::new(VecModel::from(ui_elements)).into(),
                        }
                    })
                    .collect();

                let model = Rc::new(VecModel::from(plugin_devices));
                ui.set_plugin_devices(model.into());

                ui.invoke_show_toast(
                    SharedString::from("success"),
                    SharedString::from("Plugins Refreshed"),
                    SharedString::from(format!("Loaded {} plugin device(s)", count)),
                );
            });
        });
    });
}

/// Convert a protobuf PluginUIElement to raw data (Send-safe)
/// Returns a vec to handle groups (which flatten to children)
fn convert_ui_element_data(elem: &rust_daq::grpc::PluginUiElement) -> Vec<UiElementData> {
    let element_type = elem.element_type.as_str();

    // Handle groups specially - they flatten into their children
    if element_type == "group" {
        return elem
            .children
            .iter()
            .flat_map(|child| convert_ui_element_data(child))
            .collect();
    }

    // Regular elements - extract raw data
    vec![UiElementData {
        element_type: elem.element_type.clone(),
        label: elem.label.clone(),
        target: elem.target.as_deref().unwrap_or("").to_string(),
        source: elem.source.as_deref().unwrap_or("").to_string(),
        action: elem.action.as_deref().unwrap_or("").to_string(),
        current_value: 0.0,
        current_string: String::new(),
        unit: String::new(),
        min_value: 0.0,
        max_value: 100.0,
        options: Vec::new(), // TODO: Extract from proto if available
        is_loading: false,
        child_count: elem.children.len() as i32,
    }]
}

fn register_slider_changed(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_plugin_slider_changed(move |device_id, target, value| {
        let device_id = device_id.to_string();
        let target = target.to_string();
        let state = state.clone();
        let ui_weak = ui_weak.clone();

        info!(
            "Plugin slider changed: device={}, target={}, value={}",
            device_id, target, value
        );

        tokio::spawn(async move {
            // Get client from state
            let client = {
                let app_state = state.lock().await;
                app_state.get_client()
            };

            let Some(client) = client else {
                error!("Cannot set slider value: not connected to daemon");
                return;
            };

            // Call appropriate gRPC method based on capability
            // For movable devices (sliders control position), use move_absolute
            // For settable parameters, use set_parameter
            let result = if target == "position" {
                // This is a movable device slider
                client
                    .move_absolute(&device_id, value as f64)
                    .await
                    .map(|pos| pos.to_string())
            } else {
                // This is a settable parameter slider
                client
                    .set_parameter(&device_id, &target, &value.to_string())
                    .await
            };

            let _ = ui_weak.upgrade_in_event_loop(move |ui| match result {
                Ok(actual_value) => {
                    ui.invoke_show_toast(
                        SharedString::from("success"),
                        SharedString::from("Value Set"),
                        SharedString::from(format!("{}.{} = {}", device_id, target, actual_value)),
                    );
                }
                Err(e) => {
                    error!("Failed to set slider value: {}", e);
                    ui.invoke_show_toast(
                        SharedString::from("error"),
                        SharedString::from("Slider Error"),
                        SharedString::from(format!(
                            "Failed to set {}.{}: {}",
                            device_id, target, e
                        )),
                    );
                }
            });
        });
    });
}

fn register_readout_refresh(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_plugin_readout_refresh(move |device_id, source| {
        let device_id = device_id.to_string();
        let source = source.to_string();
        let state = state.clone();
        let ui_weak = ui_weak.clone();

        info!(
            "Plugin readout refresh: device={}, source={}",
            device_id, source
        );

        tokio::spawn(async move {
            // Get client from state
            let client = {
                let app_state = state.lock().await;
                app_state.get_client()
            };

            let Some(client) = client else {
                error!("Cannot read value: not connected to daemon");
                return;
            };

            // Call HardwareService::ReadValue for readable devices
            // or GetParameter for parameter readouts
            let result = if source == "position" || source == "reading" {
                // This is a readable device (Readable trait)
                client
                    .read_value(&device_id)
                    .await
                    .map(|(value, units)| format!("{} {}", value, units))
            } else {
                // This is a parameter readout
                client.get_parameter(&device_id, &source).await
            };

            let _ = ui_weak.upgrade_in_event_loop(move |ui| match result {
                Ok(value) => {
                    ui.invoke_show_toast(
                        SharedString::from("info"),
                        SharedString::from("Readout Refreshed"),
                        SharedString::from(format!("{}.{} = {}", device_id, source, value)),
                    );
                }
                Err(e) => {
                    error!("Failed to read value: {}", e);
                    ui.invoke_show_toast(
                        SharedString::from("error"),
                        SharedString::from("Read Error"),
                        SharedString::from(format!(
                            "Failed to read {}.{}: {}",
                            device_id, source, e
                        )),
                    );
                }
            });
        });
    });
}

fn register_toggle_changed(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_plugin_toggle_changed(move |device_id, target, is_on| {
        let device_id = device_id.to_string();
        let target = target.to_string();
        let state = state.clone();
        let ui_weak = ui_weak.clone();

        info!(
            "Plugin toggle changed: device={}, target={}, is_on={}",
            device_id, target, is_on
        );

        tokio::spawn(async move {
            // Get client from state
            let client = {
                let app_state = state.lock().await;
                app_state.get_client()
            };

            let Some(client) = client else {
                error!("Cannot set toggle: not connected to daemon");
                return;
            };

            // Call HardwareService::SetParameter with boolean value
            // Toggles are typically boolean or enum parameters
            let value = if is_on { "true" } else { "false" };
            let result = client.set_parameter(&device_id, &target, value).await;

            let _ = ui_weak.upgrade_in_event_loop(move |ui| match result {
                Ok(actual_value) => {
                    ui.invoke_show_toast(
                        SharedString::from("success"),
                        SharedString::from("Toggle Changed"),
                        SharedString::from(format!("{}.{} = {}", device_id, target, actual_value)),
                    );
                }
                Err(e) => {
                    error!("Failed to set toggle: {}", e);
                    ui.invoke_show_toast(
                        SharedString::from("error"),
                        SharedString::from("Toggle Error"),
                        SharedString::from(format!(
                            "Failed to set {}.{}: {}",
                            device_id, target, e
                        )),
                    );
                }
            });
        });
    });
}

fn register_action_triggered(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_plugin_action_triggered(move |device_id, action| {
        let device_id = device_id.to_string();
        let action = action.to_string();
        let state = state.clone();
        let ui_weak = ui_weak.clone();

        info!(
            "Plugin action triggered: device={}, action={}",
            device_id, action
        );

        tokio::spawn(async move {
            // Get client from state
            let client = {
                let app_state = state.lock().await;
                app_state.get_client()
            };

            let Some(client) = client else {
                error!("Cannot execute action: not connected to daemon");
                return;
            };

            // Execute plugin-specific actions via HardwareService
            // Common actions: "home", "zero", "calibrate", etc.
            // These are typically device-specific commands
            let result = match action.as_str() {
                "home" => {
                    // For movable devices, home typically means move to position 0
                    client
                        .move_absolute(&device_id, 0.0)
                        .await
                        .map(|_| "Homed".to_string())
                }
                _ => {
                    // For other actions, use set_parameter with action name
                    // This allows plugin-defined actions to be triggered
                    client.set_parameter(&device_id, &action, "execute").await
                }
            };

            let _ = ui_weak.upgrade_in_event_loop(move |ui| match result {
                Ok(msg) => {
                    ui.invoke_show_toast(
                        SharedString::from("success"),
                        SharedString::from("Action Executed"),
                        SharedString::from(format!("{}.{}() - {}", device_id, action, msg)),
                    );
                }
                Err(e) => {
                    error!("Failed to execute action: {}", e);
                    ui.invoke_show_toast(
                        SharedString::from("error"),
                        SharedString::from("Action Error"),
                        SharedString::from(format!(
                            "Failed to execute {}.{}(): {}",
                            device_id, action, e
                        )),
                    );
                }
            });
        });
    });
}

fn register_dropdown_changed(ui: &MainWindow, ui_weak: Weak<MainWindow>, state: SharedState) {
    ui.on_plugin_dropdown_changed(move |device_id, target, value| {
        let device_id = device_id.to_string();
        let target = target.to_string();
        let value = value.to_string();
        let state = state.clone();
        let ui_weak = ui_weak.clone();

        info!(
            "Plugin dropdown changed: device={}, target={}, value={}",
            device_id, target, value
        );

        tokio::spawn(async move {
            // Get client from state
            let client = {
                let app_state = state.lock().await;
                app_state.get_client()
            };

            let Some(client) = client else {
                error!("Cannot set dropdown value: not connected to daemon");
                return;
            };

            // Call HardwareService::SetParameter for dropdown selections
            // Dropdowns typically set enum or string parameters
            let result = client.set_parameter(&device_id, &target, &value).await;

            let _ = ui_weak.upgrade_in_event_loop(move |ui| match result {
                Ok(actual_value) => {
                    ui.invoke_show_toast(
                        SharedString::from("success"),
                        SharedString::from("Selection Changed"),
                        SharedString::from(format!("{}.{} = {}", device_id, target, actual_value)),
                    );
                }
                Err(e) => {
                    error!("Failed to set dropdown value: {}", e);
                    ui.invoke_show_toast(
                        SharedString::from("error"),
                        SharedString::from("Dropdown Error"),
                        SharedString::from(format!(
                            "Failed to set {}.{}: {}",
                            device_id, target, e
                        )),
                    );
                }
            });
        });
    });
}
