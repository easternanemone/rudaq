# Modules Panel Design - Slint GUI (bd-49l3)

## Overview

The Modules panel is a comprehensive GUI component for managing experiment modules in the rust-daq system. It provides interfaces for:
- Browsing available module types
- Creating and managing module instances
- Runtime device assignment to module roles
- Configuration parameter management
- Event logging and monitoring
- Real-time data visualization

**Architecture Philosophy**: Modular tab-based UI inspired by PyMoDAQ/DynExp with clear separation of concerns between module discovery, lifecycle management, device assignment, and monitoring.

## Design Requirements

### 1. Module Type Browser
**Purpose**: Allow users to discover and select available module types

**Features**:
- List all available module types with descriptions
- Display metadata: version, required/optional roles, parameter count
- Show module categories (e.g., "monitoring", "threshold")
- Single-click creation of new module instances from selected type

**Implementation**:
- Slint component: `ModuleTypeBrowser` in `modules_panel.slint`
- Data model: `ModuleTypeInfo` struct
- Callback: `module-type-selected(type-id)`
- Fetches types via `DaqClient::list_module_types()`

### 2. Module Instance List
**Purpose**: Show running module instances with lifecycle management

**Features**:
- Display status indicator (color-coded: running=green, paused=orange, error=red)
- Show module name, type, and role fill status
- Error messages when applicable
- Uptime tracking for running modules
- Create/delete operations

**Implementation**:
- Slint component: `ModuleInstanceList`
- Data model: `ModuleInstance` struct
- State display: "created", "configured", "running", "paused", "stopped", "error"
- Fetches instances via `DaqClient::list_modules()`

### 3. Device Assignment UI
**Purpose**: Assign compatible devices to module roles at runtime

**Features**:
- Display required and optional roles
- Show required device capability (readable, movable, frame-producer)
- ComboBox selection when unassigned
- Visual feedback for assigned devices
- Support multiple devices per role (when allowed)
- Remove/reassign capability

**Implementation**:
- Slint component: `DeviceAssignmentPanel`
- Data models: `ModuleRole` struct
- Role types: defined in ModuleService proto
- Validates device capabilities before assignment
- Callbacks: `assign-device()`, `unassign-device()`

**Example Roles**:
```
Power Monitor module:
  - Required: "power_meter" (readable capability)

Data Logger module:
  - Required: "motion_stage" (movable capability)
  - Required: "detector" (frame-producer capability)
  - Optional: "reference_signal" (readable capability)
```

### 4. Configuration Panel
**Purpose**: Manage module parameters with validation

**Features**:
- Display all module parameters with descriptions
- Type-specific input controls:
  - Float/Int: LineEdit with min/max bounds
  - String: LineEdit
  - Bool: CheckBox
  - Enum: ComboBox
- Units display
- Required parameter indication (red asterisk)
- Enable/disable during running (prevent modification while running)
- Real-time validation feedback

**Implementation**:
- Slint component: `ConfigurationPanel`
- Data model: `ModuleParameter` struct
- Parameter types: float, int, string, bool, enum
- Callbacks: `parameter-changed(param-id, value)`
- Disables editing when module is running

**Example Parameters**:
```
Power Monitor:
  - "threshold" (float): Alert threshold in mW, default=10, range=[0, 1000]
  - "averaging_window" (int): Points to average, default=10, range=[1, 1000]
  - "alert_enabled" (bool): Enable threshold alerts, default=true

Data Logger:
  - "file_format" (enum): CSV, HDF5, NetCDF
  - "sample_rate_hz" (float): Acquisition rate
  - "buffer_size" (int): In-memory buffer size
```

### 5. Event Log
**Purpose**: Display module events and status changes

**Features**:
- Time-stamped event entries
- Severity color-coding (info=green, warning=orange, error=red)
- Event type classification
- Event message text with word wrapping
- Configurable max event count (default 100)
- Auto-scrolling to most recent events

**Implementation**:
- Slint component: `EventLog`
- Data model: `ModuleEvent` struct
- Severity levels: "info", "warning", "error"
- Formats time as "MM:SS" (uptime format)
- Streams via `DaqClient::stream_module_events()`

