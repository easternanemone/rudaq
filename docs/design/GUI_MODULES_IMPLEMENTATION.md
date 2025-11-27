# Modules Panel Implementation Guide

## Quick Start

### Step 1: Import ModulesPanel in main.slint

```slint
import { ModulesPanel } from "modules_panel.slint";

// In MainWindow TabWidget:
Tab {
    title: "Modules";
    ModulesPanel {
        available-module-types: root.available-module-types;
        module-instances: root.module-instances;
        available-devices: root.available-devices;
        current-module-roles: root.current-module-roles;
        current-module-parameters: root.current-module-parameters;
        current-module-events: root.current-module-events;
        selected-module-id <=> root.selected-module-id;
        selected-module-running <=> root.selected-module-running;

        // Callback handlers
        list-module-types => { root.refresh-module-types(); }
        list-modules => { root.refresh-modules(); }
        create-module(type-id) => { root.create-new-module(type-id); }
        delete-module(id) => { root.delete-module(id); }
        configure-module(module-id, param-id, value) => {
            root.update-module-parameter(module-id, param-id, value);
        }
        assign-device(module-id, role-id) => {
            root.assign-device(module-id, role-id);
        }
        unassign-device(module-id, role-id) => {
            root.remove-device-assignment(module-id, role-id);
        }
        start-module(id) => { root.start-module(id); }
        pause-module(id) => { root.pause-module(id); }
        resume-module(id) => { root.resume-module(id); }
        stop-module(id) => { root.stop-module(id); }
    }
}
```

### Step 2: Update gui/src/main.rs

Add AppState fields:
```rust
struct AppState {
    client: Option<DaqClient>,
    // ... existing fields ...

    // Module management
    available_module_types: Vec<UiModuleType>,
    module_instances: Vec<UiModuleInstance>,
    selected_module_id: Option<String>,
    current_module_roles: Vec<UiModuleRole>,
    current_module_parameters: Vec<UiModuleParameter>,
    current_module_events: Vec<UiModuleEvent>,
    module_event_stream_handle: Option<tokio::task::JoinHandle<()>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            client: None,
            available_module_types: Vec::new(),
            module_instances: Vec::new(),
            selected_module_id: None,
            current_module_roles: Vec::new(),
            current_module_parameters: Vec::new(),
            current_module_events: Vec::new(),
            module_event_stream_handle: None,
        }
    }
}
```

Add to main():
```rust
// Initialize empty module models
let empty_types = Rc::new(VecModel::<ModuleTypeInfo>::default());
ui.set_available_module_types(empty_types.into());

let empty_instances = Rc::new(VecModel::<ModuleInstance>::default());
ui.set_module_instances(empty_instances.into());

let empty_roles = Rc::new(VecModel::<ModuleRole>::default());
ui.set_current_module_roles(empty_roles.into());

let empty_params = Rc::new(VecModel::<ModuleParameter>::default());
ui.set_current_module_parameters(empty_params.into());

let empty_events = Rc::new(VecModel::<ModuleEvent>::default());
ui.set_current_module_events(empty_events.into());
```

### Step 3: Implement Callback Handlers

