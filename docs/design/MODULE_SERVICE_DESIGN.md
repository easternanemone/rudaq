# ModuleService Design (Revised)

## Overview

The ModuleService provides experiment modules with runtime instrument assignment, synthesizing patterns from PyMoDAQ, DynExp, Bluesky, Qudi, QCodes, and ScopeFoundry.

**Research-Informed Design Principles:**

1. **Observable Parameters**: All device values wrapped in `Parameter` objects with validation, units, and change signals (QCodes/ScopeFoundry pattern)
2. **Hardware/Logic Separation**: Strict three-layer model - Hardware Modules, Logic Modules, GUI Modules (Qudi pattern)
3. **Plan/Engine Separation**: Experiment logic as "Plans" that yield commands to a RunEngine (Bluesky pattern)
4. **Document Model**: Structured metadata streaming with `Start`, `Descriptor`, `Event`, `Stop` documents (Bluesky pattern)
5. **Capability Extension**: Passthrough commands to avoid "Least Common Denominator" problem
6. **Stage/Unstage Lifecycle**: Context managers for safe hardware setup/teardown
7. **Arrow Flight for Bulk Data**: gRPC for control, Arrow Flight for images/waveforms

## Architecture Comparison

| Aspect | Original Design | Revised Design |
|--------|-----------------|----------------|
| Device Access | Abstract roles only | Roles + Passthrough for device-specific features |
| Parameters | Raw key-value strings | Observable `Parameter` objects with units/validation |
| Data Streaming | Single gRPC stream | Document Model (metadata) + Arrow Flight (bulk) |
| Lifecycle | Start/Stop only | Stage/Unstage + Start/Pause/Resume/Stop |
| Experiment Logic | Module classes | Plan generators yielding commands |
| Configuration | Map strings | Typed parameters with descriptors |

## Critical Pattern: Observable Parameters

Instead of raw values, every device property is a `Parameter`:

```rust
/// Observable parameter with validation, units, and change notification
pub struct Parameter<T> {
    value: RwLock<T>,
    name: String,
    units: Option<String>,
    min: Option<T>,
    max: Option<T>,
    description: String,
    /// Subscribers notified on change
    on_change: broadcast::Sender<ParameterChange<T>>,
}

impl<T: Clone + PartialOrd> Parameter<T> {
    pub async fn set(&self, value: T) -> Result<()> {
        // Validate bounds
        if let Some(min) = &self.min {
            if value < *min { return Err(anyhow!("Below minimum")); }
        }
        // Set and notify
        *self.value.write().await = value.clone();
        self.on_change.send(ParameterChange { name: self.name.clone(), value })?;
        Ok(())
    }
}
```

**Why this matters**: Enables automatic GUI binding, validation at the edge, and change propagation without polling.

## Critical Pattern: Passthrough Commands

To avoid the "Least Common Denominator" problem where generic interfaces lose device-specific features:

```protobuf
// In HardwareService - escape hatch for device-specific commands
rpc ExecuteDeviceCommand(DeviceCommandRequest) returns (DeviceCommandResponse);

message DeviceCommandRequest {
  string device_id = 1;
  string command = 2;           // Device-specific command name
  map<string, string> args = 3; // Command arguments
}

message DeviceCommandResponse {
  bool success = 1;
  string error_message = 2;
  map<string, string> results = 3;
}
```

**Example**: A PVCAM camera's `set_clear_mode()` isn't in the generic `FrameProducer` interface, but can be called via passthrough.

## Critical Pattern: Bluesky Document Model

Data streaming should emit structured documents, not raw values:

```protobuf
// Document types for structured data streaming
enum DocumentType {
  DOC_START = 0;      // Experiment metadata, intent
  DOC_DESCRIPTOR = 1; // Schema for upcoming events
  DOC_EVENT = 2;      // Actual readings
  DOC_STOP = 3;       // Completion status, summary
}

message Document {
  DocumentType doc_type = 1;
  string uid = 2;              // Unique document ID
  uint64 timestamp_ns = 3;

  oneof payload {
    StartDocument start = 10;
    DescriptorDocument descriptor = 11;
    EventDocument event = 12;
    StopDocument stop = 13;
  }
}

message StartDocument {
  string plan_name = 1;
  map<string, string> metadata = 2;  // User-provided context
  repeated string hints = 3;         // Visualization hints
}

message DescriptorDocument {
  string start_uid = 1;              // Links to Start doc
  map<string, DataKey> data_keys = 2; // Schema for fields
}

message DataKey {
  string dtype = 1;      // "number", "array", "string"
  repeated int32 shape = 2;  // For arrays
  string units = 3;
  string source = 4;     // Device ID
}

message EventDocument {
  string descriptor_uid = 1;
  uint32 seq_num = 2;
  map<string, double> data = 3;      // Scalar values
  map<string, bytes> bulk_refs = 4;  // Arrow Flight refs for bulk data
}

message StopDocument {
  string start_uid = 1;
  string exit_status = 2;  // "success", "abort", "fail"
  string reason = 3;
  uint64 num_events = 4;
}
```

