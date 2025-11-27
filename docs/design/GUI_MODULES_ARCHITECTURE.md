# Modules Panel Architecture

## System Context

The Modules panel is the GUI component for the ModuleService (bd-c0ai), enabling experiment module management with runtime device assignment. It follows the headless-first V5 architecture where the GUI is optional, and the gRPC daemon provides the actual orchestration.

### Architectural Layers

```
┌─────────────────────────────────────────────────────────────────┐
│ Slint GUI (Presentation Layer)                                  │
│ ┌──────────────────────────────────────────────────────────────┐│
│ │ ModulesPanel Component                                       ││
│ │ ├── ModuleTypeBrowser (Discovery)                            ││
│ │ ├── ModuleInstanceList (Lifecycle)                           ││
│ │ ├── DeviceAssignmentPanel (Runtime binding)                  ││
│ │ ├── ConfigurationPanel (Parameter management)                ││
│ │ ├── EventLog (Monitoring)                                    ││
│ │ └── DataPreview (Visualization placeholder)                  ││
│ └──────────────────────────────────────────────────────────────┘│
└──────────────┬──────────────────────────────────────────────────┘
               │ gRPC callbacks
┌──────────────▼──────────────────────────────────────────────────┐
│ Rust Application Layer (gui/src)                                │
│ ┌──────────────────────────────────────────────────────────────┐│
│ │ main.rs (event loop & state management)                      ││
│ │ ├── AppState struct (module instance state)                  ││
│ │ ├── Callback handlers (UI -> gRPC)                           ││
│ │ └── Event stream task (gRPC -> UI)                           ││
│ ├──────────────────────────────────────────────────────────────┤│
│ │ modules_client.rs (ModuleService wrapper)                    ││
│ │ ├── UiModuleType, UiModuleInstance, etc. (data models)       ││
│ │ └── DaqClient methods (async gRPC calls)                     ││
│ ├──────────────────────────────────────────────────────────────┤│
│ │ grpc_client.rs (base gRPC client)                            ││
│ │ └── Connection management & proto client setup               ││
│ └──────────────────────────────────────────────────────────────┘│
└──────────────┬──────────────────────────────────────────────────┘
               │ tonic gRPC
┌──────────────▼──────────────────────────────────────────────────┐
│ gRPC Daemon (src/grpc)                                          │
│ ┌──────────────────────────────────────────────────────────────┐│
│ │ ModuleService (tonic server)                                 ││
│ │ ├── ListModuleTypes()                                        ││
│ │ ├── CreateModule()                                           ││
│ │ ├── ConfigureModule()                                        ││
│ │ ├── AssignDevice()                                           ││
│ │ ├── StartModule()                                            ││
│ │ └── StreamModuleEvents()                                     ││
│ ├──────────────────────────────────────────────────────────────┤│
│ │ Module Registry (src/modules)                                ││
│ │ ├── ModuleRegistry (instance storage)                        ││
│ │ ├── ModuleType traits (behavior definition)                  ││
│ │ └── Device Registry integration                              ││
│ ├──────────────────────────────────────────────────────────────┤│
│ │ Hardware Registry (src/hardware/registry.rs)                 ││
│ │ └── Device lookup & compatibility checking                   ││
│ └──────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

## Component Breakdown

### 1. Slint UI Components (gui/ui/modules_panel.slint)

#### ModuleTypeBrowser
- **Purpose**: Module type discovery and selection
- **Inputs**: `[ModuleTypeInfo]` available_types
- **Outputs**: `module-type-selected(type-id)`
- **Behavior**: Single-selection list with metadata display
- **Dependencies**: None (pure UI)

#### ModuleInstanceList
- **Purpose**: Display running module instances
- **Inputs**: `[ModuleInstance]` instances
- **Outputs**: `module-selected(module-id)`, `create-module()`
- **Behavior**: Color-coded status indicators, contextual info
- **Dependencies**: None (pure UI)

#### DeviceAssignmentPanel
- **Purpose**: Assign compatible devices to module roles
- **Inputs**: `[ModuleRole]` roles, `[string]` available_devices
- **Outputs**: `assign-device(role-id, device-id)`, `unassign-device(role-id)`
- **Behavior**: ComboBox for unassigned roles, badge for assigned
- **Dependencies**: None (pure UI)

#### ConfigurationPanel
- **Purpose**: Edit module parameters with type-specific controls
- **Inputs**: `[ModuleParameter]` parameters, `bool` is_running
- **Outputs**: `parameter-changed(param-id, value)`
- **Behavior**: Type-aware input controls, bounds validation
- **Dependencies**: None (pure UI)

#### EventLog
- **Purpose**: Real-time event stream display
- **Inputs**: `[ModuleEvent]` events, `int` max_events
- **Outputs**: None (view-only)
- **Behavior**: Color-coded severity, time formatting, auto-scroll
- **Dependencies**: None (pure UI)

#### DataPreview
- **Purpose**: Visualization placeholder
- **Inputs**: `string` module_id, `string` module_state
- **Outputs**: None
- **Behavior**: Placeholder for Phase 2 implementation
- **Dependencies**: None (pure UI)

#### ModulesPanel (Root)
- **Purpose**: Orchestrate all sub-components in tabbed interface
- **Inputs**: All of the above
- **Outputs**: All of the above callbacks
- **Behavior**: Tab selection, conditional rendering, state coordination
- **Dependencies**: All sub-components

### 2. Rust Client Layer (gui/src/modules_client.rs)

#### Data Models
```rust
pub struct UiModuleType {
    pub type_id: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub required_roles: usize,
    pub optional_roles: usize,
    pub num_parameters: usize,
    pub num_event_types: usize,
    pub categories: String,
}

