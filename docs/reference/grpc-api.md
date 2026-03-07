# gRPC API Reference

This document provides comprehensive reference for the rust-daq gRPC API. It covers all services, message types, and usage patterns.

## Overview

The rust-daq gRPC API provides remote control over the DAQ system with multiple specialized services:

- **HardwareService** - Direct device control (motion, readouts, streaming) - `hardware.proto`
- **ControlService** - Script upload and execution - `daq.proto`
- **ScanService** - Coordinated multi-axis scanning (DEPRECATED - use RunEngineService) - `experiment.proto`
- **RunEngineService** - Bluesky-style plan execution with pause/resume - `experiment.proto`
- **ModuleService** - Hardware-agnostic experiment modules - `experiment.proto`
- **StorageService** - HDF5 data recording and export - `storage.proto`
- **PresetService** - Device configuration snapshots - `hardware.proto`
- **PluginService** - Runtime plugin/driver management - `storage.proto`
- **HealthService** - System health monitoring (custom service) - `storage.proto`
- **NIDAQService** - National Instruments DAQ devices - `ni_daq.proto`

**Proto Files (6 total):**
- `crates/protocol/proto/daq.proto` - Core control services (ControlService)
- `crates/protocol/proto/experiment.proto` - RunEngine, Scan, and Module services
- `crates/protocol/proto/hardware.proto` - Hardware device control and presets
- `crates/protocol/proto/health.proto` - Standard gRPC health check service
- `crates/protocol/proto/ni_daq.proto` - NI-DAQ specific services
- `crates/protocol/proto/storage.proto` - Data storage, plugin management, and custom health services

## Connection

Connect to the gRPC server at the configured address and port.

**Default Configuration:**
- Address: `0.0.0.0` (all interfaces)
- Port: `50051`
- TLS: Not enabled by default

**Client Connection Examples:**

```rust
use server::grpc::proto::hardware_service_client::HardwareServiceClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = HardwareServiceClient::connect("http://localhost:50051").await?;
    Ok(())
}
```

```python
import grpc
from protocol.daq_pb2_grpc import HardwareServiceStub

channel = grpc.aio.secure_channel(
    "localhost:50051",
    grpc.ssl_channel_credentials()
)
client = HardwareServiceStub(channel)
```

## Authentication

By default, authentication is disabled. To enable token-based authentication, configure in `config/config.v4.toml`:

```toml
[grpc]
auth_enabled = true
```

When enabled, include an authorization token in RPC metadata:

```rust
let mut request = ListDevicesRequest { .. };
let metadata = tonic::metadata::MetadataValue::from_str("Bearer YOUR_TOKEN")?;
request.metadata_mut().insert("authorization", metadata);
```

## Services

### HardwareService

Direct control of physical devices. Devices have capabilities that determine available operations.

#### Device Discovery

**ListDevices** - Get all registered devices
```proto
rpc ListDevices(ListDevicesRequest) returns (ListDevicesResponse);
```

**Request:**
```proto
message ListDevicesRequest {
  optional string capability_filter = 1; // e.g., "movable", "readable"
}
```

**Response:**
```proto
message ListDevicesResponse {
  repeated DeviceInfo devices = 1;
  repeated RegistrationFailure registration_failures = 2;
}

message DeviceInfo {
  string id = 1;
  string name = 2;
  string driver_type = 3;
  DeviceCategory category = 4;
  DeviceMetadata metadata = 20;
  repeated string capabilities = 100; // e.g., "movable", "readable"
}

message DeviceMetadata {
  optional string position_units = 1;      // For Movable devices
  optional double min_position = 2;
  optional double max_position = 3;
  optional string reading_units = 4;       // For Readable devices
  optional uint32 frame_width = 10;        // For FrameProducer devices
  optional uint32 frame_height = 11;
  optional uint32 bits_per_pixel = 12;
  optional double min_exposure_ms = 20;    // For ExposureControl devices
  optional double max_exposure_ms = 21;
  optional double min_wavelength_nm = 30;  // For WavelengthTunable devices
  optional double max_wavelength_nm = 31;
}
```

**Example:**
```rust
let request = ListDevicesRequest {
    capability_filter: Some("movable".to_string()),
};
let response = client.list_devices(request).await?;
for device in response.get_ref().devices {
    println!("{}: {} ({:?})", device.id, device.name, device.capabilities);
}
```

**GetDeviceState** - Query current device state
```proto
rpc GetDeviceState(DeviceStateRequest) returns (DeviceStateResponse);
```