```rust
// List module types
{
    let state = Arc::clone(&state);
    let ui_weak = ui_weak.clone();

    ui.on_list_module_types(move |_| {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();

        tokio::spawn(async move {
            let mut app_state = state.lock().await;
            if let Some(ref client) = app_state.client {
                match client.list_module_types().await {
                    Ok(types) => {
                        app_state.available_module_types = types.clone();
                        let types_model: Vec<ModuleTypeInfo> = types
                            .into_iter()
                            .map(|t| ModuleTypeInfo {
                                type_id: SharedString::from(t.type_id),
                                display_name: SharedString::from(t.display_name),
                                description: SharedString::from(t.description),
                                version: SharedString::from(t.version),
                                required_roles: t.required_roles as i32,
                                optional_roles: t.optional_roles as i32,
                                num_parameters: t.num_parameters as i32,
                                num_event_types: t.num_event_types as i32,
                                categories: SharedString::from(t.categories),
                            })
                            .collect();

                        let types_model = Rc::new(VecModel::from(types_model));
                        ui_weak
                            .upgrade_in_event_loop(move |ui| {
                                ui.set_available_module_types(types_model.into());
                            })
                            .ok();
                    }
                    Err(e) => {
                        error!("Failed to list module types: {}", e);
                    }
                }
            }
        });
    });
}

// Create module
{
    let state = Arc::clone(&state);
    let ui_weak = ui_weak.clone();

    ui.on_create_module(move |type_id| {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();
        let type_id = type_id.to_string();

        tokio::spawn(async move {
            let mut app_state = state.lock().await;
            if let Some(ref client) = app_state.client {
                let instance_name = format!("{}-instance", type_id);
                match client.create_module(&type_id, &instance_name).await {
                    Ok(module_id) => {
                        info!("Created module: {}", module_id);
                        drop(app_state);
                        // Refresh module list
                        let state = Arc::clone(&state);
                        if let Ok(modules) = state.lock().await.client.as_ref()
                            .ok_or_else(|| anyhow!("No client"))
                            .and_then(|c| {
                                c.list_modules()
                            }) {
                            // Update UI with new list
                        }
                    }
                    Err(e) => {
                        error!("Failed to create module: {}", e);
                    }
                }
            }
        });
    });
}

// Configure module parameter
{
    let state = Arc::clone(&state);
    let ui_weak = ui_weak.clone();

    ui.on_configure_module(move |module_id, param_id, value| {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();
        let module_id = module_id.to_string();
        let param_id = param_id.to_string();
        let value = value.to_string();

        tokio::spawn(async move {
            let app_state = state.lock().await;
            if let Some(ref client) = app_state.client {
                match client.configure_module(&module_id, &param_id, &value).await {
                    Ok(_) => {
                        info!("Configured parameter: {}", param_id);
                    }
                    Err(e) => {
                        error!("Failed to configure parameter: {}", e);
                    }
                }
            }
        });
    });
}

// Start module
{
    let state = Arc::clone(&state);
    let ui_weak = ui_weak.clone();

    ui.on_start_module(move |module_id| {
        let state = Arc::clone(&state);
        let ui_weak = ui_weak.clone();
        let module_id = module_id.to_string();

        tokio::spawn(async move {
            let app_state = state.lock().await;
            if let Some(ref client) = app_state.client {
                match client.start_module(&module_id).await {
                    Ok(_) => {
                        info!("Started module: {}", module_id);
                    }
                    Err(e) => {
                        error!("Failed to start module: {}", e);
                    }
                }
            }
        });
    });
}

// Similar patterns for: list_modules, delete_module, assign_device,
// unassign_device, pause_module, resume_module, stop_module
```

## Component-Specific Usage Examples

### Using ModuleTypeBrowser

```slint
ModuleTypeBrowser {
    available-types: [
        {
            type-id: "power_monitor",
            display-name: "Power Monitor",
            description: "Real-time power measurement",
            version: "1.0",
            required-roles: 1,
            optional-roles: 0,
            num-parameters: 3,
            num-event-types: 2,
            categories: "monitoring,threshold",
        }
    ];

    module-type-selected(type-id) => {
        // Create new module of this type
    }
}
```

### Using DeviceAssignmentPanel

```slint
DeviceAssignmentPanel {
    roles: [
        {
            role-id: "power_meter",
            display-name: "Power Meter",
            required-capability: "readable",
            allows-multiple: false,
            assigned-device-id: "newport-1830c-001",
        },
        {
            role-id: "detector",
            display-name: "Detector",
            required-capability: "frame-producer",
            allows-multiple: false,
            assigned-device-id: "",
        }
    ];

    available-devices: ["power-meter-1", "camera-1", "stage-1"];

    assign-device(role-id, device-id) => {
        // Backend assigns device
    }

    unassign-device(role-id) => {
        // Backend removes assignment
    }
}
```

### Using ConfigurationPanel

```slint
ConfigurationPanel {
    parameters: [
        {
            param-id: "threshold",
            display-name: "Alert Threshold",
            description: "Power level threshold for alerts",
            param-type: "float",
            current-value: "10.5",
            default-value: "10.0",
            min-value: "0",
            max-value: "100",
            enum-values: "",
            units: "mW",
            required: true,
        },
        {
            param-id: "alert_enabled",
            display-name: "Enable Alerts",
            description: "Send alerts when threshold exceeded",
            param-type: "bool",
            current-value: "true",
            default-value: "false",
            min-value: "",
            max-value: "",
            enum-values: "",
            units: "",
            required: false,
        }
    ];

    is-running: false;

    parameter-changed(param-id, value) => {
        // Update parameter value
    }
}
```