pub struct UiModuleInstance {
    pub module_id: String,
    pub type_id: String,
    pub instance_name: String,
    pub state: String,
    pub roles_filled: usize,
    pub roles_total: usize,
    pub ready_to_start: bool,
    pub uptime_ms: u64,
    pub error_message: String,
}

// Similar for UiModuleRole, UiModuleParameter, UiModuleEvent
```

#### Client Methods
All methods are async and return `Result<T>`:
- `list_module_types()` -> Vec<UiModuleType>
- `get_module_type_info(type_id)` -> UiModuleType
- `list_modules()` -> Vec<UiModuleInstance>
- `create_module(type_id, instance_name)` -> String (module_id)
- `delete_module(module_id)` -> ()
- `get_module_status(module_id)` -> UiModuleInstance
- `get_module_config(module_id)` -> Vec<UiModuleParameter>
- `configure_module(module_id, param_id, value)` -> ()
- `get_module_roles(module_id)` -> Vec<UiModuleRole>
- `assign_device(module_id, role_id, device_id)` -> ()
- `unassign_device(module_id, role_id)` -> ()
- `start_module(module_id)` -> ()
- `pause_module(module_id)` -> ()
- `resume_module(module_id)` -> ()
- `stop_module(module_id)` -> ()
- `get_module_events(module_id, limit)` -> Vec<UiModuleEvent>

**Current Status**: Mock implementations provided for UI development. Real implementations will call gRPC ModuleService.

### 3. Application State Management (gui/src/main.rs)

#### AppState Structure
```rust
struct AppState {
    client: Option<DaqClient>,

    // Module management
    available_module_types: Vec<UiModuleType>,
    module_instances: Vec<UiModuleInstance>,
    selected_module_id: Option<String>,
    current_module_roles: Vec<UiModuleRole>,
    current_module_parameters: Vec<UiModuleParameter>,
    current_module_events: Vec<UiModuleEvent>,
    module_event_stream_handle: Option<tokio::task::JoinHandle<()>>,

    // ... existing fields (devices, streams, etc.)
}
```

#### State Management Pattern
```rust
// 1. Lock shared state
let mut app_state = state.lock().await;

// 2. Access client
if let Some(ref client) = app_state.client {
    // 3. Make async gRPC call
    let result = client.operation().await;

    // 4. Update state
    app_state.field = result;
}

// 5. Release lock, update UI
ui_weak.upgrade_in_event_loop(move |ui| {
    ui.set_field(converted_model);
}).ok();
```

#### Event Stream Pattern
```rust
// Spawn background task to stream module events
tokio::spawn(async move {
    let mut stream = client.stream_module_events(&module_id).await?;
    while let Some(event) = stream.next().await {
        // Update UI with new event
        app_state.current_module_events.push(event);
        // Notify UI (upgrade_in_event_loop)
    }
});
```

### 4. gRPC Integration (gui/src/grpc_client.rs)

#### DaqClient Extension
```rust
impl DaqClient {
    // New methods added to support modules
    pub async fn list_module_types(&self) -> Result<Vec<UiModuleType>> {
        let request = ListModuleTypesRequest::default();
        let response = self.module_client.list_module_types(request).await?;
        // Convert proto to UI types
        Ok(convert_module_types(response.module_types))
    }

    // ... other methods
}
```

#### Proto Import
```rust
use crate::grpc::proto::module_service_client::ModuleServiceClient;
use crate::grpc::proto::{ListModuleTypesRequest, /* ... */};
```

## Data Flow Diagrams

### Module Creation Flow

```
User clicks "New Module"
    ↓
ModuleTypeBrowser.module-type-selected()
    ↓
main.rs: on_create_module(type_id)
    ↓
Async tokio task spawns
    ↓
client.create_module(type_id)
    ↓
gRPC: ModuleService.CreateModule()
    ↓
Backend creates instance, returns module_id
    ↓
main.rs refreshes module list: list_modules()
    ↓
client.list_modules()
    ↓
gRPC: ModuleService.ListModules()
    ↓
Backend returns [UiModuleInstance]
    ↓
ui.set_module_instances(new_list)
    ↓
ModuleInstanceList re-renders with new instance
```

### Device Assignment Flow

```
User selects unassigned role in DeviceAssignmentPanel
    ↓
ComboBox shows available devices
    ↓
User selects device
    ↓