**Request/Response:**
```proto
message DeviceStateRequest {
  string device_id = 1;
}

message DeviceStateResponse {
  string device_id = 1;
  bool online = 2;

  // Current values (populated based on capabilities)
  optional double position = 10;        // For Movable devices
  optional double last_reading = 11;    // For Readable devices
  optional bool armed = 12;             // For Triggerable devices
  optional bool streaming = 13;         // For FrameProducer devices
  optional double exposure_ms = 14;     // For ExposureControl devices
}
```

**Example:**
```rust
let response = client.get_device_state(DeviceStateRequest {
    device_id: "rotator_2".to_string(),
}).await?;

println!("Device online: {}", response.get_ref().online);
if let Some(position) = response.get_ref().position {
    println!("Current position: {}", position);
}
```

#### Motion Control

**MoveAbsolute** - Move device to absolute position
```proto
rpc MoveAbsolute(MoveRequest) returns (MoveResponse);
```

**Request:**
```proto
message MoveRequest {
  string device_id = 1;
  double value = 2;                    // Target position or distance
  optional bool wait_for_completion = 3;
  optional uint32 timeout_ms = 4;
}
```

**Response:**
```proto
message MoveResponse {
  bool success = 1;
  string error_message = 2;
  double final_position = 3;
  optional bool settled = 4;           // True if reached target
}
```

**Example:**
```rust
let request = MoveRequest {
    device_id: "rotator_2".to_string(),
    value: 45.0,
    wait_for_completion: Some(true),
    timeout_ms: Some(5000),
};
let response = client.move_absolute(request).await?;
assert!(response.get_ref().success);
println!("Final position: {}", response.get_ref().final_position);
```

**MoveRelative** - Move device by relative offset
```proto
rpc MoveRelative(MoveRequest) returns (MoveResponse);
```

Same request/response structure as MoveAbsolute, but `value` is relative offset.

**StopMotion** - Stop ongoing motion
```proto
rpc StopMotion(StopMotionRequest) returns (StopMotionResponse);
```

**WaitSettled** - Block until motion completes
```proto
rpc WaitSettled(WaitSettledRequest) returns (WaitSettledResponse);
```

**StreamPosition** - Stream position updates during motion
```proto
rpc StreamPosition(StreamPositionRequest) returns (stream PositionUpdate);
```

#### Scalar Readouts

**ReadValue** - Read single measurement from device
```proto
rpc ReadValue(ReadValueRequest) returns (ReadValueResponse);
```

**Request/Response:**
```proto
message ReadValueRequest {
  string device_id = 1;
}

message ReadValueResponse {
  bool success = 1;
  string error_message = 2;
  double value = 3;
  string units = 4;                   // e.g., "W", "mW", "V"
  uint64 timestamp_ns = 5;
}
```

**Example:**
```rust
let request = ReadValueRequest {
    device_id: "power_meter".to_string(),
};
let response = client.read_value(request).await?;
println!("Power: {} {}", response.get_ref().value, response.get_ref().units);
```

**Critical:** The `units` field is provided by the device. Clients must interpret values according to units:
- Newport 1830-C returns Watts (W) - multiply by 1000 for milliwatts
- GUI automatically normalizes power measurements to mW

**StreamValues** - Continuous reading stream
```proto
rpc StreamValues(StreamValuesRequest) returns (stream ValueUpdate);
```

#### Trigger Control

**Arm** - Prepare device for triggering
```proto
rpc Arm(ArmRequest) returns (ArmResponse);
```

**Trigger** - Send trigger pulse
```proto
rpc Trigger(TriggerRequest) returns (TriggerResponse);
```

#### Exposure Control

**SetExposure** - Set camera exposure time
```proto
rpc SetExposure(SetExposureRequest) returns (SetExposureResponse);

message SetExposureRequest {
  string device_id = 1;
  double exposure_ms = 2;              // Exposure in milliseconds
}

message SetExposureResponse {
  bool success = 1;
  string error_message = 2;
  double actual_exposure_ms = 3;       // May differ from requested
}
```

**GetExposure** - Query current exposure
```proto
rpc GetExposure(GetExposureRequest) returns (GetExposureResponse);
```

#### Laser Control

**SetShutter** - Open/close laser shutter
```proto
rpc SetShutter(SetShutterRequest) returns (SetShutterResponse);

message SetShutterRequest {
  string device_id = 1;
  bool open = 2;                       // true = open, false = closed
}

message SetShutterResponse {
  bool success = 1;
  string error_message = 2;
  bool is_open = 3;
}
```

**GetShutter** - Query shutter state
```proto
rpc GetShutter(GetShutterRequest) returns (GetShutterResponse);
```