## Critical Pattern: Plan/Engine Separation

Don't embed experiment logic in modules. Use Plans:

```rust
/// A Plan is an async generator that yields Msgs to the RunEngine
pub trait Plan {
    async fn run(&mut self, ctx: &mut PlanContext) -> Result<()>;
}

/// Messages yielded by Plans
pub enum Msg {
    /// Read a device
    Read { device_id: String },
    /// Set a device value
    Set { device_id: String, value: f64 },
    /// Wait for a device to settle
    WaitSettled { device_id: String },
    /// Trigger a device
    Trigger { device_id: String },
    /// Emit metadata
    Declare { key: String, value: serde_json::Value },
    /// Sleep
    Sleep { duration: Duration },
    /// Checkpoint for pause/resume
    Checkpoint,
}

/// Example: A simple scan plan
pub struct LineScan {
    motor: String,
    detector: String,
    start: f64,
    stop: f64,
    num_points: u32,
}

impl Plan for LineScan {
    async fn run(&mut self, ctx: &mut PlanContext) -> Result<()> {
        let step = (self.stop - self.start) / (self.num_points - 1) as f64;

        for i in 0..self.num_points {
            let pos = self.start + step * i as f64;

            ctx.yield_msg(Msg::Set { device_id: self.motor.clone(), value: pos }).await?;
            ctx.yield_msg(Msg::WaitSettled { device_id: self.motor.clone() }).await?;
            ctx.yield_msg(Msg::Trigger { device_id: self.detector.clone() }).await?;
            ctx.yield_msg(Msg::Read { device_id: self.detector.clone() }).await?;
            ctx.yield_msg(Msg::Checkpoint).await?;  // Safe to pause here
        }
        Ok(())
    }
}
```

## Critical Pattern: Stage/Unstage Lifecycle

Every device should support safe setup/teardown:

```rust
#[async_trait]
pub trait Stageable {
    /// Prepare device for acquisition (open shutter, enable output, etc.)
    async fn stage(&self) -> Result<()>;

    /// Return device to safe state (close shutter, disable output, etc.)
    async fn unstage(&self) -> Result<()>;
}

// RunEngine calls these automatically
impl RunEngine {
    pub async fn run_plan(&mut self, plan: &mut dyn Plan, devices: &[&dyn Stageable]) -> Result<()> {
        // Stage all devices
        for device in devices {
            device.stage().await?;
        }

        // Run plan with automatic unstage on error/completion
        let result = plan.run(&mut self.context).await;

        // Always unstage
        for device in devices.iter().rev() {
            if let Err(e) = device.unstage().await {
                tracing::error!("Unstage error: {}", e);
            }
        }

        result
    }
}
```

## Bulk Data: Arrow Flight Instead of gRPC

For images and waveforms, don't serialize through gRPC:

```rust
// In daemon - start Arrow Flight server alongside gRPC
let flight_server = FlightDataServer::new(data_store.clone());
tokio::spawn(async move {
    flight_server.serve("[::]:50052").await
});

// In EventDocument, reference bulk data by Flight ticket
message EventDocument {
  // ... scalar data ...

  // For bulk data, provide Arrow Flight ticket
  map<string, FlightTicket> bulk_data = 10;
}

message FlightTicket {
  string endpoint = 1;  // "localhost:50052"
  bytes ticket = 2;     // Opaque ticket for DoGet
}
```

Client retrieves bulk data via Arrow Flight's `DoGet` RPC - zero-copy, efficient for multi-MB frames.

## Module Types

### 1. PowerMonitor (`power_monitor`)

**Purpose:** Monitor power levels with threshold alerts and statistics.

**Use Cases:**
- Laser power stability monitoring
- Threshold-based safety interlocks
- Power logging with statistics

**Roles:**
| Role ID | Display Name | Required Capability | Multiple? |
|---------|--------------|---------------------|-----------|
| `power_meter` | Power Meter | `readable` | No |

**Parameters:**
| Parameter | Type | Default | Units | Description |
|-----------|------|---------|-------|-------------|
| `sample_rate_hz` | float | 10.0 | Hz | Sampling rate |
| `low_threshold` | float | - | mW | Alert if below (optional) |
| `high_threshold` | float | - | mW | Alert if above (optional) |
| `averaging_window_s` | float | 1.0 | s | Window for statistics |
| `log_to_file` | bool | false | - | Enable file logging |