### Using EventLog

```slint
EventLog {
    events: [
        {
            event-id: "evt-001",
            event-type: "ModuleCreated",
            timestamp-ms: 0,
            message: "Module instance created successfully",
            severity: "info",
        },
        {
            event-id: "evt-002",
            event-type: "ThresholdExceeded",
            timestamp-ms: 82000,
            message: "Power exceeded threshold: 125mW > 100mW",
            severity: "warning",
        },
        {
            event-id: "evt-003",
            event-type: "ConfigError",
            timestamp-ms: 85000,
            message: "Missing required role assignment: power_meter",
            severity: "error",
        }
    ];

    max-events: 100;  // Auto-trim to 100 most recent
}
```

## Data Type Conversions

Converting between Rust types and Slint models:

```rust
// UiModuleType -> ModuleTypeInfo (Slint)
let ui_module_type = UiModuleType {
    type_id: "power_monitor".to_string(),
    display_name: "Power Monitor".to_string(),
    // ...
};

let slint_module_type = ModuleTypeInfo {
    type_id: SharedString::from(ui_module_type.type_id),
    display_name: SharedString::from(ui_module_type.display_name),
    description: SharedString::from(ui_module_type.description),
    version: SharedString::from(ui_module_type.version),
    required_roles: ui_module_type.required_roles as i32,
    optional_roles: ui_module_type.optional_roles as i32,
    num_parameters: ui_module_type.num_parameters as i32,
    num_event_types: ui_module_type.num_event_types as i32,
    categories: SharedString::from(ui_module_type.categories),
};

// Create VecModel for ListView
let types_model = Rc::new(VecModel::from(vec![slint_module_type]));
ui.set_available_module_types(types_model.into());
```

## Testing the Panel

### Manual Testing Checklist

- [ ] Module type list loads on connect
- [ ] Clicking module type creates instance
- [ ] New instance appears in instance list
- [ ] Selecting instance shows roles
- [ ] Assigning device updates assignment UI
- [ ] Configuring parameter updates config
- [ ] Starting module changes state to "running"
- [ ] Events appear in event log in real-time
- [ ] Pausing module disables editing
- [ ] Stopping module allows deletion

### Mock Data for Testing

```rust
// In modules_client.rs - already provided for UI development
// Mock list_module_types returns predefined types
// Mock list_modules returns empty (user creates instances)
// Mock create_module returns generated ID
// Mock get_module_status returns template instance
```

## Performance Tuning

### Event Log Management
```rust
// Keep only last 100 events
if app_state.current_module_events.len() > 100 {
    app_state.current_module_events.remove(0);
}
```

### Lazy Loading
```rust
// Only fetch roles/params when module selected
ui.on_modules_module_selected(move |module_id| {
    // Fetch and display module details
});
```

### Device List Filtering
```rust
// Pre-filter devices by required capability in assignment panel
let compatible_devices = available_devices
    .iter()
    .filter(|d| has_capability(d, required_capability))
    .collect();
```

## Common Issues & Solutions

### Issue: Module list doesn't refresh after creation
**Solution**: Call `list_modules()` after `create_module()` succeeds

### Issue: Parameter changes blocked when running
**Solution**: Configuration panel checks `is-running` property and disables edits

### Issue: Event log grows unbounded
**Solution**: Event log component caps at `max-events` (default 100)

### Issue: Device assignment ComboBox always shows first device
**Solution**: Set `current-index: -1` on creation, update on selection

## Related Code Files

- **Slint UI**: `/Users/briansquires/code/rust-daq/gui/ui/modules_panel.slint`
- **Rust Client**: `/Users/briansquires/code/rust-daq/gui/src/modules_client.rs`
- **Main App**: `/Users/briansquires/code/rust-daq/gui/src/main.rs`
- **gRPC Proto**: `/Users/briansquires/code/rust-daq/proto/daq.proto`
- **Backend Service**: `/Users/briansquires/code/rust-daq/src/grpc/module_service.rs`