**SetWavelength** - Set laser wavelength (for tunable lasers)
```proto
rpc SetWavelength(SetWavelengthRequest) returns (SetWavelengthResponse);

message SetWavelengthRequest {
  string device_id = 1;
  double wavelength_nm = 2;
}

message SetWavelengthResponse {
  bool success = 1;
  string error_message = 2;
  double actual_wavelength_nm = 3;
}
```

**GetWavelength** - Query current wavelength
```proto
rpc GetWavelength(GetWavelengthRequest) returns (GetWavelengthResponse);
```

**SetEmission** - Enable/disable laser emission
```proto
rpc SetEmission(SetEmissionRequest) returns (SetEmissionResponse);

message SetEmissionRequest {
  string device_id = 1;
  bool enabled = 2;                    // true = on, false = off
}

message SetEmissionResponse {
  bool success = 1;
  string error_message = 2;
  bool is_enabled = 3;
}
```

**GetEmission** - Query emission state
```proto
rpc GetEmission(GetEmissionRequest) returns (GetEmissionResponse);
```

#### Frame Streaming

**StartStream** - Begin streaming camera frames
```proto
rpc StartStream(StartStreamRequest) returns (StartStreamResponse);

message StartStreamRequest {
  string device_id = 1;
  optional uint32 frame_count = 2;     // 0 = continuous
}
```

**StreamFrames** - Server streaming camera frame data (server streams frames to client)
```proto
rpc StreamFrames(StreamFramesRequest) returns (stream FrameData);

message StreamFramesRequest {
  string device_id = 1;
  uint32 max_fps = 2;                  // Rate limit (0 = unlimited)
  StreamQuality quality = 3;           // FULL, PREVIEW (2x2), or FAST (4x4)
}

enum StreamQuality {
  STREAM_QUALITY_FULL = 0;             // Full resolution
  STREAM_QUALITY_PREVIEW = 1;          // 2x2 binning (~4x smaller)
  STREAM_QUALITY_FAST = 2;             // 4x4 binning (~16x smaller)
}
```

**FrameData** - Single frame of image data
```proto
message FrameData {
  string device_id = 1;
  uint32 width = 2;
  uint32 height = 3;
  uint32 bit_depth = 4;                // 8, 12, or 16
  bytes data = 5;                      // Raw pixel data (row-major, little-endian)
  uint64 frame_number = 6;
  uint64 timestamp_ns = 7;             // Nanoseconds since epoch
  optional double exposure_ms = 8;

  // ROI offset in sensor coordinates
  uint32 roi_x = 10;                   // X offset (0 = left edge)
  uint32 roi_y = 11;                   // Y offset (0 = top edge)

  // Extended metadata
  optional double temperature_c = 20;
  optional string gain_mode = 21;
  optional string readout_speed = 22;
  optional string trigger_mode = 23;
  optional uint32 binning_x = 24;
  optional uint32 binning_y = 25;
  map<string, string> metadata = 30;
  optional StreamingMetrics metrics = 40;

  // Compression
  CompressionType compression = 50;
  uint32 uncompressed_size = 51;
}

message StreamingMetrics {
  double current_fps = 1;
  uint64 frames_sent = 2;
  uint64 frames_dropped = 3;
  double avg_latency_ms = 4;
}
```

**Example:**
```rust
let request = StreamFramesRequest {
    device_id: "camera0".to_string(),
    max_fps: 30,
    quality: StreamQuality::Preview.into(),
};

let mut stream = client.stream_frames(request).await?;
while let Some(frame) = stream.message().await? {
    println!("Frame {}: {}x{} ({} bytes)",
        frame.frame_number, frame.width, frame.height, frame.data.len());

    // Decompress if needed
    if frame.compression != CompressionType::None as i32 {
        // Use decompression library
    }
}
```

**StopStream** - End frame streaming
```proto
rpc StopStream(StopStreamRequest) returns (StopStreamResponse);
```

#### Parameters and Observables

**ListParameters** - Get device parameters
```proto
rpc ListParameters(ListParametersRequest) returns (ListParametersResponse);

message ListParametersRequest {
  string device_id = 1;
}

message ListParametersResponse {
  repeated ParameterDescriptor parameters = 1;
}

message ParameterDescriptor {
  string device_id = 1;
  string name = 2;                     // e.g., "exposure_ms"
  string description = 3;
  string dtype = 4;                    // "float", "int", "bool", "enum"
  string units = 5;                    // e.g., "ms", "nm"
  bool readable = 6;
  bool writable = 7;
  optional double min_value = 10;
  optional double max_value = 11;
  repeated string enum_values = 12;
  optional ParameterMetadata metadata = 13;
}
```