**Events:**
- `threshold_low` - Power dropped below low threshold
- `threshold_high` - Power exceeded high threshold
- `threshold_normal` - Power returned to normal range

**Data Types:**
- `power_reading` - Individual readings: `{value, units}`
- `statistics` - Computed stats: `{mean, std, min, max, count}`

### 2. DataLogger (`data_logger`)

**Purpose:** Record values from multiple devices to file.

**Use Cases:**
- Long-term data recording
- Multi-channel logging
- Triggered recording

**Roles:**
| Role ID | Display Name | Required Capability | Multiple? |
|---------|--------------|---------------------|-----------|
| `data_source` | Data Source | `readable` | Yes |
| `trigger_source` | Trigger (optional) | `triggerable` | No |

**Parameters:**
| Parameter | Type | Default | Units | Description |
|-----------|------|---------|-------|-------------|
| `sample_rate_hz` | float | 1.0 | Hz | Sampling rate |
| `output_format` | enum | csv | - | csv, hdf5, arrow |
| `output_path` | string | - | - | Output file path |
| `buffer_size` | int | 1000 | - | Buffer before flush |
| `triggered_mode` | bool | false | - | Wait for triggers |

**Events:**
- `recording_started` - Recording began
- `recording_stopped` - Recording ended
- `file_rotated` - New file started
- `buffer_flushed` - Data written to disk

**Data Types:**
- `buffer_status` - `{buffered_points, written_points, file_size}`

### 3. PositionTracker (`position_tracker`)

**Purpose:** Monitor position with limits and tracking.

**Use Cases:**
- Stage position monitoring
- Limit detection and alerts
- Position logging

**Roles:**
| Role ID | Display Name | Required Capability | Multiple? |
|---------|--------------|---------------------|-----------|
| `position_source` | Position Source | `movable` | No |

**Parameters:**
| Parameter | Type | Default | Units | Description |
|-----------|------|---------|-------|-------------|
| `poll_rate_hz` | float | 10.0 | Hz | Position polling rate |
| `soft_limit_low` | float | - | units | Low soft limit (optional) |
| `soft_limit_high` | float | - | units | High soft limit (optional) |
| `track_velocity` | bool | false | - | Compute velocity |

**Events:**
- `soft_limit_low` - Position below soft limit
- `soft_limit_high` - Position above soft limit
- `motion_started` - Device started moving
- `motion_stopped` - Device stopped moving

**Data Types:**
- `position` - `{position, is_moving, velocity}`

### 4. ExposureSequencer (`exposure_sequencer`)

**Purpose:** Coordinate camera exposure with stage motion.

**Use Cases:**
- Triggered image acquisition
- Time-lapse imaging
- Multi-position imaging

**Roles:**
| Role ID | Display Name | Required Capability | Multiple? |
|---------|--------------|---------------------|-----------|
| `camera` | Camera | `frame_producer` | No |
| `stage` | Stage (optional) | `movable` | No |
| `trigger` | Trigger (optional) | `triggerable` | No |

**Parameters:**
| Parameter | Type | Default | Units | Description |
|-----------|------|---------|-------|-------------|
| `exposure_ms` | float | 100.0 | ms | Exposure time |
| `frame_count` | int | 1 | - | Frames per sequence |
| `inter_frame_delay_ms` | float | 0 | ms | Delay between frames |
| `triggered_mode` | bool | false | - | External trigger mode |

**Events:**
- `sequence_started` - Sequence began
- `frame_acquired` - Frame captured
- `sequence_completed` - All frames acquired
- `sequence_aborted` - Sequence stopped early

**Data Types:**
- `frame_info` - `{frame_number, timestamp, exposure_ms}`

### 5. AlignmentAssist (`alignment_assist`)

**Purpose:** Assist with optical alignment using power feedback.

**Use Cases:**
- Beam alignment optimization
- Coupling efficiency monitoring
- Alignment drift detection

**Roles:**
| Role ID | Display Name | Required Capability | Multiple? |
|---------|--------------|---------------------|-----------|
| `power_meter` | Power Meter | `readable` | No |
| `x_stage` | X Stage (optional) | `movable` | No |
| `y_stage` | Y Stage (optional) | `movable` | No |

**Parameters:**
| Parameter | Type | Default | Units | Description |
|-----------|------|---------|-------|-------------|
| `target_power` | float | - | mW | Target power level |
| `tolerance_percent` | float | 5.0 | % | Acceptable deviation |
| `sample_rate_hz` | float | 10.0 | Hz | Sampling rate |
| `drift_window_s` | float | 60.0 | s | Window for drift detection |
| `drift_threshold` | float | 10.0 | % | Drift alert threshold |