DeviceAssignmentPanel.assign-device(role_id, device_id)
    ↓
main.rs: on_assign_device(module_id, role_id)
    ↓
Async tokio task spawns
    ↓
client.assign_device(module_id, role_id, device_id)
    ↓
gRPC: ModuleService.AssignDevice()
    ↓
Backend validates capability and assigns
    ↓
main.rs refreshes module roles: get_module_roles(module_id)
    ↓
client.get_module_roles(module_id)
    ↓
gRPC: ModuleService.ListAssignments()
    ↓
Backend returns [UiModuleRole] with assignments
    ↓
ui.set_current_module_roles(updated_roles)
    ↓
DeviceAssignmentPanel re-renders with new assignment
```

### Module Lifecycle Flow

```
START:
  user.click("Start")
    ↓
  client.start_module(module_id)
    ↓
  gRPC: ModuleService.StartModule()
    ↓
  Backend: Module state "running"
    ↓
  client.get_module_status(module_id)
    ↓
  ui.set_selected_module_running(true)
    ↓
  ConfigurationPanel disables editing
    ↓
  Background task streams events
    ↓
    EventLog receives events in real-time

PAUSE:
  user.click("Pause")
    ↓
  client.pause_module(module_id)
    ↓
  gRPC: ModuleService.PauseModule()
    ↓
  Backend: Module state "paused"

RESUME:
  user.click("Resume")
    ↓
  client.resume_module(module_id)
    ↓
  gRPC: ModuleService.ResumeModule()
    ↓
  Backend: Module state "running" again

STOP:
  user.click("Stop")
    ↓
  client.stop_module(module_id)
    ↓
  gRPC: ModuleService.StopModule()
    ↓
  Backend: Module state "stopped"
    ↓
  Event stream terminates
    ↓
  user.click("Delete") now enabled
```

## Threading & Concurrency

### Main Thread (UI Thread)
- Slint event loop (UI rendering, user interactions)
- Callback invocation from user input

### Tokio Runtime Threads
- Async gRPC calls (non-blocking)
- Event stream receiving (background task)
- Protobuf encoding/decoding

### Synchronization
- `Arc<Mutex<AppState>>` for shared state
- `VecModel` for UI data binding
- `upgrade_in_event_loop()` for cross-thread UI updates

## Error Handling Strategy

### Network Errors
```rust
match client.list_modules().await {
    Ok(modules) => {
        // Update UI
    }
    Err(e) => {
        error!("Failed to list modules: {}", e);
        // Show error in status bar or event log
        app_state.current_module_events.push(UiModuleEvent {
            event_type: "Error".to_string(),
            message: format!("Network error: {}", e),
            severity: "error".to_string(),
            // ...
        });
    }
}
```

### Validation Errors
- Parameter bounds checked on UI before sending
- Backend returns error in response
- Error displayed in event log
- User retries with valid value

### State Conflicts
- Prevent invalid transitions (e.g., start when already running)
- UI disables buttons based on current state
- Backend rejects invalid transitions with error

## Performance Considerations

### UI Responsiveness
- All gRPC calls in tokio tasks (non-blocking)
- VecModel uses reference counting (efficient updates)
- Event log caps at 100 entries (configurable)

### Memory Usage
- Event stream buffers in-memory
- Old events dropped when buffer full
- Module instances kept in AppState (reasonable count)

### Network Bandwidth
- gRPC/protobuf efficient encoding
- Streaming for events (vs polling)
- Device list cached after connect

## Testing Approach

### Unit Tests
- `modules_client.rs`: Test mock implementations
- Data model conversions

### Integration Tests (future)
- Launch local gRPC server
- Test full module workflow
- Verify event streaming

### Manual Testing
- Connect to daemon
- Create module from type
- Assign devices
- Configure parameters
- Start/stop/delete
- Verify event log

## Dependencies

### Slint Framework
- UI rendering and event handling
- VecModel for data binding

### Tokio
- Async runtime
- Synchronization primitives (Mutex)

### Tonic/Prost
- gRPC client generation
- Protocol buffer handling

### UUID
- Module ID generation
- Event ID generation

## Future Enhancement Points

1. **Real-time Data Streaming**
   - Arrow Flight integration for bulk data
   - DataPreview component implementation

2. **Advanced Module Composition**
   - Nested modules
   - Module pipelines

3. **Experiment Plans**
   - Bluesky-style plan editor
   - Plan execution interface

4. **Performance Monitoring**
   - CPU/memory per module
   - Event rate graphing

5. **Module Marketplace**
   - Download/share modules
   - Custom implementations

## Related Documents

- [GUI_MODULES_PANEL.md](GUI_MODULES_PANEL.md) - UI component design
- [GUI_MODULES_IMPLEMENTATION.md](GUI_MODULES_IMPLEMENTATION.md) - Integration guide
- [MODULE_SERVICE_DESIGN.md](MODULE_SERVICE_DESIGN.md) - Backend service design
- [../../proto/daq.proto](../../proto/daq.proto) - gRPC service definitions