**Example Events**:
```
[00:15] info    ModuleCreated       "power_monitor instance created"
[00:16] info    DeviceAssigned      "device power-meter assigned to power_meter role"
[00:17] info    ConfiguringStarted  "Validating configuration..."
[00:18] info    ReadyToStart        "Module ready: all roles assigned, parameters valid"
[00:19] info    ModuleStarted       "Starting power_monitor execution"
[01:22] warning ThresholdExceeded   "Power exceeded threshold: 125mW"
```

### 6. Data Preview
**Purpose**: Real-time visualization placeholder

**Features**:
- Shows data acquisition status
- Placeholder for charts/graphs (future enhancement)
- Statistics: points captured, last update time
- Contextual messages based on module state
- Ready for Arrow Flight bulk data integration

**Implementation**:
- Slint component: `DataPreview`
- Placeholder implementation for Phase 2
- Would integrate with Arrow Flight for image/waveform data
- Shows acquisition metadata

## File Structure

```
gui/
├── ui/
│   ├── main.slint                    # Main window (import modules_panel)
│   └── modules_panel.slint           # NEW: Modules panel UI (this document)
│
└── src/
    ├── main.rs                       # Main app (integrate ModulesPanel)
    ├── grpc_client.rs                # Base gRPC client
    └── modules_client.rs             # NEW: ModuleService client bindings
```

## Integration Points

### 1. Add to Main Window (gui/ui/main.slint)

```slint
// In MainWindow Tab widget:
Tab {
    title: "Modules";
    ModulesPanel {
        available-module-types: root.module-types;
        module-instances: root.modules;
        available-devices: root.device-list;
        current-module-roles: root.module-roles;
        current-module-parameters: root.module-parameters;
        current-module-events: root.module-events;
        selected-module-id <=> root.selected-module-id;
        selected-module-running <=> root.selected-module-running;

        // Callbacks wired to backend
        list-module-types => { root.load-module-types(); }
        // ... more callbacks
    }
}
```

### 2. Update gui/src/main.rs

Add these to AppState:
```rust
struct AppState {
    // ... existing fields
    modules: Vec<UiModuleInstance>,
    module_types: Vec<UiModuleType>,
    current_module_id: Option<String>,
    current_module_roles: Vec<UiModuleRole>,
    current_module_parameters: Vec<UiModuleParameter>,
    current_module_events: Vec<UiModuleEvent>,
    module_event_stream_handle: Option<tokio::task::JoinHandle<()>>,
}
```

Add callback handlers:
```rust
ui.on_list_module_types(|_| {
    // Fetch module types from ModuleService
});

ui.on_create_module(move |type_id| {
    // Create new module instance
});

// ... more callback implementations
```

### 3. Update gui/src/grpc_client.rs

Integrate ModuleService proto client:
```rust
// In DaqClient impl block:

pub async fn list_module_types(&self) -> Result<Vec<UiModuleType>> {
    let request = ListModuleTypesRequest::default();
    let response = self.module_service_client.list_module_types(request).await?;

    Ok(response.into_inner().module_types.into_iter().map(|m| {
        UiModuleType {
            type_id: m.type_id,
            // ... map other fields
        }
    }).collect())
}
```

## Slint Component Architecture

### Component Hierarchy

```
ModulesPanel (root)
├── Tab: "Modules"
│   ├── ModuleTypeBrowser (left sidebar)
│   └── VerticalBox (right panel)
│       ├── ModuleInstanceList
│       └── ModuleControl (start/pause/stop/delete)
│
├── Tab: "Assignment"
│   └── DeviceAssignmentPanel (conditional on selection)
│
├── Tab: "Configuration"
│   └── ConfigurationPanel (conditional on selection)
│
└── Tab: "Data"
    ├── DataPreview (left)
    └── EventLog (right)
```

### Data Flow