**Events:**
- `alignment_optimal` - Power within tolerance of target
- `alignment_degraded` - Power drifted outside tolerance
- `drift_detected` - Significant drift over time window

**Data Types:**
- `alignment_status` - `{power, target, deviation_percent, aligned}`

## API Usage Examples

### Example 1: Power Monitoring with Alerts

```python
# 1. List available module types
types = client.ListModuleTypes()
# Returns: [PowerMonitor, DataLogger, PositionTracker, ...]

# 2. Create a PowerMonitor instance
response = client.CreateModule(
    type_id="power_monitor",
    instance_name="Laser Power Monitor"
)
module_id = response.module_id

# 3. Assign the power meter device
client.AssignDevice(
    module_id=module_id,
    role_id="power_meter",
    device_id="newport_1830c"
)

# 4. Configure thresholds
client.ConfigureModule(
    module_id=module_id,
    parameters={
        "sample_rate_hz": "20",
        "low_threshold": "50.0",
        "high_threshold": "150.0"
    }
)

# 5. Start monitoring
client.StartModule(module_id=module_id)

# 6. Stream events (threshold alerts)
for event in client.StreamModuleEvents(module_id=module_id):
    if event.event_type == "threshold_high":
        print(f"WARNING: Power too high! {event.data['value']} mW")
```

### Example 2: Multi-Channel Data Logger

```python
# Create data logger
response = client.CreateModule(
    type_id="data_logger",
    instance_name="Experiment Logger"
)
module_id = response.module_id

# Assign multiple data sources
client.AssignDevice(module_id, "data_source", "power_meter_1")
client.AssignDevice(module_id, "data_source", "power_meter_2")
client.AssignDevice(module_id, "data_source", "stage_position")

# Configure
client.ConfigureModule(module_id, {
    "sample_rate_hz": "1.0",
    "output_format": "hdf5",
    "output_path": "/data/experiment_001.h5"
})

# Start logging
client.StartModule(module_id)

# ... run experiment ...

# Stop and get summary
response = client.StopModule(module_id)
print(f"Logged {response.data_points_produced} points")
```

### Example 3: Position Monitoring with Soft Limits

```python
# Create position tracker
response = client.CreateModule(
    type_id="position_tracker",
    instance_name="Stage Monitor"
)
module_id = response.module_id

# Assign stage
client.AssignDevice(module_id, "position_source", "esp300_stage")

# Configure soft limits
client.ConfigureModule(module_id, {
    "soft_limit_low": "0.0",
    "soft_limit_high": "25.0",  # 25mm travel
    "track_velocity": "true"
})

# Start tracking
client.StartModule(module_id)

# Stream position data
for data in client.StreamModuleData(module_id, max_rate_hz=10):
    print(f"Position: {data.values['position']:.3f} mm")
```

## Implementation Notes

### Module Registration

Modules self-register via a `ModuleRegistry`:

```rust
pub trait Module: Send + Sync + 'static {
    fn type_info() -> ModuleTypeInfo;
    fn create(config: &ModuleConfig) -> Result<Box<dyn Module>>;
    async fn start(&mut self, ctx: ModuleContext) -> Result<()>;
    async fn pause(&mut self) -> Result<()>;
    async fn resume(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
}
```

### Device Access

Modules access devices through the `ModuleContext`:

```rust
impl ModuleContext {
    /// Get a device assigned to a role
    pub fn get_device(&self, role_id: &str) -> Option<&dyn AnyDevice>;

    /// Emit an event
    pub fn emit_event(&self, event: ModuleEvent);

    /// Emit a data point
    pub fn emit_data(&self, data: ModuleDataPoint);
}
```

### Thread Safety

- Each module runs in its own Tokio task
- Device access is coordinated via `Arc<Mutex<...>>`
- Events/data are sent through mpsc channels

## GUI Integration

The GUI Modules panel should:

1. **Module Browser**: List available module types with descriptions
2. **Instance List**: Show running modules with status indicators
3. **Assignment UI**: Drag-and-drop devices to module roles
4. **Configuration Panel**: Edit module parameters with validation
5. **Event Log**: Display recent module events
6. **Data Preview**: Real-time visualization of module data

## Future Extensions

1. **Custom Modules**: User-defined modules via Rhai scripts
2. **Module Chains**: Connect module outputs to other module inputs
3. **Presets**: Save/load module configurations
4. **Module Templates**: Pre-configured module setups for common experiments