**GetParameter** - Read specific parameter
```proto
rpc GetParameter(GetParameterRequest) returns (ParameterValue);

message ParameterValue {
  string device_id = 1;
  string name = 2;
  string value = 3;                    // Value as string
  string units = 4;
  uint64 timestamp_ns = 5;
}
```

**SetParameter** - Write parameter value
```proto
rpc SetParameter(SetParameterRequest) returns (SetParameterResponse);

message SetParameterRequest {
  string device_id = 1;
  string parameter_name = 2;
  string value = 3;                    // Value as string
}

message SetParameterResponse {
  bool success = 1;
  string error_message = 2;
  string actual_value = 3;             // After setting
}
```

**StreamParameterChanges** - Monitor parameter changes
```proto
rpc StreamParameterChanges(StreamParameterChangesRequest)
    returns (stream ParameterChange);
```

**StreamObservables** - Stream observable values with deadband filtering
```proto
rpc StreamObservables(StreamObservablesRequest)
    returns (stream ObservableValue);

message StreamObservablesRequest {
  repeated string device_ids = 1;      // Empty = all
  repeated string observable_names = 2; // Empty = all
  uint32 sample_rate_hz = 3;           // 0 = as fast as available
  double deadband = 4;                 // Min change to trigger update
}

message ObservableValue {
  string device_id = 1;
  string observable_name = 2;
  double value = 3;
  string units = 4;
  uint64 timestamp_ns = 5;
}
```

#### Device Lifecycle

**StageDevice** - Allocate resources for device
```proto
rpc StageDevice(StageDeviceRequest) returns (StageDeviceResponse);
```

**UnstageDevice** - Release device resources
```proto
rpc UnstageDevice(UnstageDeviceRequest) returns (UnstageDeviceResponse);
```

#### State Synchronization

**SubscribeDeviceState** - Real-time device state updates
```proto
rpc SubscribeDeviceState(DeviceStateSubscribeRequest)
    returns (stream DeviceStateUpdate);

message DeviceStateSubscribeRequest {
  repeated string device_ids = 1;      // Empty = all
  uint32 max_rate_hz = 2;
  uint64 last_seen_version = 3;        // Resume from checkpoint
  bool include_snapshot = 4;           // Get initial snapshot
}

message DeviceStateUpdate {
  string device_id = 1;
  uint64 timestamp_ns = 2;
  uint64 version = 3;                  // Monotonic per-device version
  bool is_snapshot = 4;
  map<string, string> fields_json = 5; // Changed fields as JSON
}
```

### ControlService

Script management and execution.

**UploadScript** - Upload a Rhai script
```proto
rpc UploadScript(UploadRequest) returns (UploadResponse);

message UploadRequest {
  string script_content = 1;
  string name = 2;
  map<string, string> metadata = 3;
}

message UploadResponse {
  string script_id = 1;
  bool success = 2;
  string error_message = 3;
}
```

**StartScript** - Execute uploaded script
```proto
rpc StartScript(StartRequest) returns (StartResponse);

message StartRequest {
  string script_id = 1;
  map<string, string> parameters = 2;
}

message StartResponse {
  bool started = 1;
  string execution_id = 2;
}
```

**GetScriptStatus** - Query execution status
```proto
rpc GetScriptStatus(StatusRequest) returns (ScriptStatus);

message StatusRequest {
  string execution_id = 1;
}

message ScriptStatus {
  string execution_id = 1;
  string state = 2;                    // PENDING, RUNNING, COMPLETED, ERROR, STOPPED
  string error_message = 3;
  uint64 start_time_ns = 4;
  uint64 end_time_ns = 5;
  string script_id = 6;
  uint32 progress_percent = 7;
  string current_line = 8;
}
```

**StopScript** - Stop running script
```proto
rpc StopScript(StopRequest) returns (StopResponse);

message StopRequest {
  string execution_id = 1;
  bool force = 2;                      // Force immediate kill
}
```

**StreamStatus** - Monitor system status
```proto
rpc StreamStatus(StatusRequest) returns (stream SystemStatus);

message SystemStatus {
  string current_state = 1;
  double current_memory_usage_mb = 2;
  map<string, double> live_values = 3;
  uint64 timestamp_ns = 4;
}
```

### RunEngineService

Bluesky-style plan execution with structured documents.