```
ModulesPanel
  ├─ [list_module_types] ──> DaqClient ──> ModuleService.ListModuleTypes ──> [available_module_types]
  ├─ [list_modules] ──────> DaqClient ──> ModuleService.ListModules ────────> [module_instances]
  ├─ [create_module] ─────> DaqClient ──> ModuleService.CreateModule ───────> new instance
  ├─ [configure_module] ──> DaqClient ──> ModuleService.ConfigureModule ───> parameter updated
  ├─ [assign_device] ─────> DaqClient ──> ModuleService.AssignDevice ──────> role filled
  ├─ [start_module] ──────> DaqClient ──> ModuleService.StartModule ──────> state changed
  └─ [stream_events] ─────> DaqClient ──> ModuleService.StreamModuleEvents > [current_module_events]
```

## State Management

### Module Instance States

```
ModuleCreated
    ↓
Configured (all required roles assigned, parameters set)
    ↓
    ├──> Running ──> Paused ──> Running ──> Stopped
    │
    └──> Error (invalid configuration, device failure)
         ↓
         Stopped
```

### UI State Requirements

1. **Module Selection**: Track currently selected module for detailed view
2. **Role Assignment**: Show only unassigned roles or current assignments
3. **Parameter Editing**: Disable during running state
4. **Event Filtering**: Optionally filter by severity/type (future enhancement)

## Design Patterns

### 1. Lazy Loading
- Module types loaded on first tab activation
- Detailed module info (parameters, roles) fetched on selection
- Event stream started only for selected module

### 2. Progressive Disclosure
- Brief module summary in list view
- Detailed info in assignment/configuration tabs
- Events shown in real-time data tab

### 3. Validation Feedback
- Parameter bounds shown in configuration
- Role requirements shown in assignment
- Ready-to-start indicator in module list
- Error messages in event log and module header

### 4. Visual Hierarchy
- Status color-coding (consistent across panels)
- Font weight for important info (module name, state)
- Muted colors for secondary info (timestamps, descriptions)
- Icon/badge indicators (required params, multiple roles allowed)

## Future Enhancements

### Phase 2
- [ ] Real-time data visualization (DataPreview component)
- [ ] Arrow Flight integration for bulk data transfer
- [ ] Module composition (nested modules)
- [ ] Experiment plan editor (Bluesky-style plans)

### Phase 3
- [ ] Saved module configurations
- [ ] Module templates
- [ ] Batch operations (start/stop multiple modules)
- [ ] Advanced event filtering and search
- [ ] Performance monitoring (CPU, memory per module)

### Phase 4
- [ ] Module marketplace (share/download)
- [ ] Custom parameter widgets
- [ ] Live parameter change notifications
- [ ] Device emulation/simulation mode

## Testing Strategy

### Unit Tests (Rust)
- `modules_client.rs`: Mock gRPC responses
- Parameter validation logic
- State transition logic

### Integration Tests
- GUI component rendering with mock data
- Callback invocation and data flow
- Event log updates and clearing

### User Acceptance Tests
- Create/delete module workflows
- Device assignment error handling
- Configuration parameter bounds
- Module state transitions

## Performance Considerations

1. **Event Log Scaling**: Cap events at 100 (configurable) to prevent UI lag
2. **Device List Filtering**: Pre-filter available devices by required capability
3. **Lazy Module Type Loading**: Fetch full info only on demand
4. **Event Stream Backpressure**: Drop oldest events if buffer fills

## Accessibility

- Color-coded status not sole indicator (also text labels)
- Adequate contrast ratios (WCAG AA compliance)
- Keyboard navigation for all controls
- Tooltip descriptions for icons/abbreviations
- Screen reader friendly labels

## Error Handling

- Network errors: Show in status with retry option
- Invalid parameters: Display in event log with suggestion
- Device assignment failures: Clear error message in event log
- Module state conflicts: Prevent invalid transitions in UI

## Related Documents

- [MODULE_SERVICE_DESIGN.md](MODULE_SERVICE_DESIGN.md) - Backend gRPC API design
- [proto/daq.proto](../../proto/daq.proto) - Protocol Buffer definitions
- [ARCHITECTURAL_FLAW_ANALYSIS.md](ARCHITECTURAL_FLAW_ANALYSIS.md) - V5 headless architecture
