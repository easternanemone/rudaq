# server

gRPC server for rust-daq providing remote hardware control and data streaming.

## Overview

`server` exposes the rust-daq system over gRPC, enabling:

- **Remote Hardware Control** - Device operations over network
- **Frame Streaming** - Adaptive quality video with bandwidth optimization
- **Script Execution** - Run Rhai experiments remotely
- **Plan Execution** - Bluesky-style experiment plans with pause/resume
- **Real-time Data** - Broadcast measurements to multiple clients

## gRPC Services

| Service | Purpose |
|---------|---------|
| **HardwareService** | Direct device control (move, read, trigger) |
| **ControlService** | Script upload, validation, and execution |
| **RunEngineService** | Plan execution with pause/resume/abort |
| **PresetService** | Save/load device configuration presets |
| **ModuleService** | Device module lifecycle management |
| **StorageService** | Data persistence and retrieval |
| **Health** | Standard gRPC health checks |

## Quick Start

### Starting the Server

```rust
use daq_server::DaqServer;
use daq_hardware::DeviceRegistry;

let registry = Arc::new(DeviceRegistry::new());
// ... register devices ...

let server = DaqServer::new(registry)?;
server.serve("0.0.0.0:50051").await?;
```

### Configuration

Server configuration in `config/config.v4.toml`:

```toml
[grpc]
bind_address = "0.0.0.0"      # Listen on all interfaces
port = 50051
auth_enabled = false          # Optional JWT/API key auth
allowed_origins = ["http://localhost:3000"]  # CORS for gRPC-web
```

## Frame Streaming

Adaptive quality modes for bandwidth optimization:

| Quality | Downsampling | Size Reduction | Use Case |
|---------|--------------|----------------|----------|
| **Full** | None | 0% | Local network, analysis |
| **Preview** | 2×2 bin | ~75% | Remote monitoring |
| **Fast** | 4×4 bin | ~94% | Low bandwidth |

```rust
// Client request
let request = StreamFramesRequest {
    device_id: "camera".into(),
    quality: StreamQuality::Preview.into(),
    max_fps: 30,
};
```

### Backpressure Handling

- Channel buffer: 8 frames
- Auto frame-skip when buffer ≥75% full
- Prevents lag accumulation on slow connections

## Device Integration

The server wraps `DeviceRegistry` for network access:

```rust
// List all devices
let devices = hardware_service.list_devices().await?;

// Access by capability
if let Some(stage) = registry.get_movable("stage") {
    stage.move_abs(10.0).await?;
}

// Stream frames from camera
let stream = hardware_service.stream_frames(request).await?;
```

## Data Distribution (Mullet Strategy)

Dual-path data flow for reliability:

```
Device → Tee ─┬─→ RingBuffer (reliable, 100MB memory-mapped)
              │        ↓
              │   HDF5Writer (disk persistence)
              │
              └─→ Broadcast (lossy, real-time clients)
                     ↓
               gRPC Streams
```

- **Reliable path**: Memory-mapped buffer → HDF5 file
- **Real-time path**: Broadcast to connected clients

## Authentication

Optional JWT or API key authentication:

```toml
[grpc]
auth_enabled = true
auth_token = "your-secret-key"
```

Clients include token in metadata:
```
authorization: Bearer <token>
```

## Feature Flags

```toml
[features]
server = []                    # Core gRPC server
scripting = ["scripting"]  # Script execution
storage_hdf5 = []              # HDF5 persistence
storage_arrow = []             # Arrow/Parquet output
modules = []                   # Module lifecycle
metrics = ["prometheus"]       # Prometheus metrics
```

## Related Crates

- [`hardware`](../hardware) - Device registry and drivers
- [`scripting`](../scripting) - Rhai script engine
- [`protocol`](../protocol) - Protocol buffer definitions
- [`storage`](../storage) - Ring buffer and writers

## Proto Definitions

See `proto/` directory for service definitions:
- `hardware.proto` - Device control and streaming
- `control.proto` - Script execution
- `run_engine.proto` - Plan execution
- `storage.proto` - Data persistence

## License

See the repository root for license information.