**QueuePlan** - Queue experiment plan
```proto
rpc QueuePlan(QueuePlanRequest) returns (QueuePlanResponse);

message QueuePlanRequest {
  string plan_type = 1;                // e.g., "line_scan", "grid_scan"
  map<string, string> parameters = 2;
  map<string, string> device_mapping = 3; // role_id -> device_id
  map<string, string> metadata = 4;
}

message QueuePlanResponse {
  bool success = 1;
  string run_uid = 2;                  // Unique run identifier
  string error_message = 3;
  uint32 queue_position = 4;
}
```

**StartEngine** - Begin executing queued plans
```proto
rpc StartEngine(StartEngineRequest) returns (StartEngineResponse);
```

**PauseEngine** - Pause at next checkpoint
```proto
rpc PauseEngine(PauseEngineRequest) returns (PauseEngineResponse);
```

**ResumeEngine** - Resume from pause
```proto
rpc ResumeEngine(ResumeEngineRequest) returns (ResumeEngineResponse);
```

**AbortPlan** - Abort current plan gracefully
```proto
rpc AbortPlan(AbortPlanRequest) returns (AbortPlanResponse);
```

**HaltEngine** - Emergency stop
```proto
rpc HaltEngine(HaltEngineRequest) returns (HaltEngineResponse);
```

**StreamDocuments** - Receive structured experiment data
```proto
rpc StreamDocuments(StreamDocumentsRequest) returns (stream Document);

enum DocumentType {
  DOC_START = 1;          // Experiment metadata
  DOC_DESCRIPTOR = 2;     // Data schema
  DOC_EVENT = 3;          // Measurements
  DOC_STOP = 4;           // Completion status
}

message Document {
  DocumentType doc_type = 1;
  string uid = 2;
  uint64 timestamp_ns = 3;

  oneof payload {
    StartDocument start = 10;
    DescriptorDocument descriptor = 11;
    EventDocument event = 12;
    StopDocument stop = 13;
  }
}

message EventDocument {
  string descriptor_uid = 1;
  uint32 seq_num = 2;
  uint64 time_ns = 3;
  map<string, double> data = 10;           // Scalar data
  map<string, bytes> arrays = 31;          // Small arrays (packed)
  map<string, string> metadata = 30;       // Status, enums
}
```

### StorageService

HDF5 data recording and export.

**ConfigureStorage** - Set up storage parameters
```proto
rpc ConfigureStorage(ConfigureStorageRequest)
    returns (ConfigureStorageResponse);

message ConfigureStorageRequest {
  string output_directory = 1;
  HDF5Config hdf5_config = 2;
  optional uint32 flush_interval_ms = 3;
  optional uint32 max_buffer_mb = 4;
}

message HDF5Config {
  string compression = 1;              // "none", "gzip", "lz4", "zstd"
  optional uint32 compression_level = 2;
  optional uint32 chunk_size = 3;
  optional string filename_pattern = 4;
  bool include_timestamps = 5;
  bool include_device_metadata = 6;
}
```

**StartRecording** - Begin recording to HDF5
```proto
rpc StartRecording(StartRecordingRequest) returns (StartRecordingResponse);

message StartRecordingRequest {
  string name = 1;
  map<string, string> metadata = 2;
  optional HDF5Config config_override = 3;
  optional string scan_id = 10;
  optional string run_uid = 11;
}

message StartRecordingResponse {
  bool success = 1;
  string error_message = 2;
  string recording_id = 3;
  string output_path = 4;
}
```

**StopRecording** - End recording and finalize file
```proto
rpc StopRecording(StopRecordingRequest) returns (StopRecordingResponse);

message StopRecordingResponse {
  bool success = 1;
  string error_message = 2;
  string acquisition_id = 3;
  string output_path = 4;
  uint64 file_size_bytes = 5;
  uint64 total_samples = 6;
  uint64 duration_ns = 7;
}
```

**ListAcquisitions** - Query saved files
```proto
rpc ListAcquisitions(ListAcquisitionsRequest)
    returns (ListAcquisitionsResponse);

message ListAcquisitionsRequest {
  optional string name_pattern = 1;
  optional uint64 after_timestamp_ns = 2;
  optional uint64 before_timestamp_ns = 3;
  optional uint32 limit = 4;
  optional uint32 offset = 5;
}
```

