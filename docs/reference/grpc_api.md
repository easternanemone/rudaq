# gRPC API Reference

_Auto-generated from `proto/daq.proto`_

## Overview

The rust-daq gRPC API provides remote control of the DAQ system. Connect to the daemon at `localhost:50051` (default).

## Services

| Service | Description |
|---------|-------------|
| [`ControlService`](#controlservice) | Script management and execution |
| [`HardwareService`](#hardwareservice) | Direct device control |
| [`PresetService`](#presetservice) | Configuration presets |
| [`ScanService`](#scanservice) | Coordinated multi-axis scans |
| [`ModuleService`](#moduleservice) | Experiment modules with runtime assignment |
| [`RunEngineService`](#runengineservice) | Bluesky-style plan execution |
| [`StorageService`](#storageservice) | HDF5 data storage |
| [`PluginService`](#pluginservice) | Dynamic instrument plugins |
| [`HealthService`](#healthservice) | System health monitoring |

---

## ControlService

Script management and execution.

### RPCs

| Method | Request | Response | Description |
|--------|---------|----------|-------------|
| `UploadScript` | `UploadRequest` | `UploadResponse` | Upload a Rhai script |
| `StartScript` | `StartRequest` | `StartResponse` | Execute an uploaded script |
| `StopScript` | `StopRequest` | `StopResponse` | Stop a running script |
| `GetScriptStatus` | `StatusRequest` | `ScriptStatus` | Get execution status |
| `StreamStatus` | `StatusRequest` | `stream SystemStatus` | Stream system status |
| `StreamMeasurements` | `MeasurementRequest` | `stream DataPoint` | Stream measurement data |
| `ListScripts` | `ListScriptsRequest` | `ListScriptsResponse` | List uploaded scripts |
| `ListExecutions` | `ListExecutionsRequest` | `ListExecutionsResponse` | List all executions |
| `GetDaemonInfo` | `DaemonInfoRequest` | `DaemonInfoResponse` | Get daemon version/capabilities |

### Script States
- `PENDING` - Queued for execution
- `RUNNING` - Currently executing
- `COMPLETED` - Finished successfully
- `ERROR` - Execution failed
- `STOPPED` - Stopped by user

---

## HardwareService

Direct device control for all capability traits.

### Device Discovery

| Method | Description |
|--------|-------------|
| `ListDevices` | List all registered devices with capabilities |
| `GetDeviceState` | Get current state of a device |
| `SubscribeDeviceState` | Stream real-time state updates |

### Motion Control (Movable devices)

| Method | Description |
|--------|-------------|
| `MoveAbsolute` | Move to absolute position |
| `MoveRelative` | Move relative to current position |
| `StopMotion` | Emergency stop |
| `WaitSettled` | Wait for motion to complete |
| `StreamPosition` | Stream position updates |

### Scalar Readout (Readable devices)

| Method | Description |
|--------|-------------|
| `ReadValue` | Read current value |
| `StreamValues` | Stream continuous readings |

### Trigger Control (Triggerable devices)

| Method | Description |
|--------|-------------|
| `Arm` | Arm device for triggering |
| `Trigger` | Send trigger signal |

### Exposure Control (ExposureControl devices)

| Method | Description |
|--------|-------------|
| `SetExposure` | Set exposure time (ms) |
| `GetExposure` | Get current exposure |

### Laser Control

| Method | Description |
|--------|-------------|
| `SetShutter` / `GetShutter` | Shutter open/close |
| `SetWavelength` / `GetWavelength` | Wavelength tuning (nm) |
| `SetEmission` / `GetEmission` | Emission on/off |

### Frame Streaming (FrameProducer devices)

| Method | Description |
|--------|-------------|
| `StartStream` | Begin frame acquisition |
| `StopStream` | Stop acquisition |
| `StreamFrames` | Stream frame data |

### Observable Parameters

| Method | Description |
|--------|-------------|
| `ListParameters` | List all device parameters |
| `GetParameter` | Get parameter value |
| `SetParameter` | Set parameter value |
| `StreamParameterChanges` | Stream parameter changes |

---

## ScanService

Coordinated multi-axis scanning.

### Scan Types
- `LINE_SCAN` - Single axis
- `GRID_SCAN` - 2D raster
- `SNAKE_SCAN` - Bidirectional raster
- `CUSTOM_SCAN` - User-defined points

### RPCs

| Method | Description |
|--------|-------------|
| `CreateScan` | Configure a new scan |
| `StartScan` | Begin execution |
| `PauseScan` | Pause at safe point |
| `ResumeScan` | Resume paused scan |
| `StopScan` | Stop/abort scan |
| `GetScanStatus` | Get current status |
| `StreamScanProgress` | Stream progress updates |

### Scan States
- `SCAN_CREATED` - Ready to start
- `SCAN_RUNNING` - Actively executing
- `SCAN_PAUSED` - Paused
- `SCAN_COMPLETED` - All points acquired
- `SCAN_STOPPED` - Stopped by user
- `SCAN_ERROR` - Error occurred

---

## StorageService

HDF5 data storage and export.

### RPCs

| Method | Description |
|--------|-------------|
| `ConfigureStorage` | Set output directory and HDF5 options |
| `StartRecording` | Begin recording to HDF5 |
| `StopRecording` | Finalize HDF5 file |
| `GetRecordingStatus` | Get recording progress |
| `ListAcquisitions` | List saved HDF5 files |
| `GetAcquisitionInfo` | Get file metadata |
| `FlushToStorage` | Manual flush to disk |
| `GetRingBufferTapInfo` | Get ring buffer mmap tap info for zero-copy access |

### HDF5 Configuration Options
- **Compression:** `"none"`, `"gzip"`, `"lz4"`, `"zstd"`
- **Chunk size:** Default 4096
- **Filename pattern:** e.g., `"{name}_{timestamp}.h5"`

### Ring Buffer Tap (Zero-Copy Access)

The `GetRingBufferTapInfo` RPC returns information needed for Python/Julia clients to access the ring buffer directly via memory-mapped files, bypassing gRPC for high-throughput data access.

**Response fields:**
- `file_path` - Path to memory-mapped ring buffer file
- `capacity_bytes` - Data region size
- `header_size` - Header size (128 bytes)
- `stream_id` - Changes when buffer is recreated
- `write_head` / `read_tail` / `write_epoch` - Current buffer state

**Python example:** See `clients/python/examples/09_ring_buffer_tap.py`

---

## RunEngineService

Bluesky-style plan execution with Document Model.

### RPCs

| Method | Description |
|--------|-------------|
| `ListPlanTypes` | List available plan types |
| `QueuePlan` | Queue a plan for execution |
| `StartEngine` | Start processing queue |
| `PauseEngine` | Pause at checkpoint |
| `ResumeEngine` | Resume execution |
| `AbortPlan` | Abort current plan |
| `HaltEngine` | Emergency stop |
| `StreamDocuments` | Stream experiment documents |

### Document Types
- `DOC_START` - Experiment intent and metadata
- `DOC_DESCRIPTOR` - Data stream schema
- `DOC_EVENT` - Actual measurements
- `DOC_STOP` - Completion status

---

## PluginService

Dynamic instrument plugin management.

### RPCs

| Method | Description |
|--------|-------------|
| `ListPlugins` | List available plugin types |
| `GetPluginInfo` | Get plugin capabilities and UI hints |
| `SpawnPlugin` | Create plugin instance |
| `ListPluginInstances` | List active instances |
| `DestroyPluginInstance` | Remove instance |

### Plugin Driver Types
- `serial_scpi` - Serial port with SCPI commands
- `tcp_scpi` - TCP with SCPI commands
- `serial_raw` - Serial with raw protocol
- `tcp_raw` - TCP with raw protocol

---

## HealthService

System health monitoring.

### RPCs

| Method | Description |
|--------|-------------|
| `GetSystemHealth` | Overall system status |
| `GetModuleHealth` | Per-module health |
| `GetErrorHistory` | Recent errors |
| `StreamHealthUpdates` | Real-time health stream |

### Health Status Levels
- `SYSTEM_HEALTH_HEALTHY` - All systems operational
- `SYSTEM_HEALTH_DEGRADED` - Some issues but functional
- `SYSTEM_HEALTH_CRITICAL` - Critical failures

---

## Python Client Example

```python
import grpc
from rust_daq_client import DaqClient

async with DaqClient("localhost:50051") as client:
    # List devices
    devices = await client.list_devices()

    # Move stage
    await client.move_absolute("stage_x", 10.0, wait=True)

    # Read power meter
    value = await client.read_value("power_meter")
    print(f"Power: {value.value} {value.units}")

    # Start scan
    scan_id = await client.create_scan(
        axes=[{"device_id": "stage_x", "start": 0, "end": 100, "points": 101}],
        detectors=["power_meter"],
        dwell_time_ms=100
    )
    await client.start_scan(scan_id)
```

## Protocol Buffers

Full protobuf definitions: [`proto/daq.proto`](../../proto/daq.proto)

Generate client stubs:
```bash
# Python
python -m grpc_tools.protoc -I proto --python_out=. --grpc_python_out=. proto/daq.proto

# Rust (handled by build.rs)
cargo build --features networking
```