**GetAcquisitionInfo** - Get file metadata
```proto
rpc GetAcquisitionInfo(GetAcquisitionInfoRequest)
    returns (AcquisitionInfo);

message AcquisitionInfo {
  string acquisition_id = 1;
  string name = 2;
  string file_path = 3;
  uint64 file_size_bytes = 4;
  uint64 created_at_ns = 5;
  uint64 duration_ns = 6;
  repeated DatasetInfo datasets = 10;
  map<string, string> metadata = 20;
  optional string scan_id = 30;
  optional string run_uid = 31;
  HDF5Structure structure = 40;
}

message DatasetInfo {
  string name = 1;                     // Dataset path
  string dtype = 2;                    // "float64", "uint16", etc.
  repeated uint64 shape = 3;
  string units = 4;
  string device_id = 5;
  uint64 sample_count = 6;
  optional double min_value = 7;
  optional double max_value = 8;
}
```

### PresetService

Device configuration snapshots.

**SavePreset** - Save device configuration
```proto
rpc SavePreset(SavePresetRequest) returns (SavePresetResponse);

message SavePresetRequest {
  Preset preset = 1;
  bool overwrite = 2;
}

message Preset {
  PresetMetadata meta = 1;
  map<string, string> device_configs_json = 2; // device_id -> JSON config
  string scan_template_json = 3;
}
```

**LoadPreset** - Restore device configuration
```proto
rpc LoadPreset(LoadPresetRequest) returns (LoadPresetResponse);
```

**ListPresets** - Get available presets
```proto
rpc ListPresets(ListPresetsRequest) returns (ListPresetsResponse);
```

### PluginService

Runtime plugin management for YAML-defined drivers.

**ListPlugins** - Get available plugin types
```proto
rpc ListPlugins(ListPluginsRequest) returns (ListPluginsResponse);

message ListPluginsResponse {
  repeated PluginSummary plugins = 1;
}

message PluginSummary {
  string plugin_id = 1;
  string name = 2;
  string version = 3;
  string driver_type = 4;              // "serial_scpi", "tcp_scpi", etc.
}
```

**SpawnPlugin** - Create plugin instance
```proto
rpc SpawnPlugin(SpawnPluginRequest) returns (SpawnPluginResponse);

message SpawnPluginRequest {
  string plugin_id = 1;
  string address = 2;                  // Port path or "host:port"
  optional string instance_name = 3;
  bool mock_mode = 4;
}

message SpawnPluginResponse {
  bool success = 1;
  string error_message = 2;
  string instance_id = 3;
  string device_id = 4;                // Registered with HardwareService
}
```

**GetPluginInfo** - Get plugin configuration
```proto
rpc GetPluginInfo(GetPluginInfoRequest) returns (PluginInfo);

message PluginInfo {
  string plugin_id = 1;
  string name = 2;
  string version = 3;
  PluginProtocol protocol = 10;
  PluginCapabilities capabilities = 20;
  repeated PluginUIElement ui_layout = 30;
}

message PluginCapabilities {
  repeated PluginReadable readable = 1;      // Sensor readings
  optional PluginMovable movable = 2;        // Motion axes
  repeated PluginSettable settable = 3;      // Parameters
  repeated PluginSwitchable switchable = 4;  // On/off controls
  repeated PluginActionable actionable = 5;  // One-shot commands
}
```

### HealthService

System health monitoring.

**GetSystemHealth** - Overall system status
```proto
rpc GetSystemHealth(GetSystemHealthRequest)
    returns (GetSystemHealthResponse);

enum SystemHealthStatus {
  SYSTEM_HEALTH_HEALTHY = 1;
  SYSTEM_HEALTH_DEGRADED = 2;
  SYSTEM_HEALTH_CRITICAL = 3;
}

message GetSystemHealthResponse {
  SystemHealthStatus status = 1;
  uint32 total_modules = 2;
  uint32 healthy_modules = 3;
  uint32 unhealthy_modules = 4;
  uint32 total_errors = 5;
  uint32 critical_errors = 6;
  uint64 timestamp_ns = 7;
  // Database readiness (bd-9n9k.3)
  bool db_available = 8;
  bool config_service_available = 9;
  optional string db_engine = 10;        // e.g. "rocksdb", "mem"
  optional string db_state_message = 11; // Human-readable status
}
```

> **Database readiness (bd-9n9k.3):** `db_available` reflects whether SurrealDB
> initialized **and** passes its health check. When `false`,
> `config_service_available` is also `false` and `ConfigService` RPCs will
> return `Unimplemented`. Use `db_engine` (`"rocksdb"`, `"mem"`) and
> `db_state_message` from `GetSystemHealth` for static metadata — the
> streaming `HealthUpdate` carries only the boolean availability flags to
> minimize per-tick overhead. The standard gRPC health check
> (`grpc.health.v1`) reports `ConfigService` as `NOT_SERVING` when DB
> initialization failed entirely.

**StreamHealthUpdates** - Real-time health monitoring
```proto
rpc StreamHealthUpdates(StreamHealthUpdatesRequest)
    returns (stream HealthUpdate);

message StreamHealthUpdatesRequest {
  uint32 update_interval_ms = 1;       // Default: 5000ms
}

message HealthUpdate {
  SystemHealthStatus system_status = 1;
  repeated ModuleHealthStatus modules = 2;
  optional HealthErrorRecord latest_error = 3;
  uint64 timestamp_ns = 4;
  bool db_available = 5;               // Database readiness (bd-9n9k.3)
  bool config_service_available = 6;
}
```

## Streaming Overview

### Bidirectional Streaming Pattern

Several RPCs use streaming for real-time data:

```rust
// Example: StreamFrames
let request = StreamFramesRequest { .. };
let mut stream = client.stream_frames(request).await?;

// Receive frames in a loop
while let Some(frame) = stream.message().await? {
    process_frame(frame);
}
```

### Backpressure Handling

The server implements adaptive backpressure for frame streaming:
- Channel buffer capacity: 8 frames
- Skip threshold: 6 frames (75% full)
- Action: Newest frames dropped when buffer full to prevent lag

This ensures responsive UI even on slow networks.

### Compression

Frame data supports optional compression:

```proto
enum CompressionType {
  COMPRESSION_NONE = 0;
  COMPRESSION_LZ4 = 1;        // Fast compression for camera data
}

message FrameData {
  bytes data = 5;
  CompressionType compression = 50;
  uint32 uncompressed_size = 51;
}
```

Decompress using the indicated algorithm:

```rust
if frame.compression == CompressionType::Lz4 as i32 {
    let decompressed = lz4::decompress(&frame.data, frame.uncompressed_size as usize)?;
    // Use decompressed data
}
```

## Error Handling

All responses include success/error fields:

```proto
message ReadValueResponse {
  bool success = 1;
  string error_message = 2;
  double value = 3;
  // ...
}
```

**Always check `success` flag:**

```rust
let response = client.move_absolute(request).await?;
if !response.get_ref().success {
    eprintln!("Motion failed: {}", response.get_ref().error_message);
    return Err(response.get_ref().error_message.clone().into());
}
```

**Common Errors:**
- `device_not_found` - Device ID doesn't exist
- `capability_not_supported` - Operation not available on device
- `device_offline` - Hardware disconnected or unavailable
- `operation_in_progress` - Can't perform operation while another is running
- `timeout` - Operation exceeded timeout limit

## Client Libraries

### Rust

```rust
use server::grpc::{
    proto::hardware_service_client::HardwareServiceClient,
    proto::ListDevicesRequest,
};
use tonic::transport::Channel;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let channel = Channel::from_static("http://localhost:50051")
        .connect()
        .await?;

    let mut client = HardwareServiceClient::new(channel);

    let request = ListDevicesRequest {
        capability_filter: Some("movable".to_string()),
    };

    let response = client.list_devices(request).await?;
    for device in response.get_ref().devices {
        println!("{}: {}", device.id, device.name);
    }

    Ok(())
}
```

Add to `Cargo.toml`:
```toml
[dependencies]
tonic = "0.11"
tokio = { version = "1", features = ["full"] }
protocol = { path = "../protocol" }
```

### Python

```python
import grpc
import asyncio
from protocol.daq_pb2 import ListDevicesRequest
from protocol.daq_pb2_grpc import HardwareServiceStub

async def main():
    async with grpc.aio.insecure_channel("localhost:50051") as channel:
        stub = HardwareServiceStub(channel)

        request = ListDevicesRequest(
            capability_filter="movable"
        )

        response = await stub.ListDevices(request)
        for device in response.devices:
            print(f"{device.id}: {device.name}")

if __name__ == "__main__":
    asyncio.run(main())
```

Install gRPC Python tools:
```bash
pip install grpcio grpcio-tools
python -m grpc_tools.protoc -I. --python_out=. --grpc_python_out=. protocol/daq.proto
```

## Configuration

gRPC server settings are configured in two places:

**Static Configuration** in `config/config.v4.toml`:

```toml
[grpc]
# Bind address (0.0.0.0 = all interfaces, 127.0.0.1 = loopback only)
bind_address = "0.0.0.0"

# Enable token authentication
auth_enabled = false

# Allowed origins for CORS
allowed_origins = [
    "http://localhost:3000",
    "http://127.0.0.1:3000",
]

# TLS certificate and key paths (optional, enables TLS when both are set)
# tls_cert_path = "config/tls/server.crt"
# tls_key_path = "config/tls/server.key"
```

**Runtime Configuration** via CLI arguments:

- **Port:** Set via `--port` flag (default: 50051)
  ```bash
  ./rust-daq-daemon daemon --port 50051
  ```
- **Hardware Config:** Set via `--hardware-config` flag (required for device initialization)
  ```bash
  ./rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml
  ```

**TLS Behavior:**
- TLS is enabled automatically when both `tls_cert_path` and `tls_key_path` are configured in the TOML file
- If either path is missing or commented out, the server runs without TLS (unencrypted gRPC)

## Performance Considerations

### Rate Limiting

Streaming RPCs support rate limiting:

```proto
message StreamValuesRequest {
  string device_id = 1;
  uint32 rate_hz = 2;              // Max sample rate
}
```

### Memory Usage

Large frame streaming can consume significant memory:
- Full resolution camera: ~8 MB/frame (2048x2048, 16-bit)
- Preview (2x2 binned): ~2 MB/frame
- Fast (4x4 binned): ~0.5 MB/frame

Use preview mode for remote connections or when bandwidth is limited.

### Deadband Filtering

ObservableValue streaming supports deadband to reduce updates:

```proto
message StreamObservablesRequest {
  double deadband = 4;             // Only update if change > deadband
}
```

This prevents noise from triggering excessive updates.

## Timestamps and Timing

All timestamps are in nanoseconds since Unix epoch (UNIX_EPOCH):

```rust
use std::time::{SystemTime, UNIX_EPOCH};

let duration = SystemTime::now()
    .duration_since(UNIX_EPOCH)?;
let timestamp_ns = duration.as_nanos() as u64;
```

## Examples

### Complete Motion Workflow

```rust
use server::grpc::proto::hardware_service_client::HardwareServiceClient;
use server::grpc::proto::{ListDevicesRequest, MoveRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = HardwareServiceClient::connect("http://localhost:50051").await?;

    // List movable devices
    let devices = client.list_devices(ListDevicesRequest {
        capability_filter: Some("movable".to_string()),
    }).await?;

    if devices.get_ref().devices.is_empty() {
        eprintln!("No movable devices found");
        return Ok(());
    }

    let device_id = &devices.get_ref().devices[0].id;
    println!("Using device: {}", device_id);

    // Move to position 90 degrees
    let response = client.move_absolute(MoveRequest {
        device_id: device_id.clone(),
        value: 90.0,
        wait_for_completion: Some(true),
        timeout_ms: Some(10000),
    }).await?;

    if response.get_ref().success {
        println!("Moved to: {}", response.get_ref().final_position);
    } else {
        eprintln!("Move failed: {}", response.get_ref().error_message);
    }

    Ok(())
}
```

### Streaming Camera Frames

```rust
use server::grpc::proto::hardware_service_client::HardwareServiceClient;
use server::grpc::proto::{StreamFramesRequest, StreamQuality};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = HardwareServiceClient::connect("http://localhost:50051").await?;

    let request = StreamFramesRequest {
        device_id: "camera0".to_string(),
        max_fps: 30,
        quality: StreamQuality::Preview.into(),
    };

    let mut stream = client.stream_frames(request).await?;

    let mut frame_count = 0;
    while let Some(frame) = stream.message().await? {
        frame_count += 1;
        println!("Frame {}: {}x{} ({} bytes)",
            frame.frame_number, frame.width, frame.height, frame.data.len());

        if frame_count >= 100 {
            break;
        }
    }

    println!("Received {} frames", frame_count);
    Ok(())
}
```

### Reading Sensor Values

```rust
use server::grpc::proto::hardware_service_client::HardwareServiceClient;
use server::grpc::proto::ReadValueRequest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = HardwareServiceClient::connect("http://localhost:50051").await?;

    // Read power meter
    let response = client.read_value(ReadValueRequest {
        device_id: "power_meter".to_string(),
    }).await?;

    let ref resp = response.get_ref();
    if resp.success {
        // Note: units field is important - Newport returns Watts
        let power_mw = resp.value * 1000.0; // Convert to mW
        println!("Power: {:.2} {} (original: {} {})",
            power_mw, "mW", resp.value, resp.units);
    } else {
        eprintln!("Read failed: {}", resp.error_message);
    }

    Ok(())
}
```

## See Also

- [DEMO.md](../../docs/tutorials/demo-mode.md) - Quick start with mock devices
- [CLAUDE.md](../../CLAUDE.md) - Project architecture and development guidelines
- Rhai scripting documentation in crate `scripting` - Script examples and API reference
