# protocol

Protobuf message definitions for rust-daq gRPC API.

## Overview

The `protocol` crate contains the protobuf definitions for the rust-daq gRPC services. These are compiled at build time to generate Rust code for serialization, deserialization, and service definitions.

The generated code is NOT checked in. Instead, it is generated on-the-fly at build time using the `tonic::include_proto!()` macro, which compiles the `.proto` files and includes the generated types directly into the binary.

## Proto Files

Protobuf definitions are in the `proto/` directory:

| File | Purpose |
|------|---------|
| `daq.proto` | Core DAQ services and messages |
| `health.proto` | gRPC health checking protocol (standard) |
| `ni_daq.proto` | NI DAQ-specific extensions for Comedi hardware |

### Services

The protocol defines eight primary gRPC services:

| Service | Purpose |
|---------|---------|
| **ControlService** | Script management and execution with system status streaming |
| **HardwareService** | Device enumeration, control, motion, streaming |
| **PresetService** | Save/load/manage device configuration presets |
| **ScanService** | Multi-axis coordinated scanning (DEPRECATED - use RunEngineService) |
| **ModuleService** | Hardware-agnostic experiment modules with device role assignment |
| **RunEngineService** | Plan-based experiment execution with Bluesky document streaming |
| **StorageService** | HDF5 data storage, recording, and export (bd-p6im) |
| **PluginService** | Plugin discovery and instance management for declarative drivers |
| **HealthService** | System health monitoring and error tracking (bd-pauy) |

### Key Message Types

| Message | Purpose |
|---------|---------|
| `DeviceInfo` | Complete device metadata and capabilities (replaces old `Device`) |
| `DeviceMetadata` | Device-specific configuration (position units, exposure limits, etc.) |
| `ParameterDescriptor` | Observable parameter definition (for parameterized devices) |
| `FrameData` | Camera/sensor frame with pixel data and metadata |
| `Document` | Bluesky-pattern structured experiment event (RunEngineService) |
| `Preset` | Saved device configuration snapshot (PresetService) |
| `PlanParameter` | Plan configuration parameter (RunEngineService) |
| `RecordingStatus` | Data recording progress and state (StorageService) |
| `RingBufferTapInfo` | Cross-process ring buffer access info (StorageService, bd-vms4.2) |
| `ModuleEvent` | Module-emitted events with status and data (ModuleService) |
| `HealthUpdate` | Real-time system health monitoring (HealthService, bd-pauy) |

## Generated Code Workflow

The protobuf definitions are compiled at build time by `build.rs`:

```bash
# During: cargo build -p protocol
# build.rs runs tonic_build::configure().compile() to:
# 1. Compile proto files with protobuf compiler
# 2. Generate Rust code from message and service definitions
# 3. Return generated code to OUT_DIR (not committed)
```

The generated code is then included directly into the library using the `tonic::include_proto!()` macro in `src/lib.rs`:

```rust
pub mod daq {
    tonic::include_proto!("daq");  // Includes generated types at compile time
}
```

**No `.rs` files are committed to the repository.** Generated code exists only in the build output and is regenerated on each build.

## Modifying Protobuf Definitions

### 1. Edit Proto File

Edit the relevant proto file to add/change messages or services:
- `proto/daq.proto` - Core DAQ API
- `proto/health.proto` - Health check (rarely modified)
- `proto/ni_daq.proto` - NI DAQ extensions

### 2. Build to Regenerate

```bash
cargo build -p protocol
```

This automatically:
1. Compiles updated proto files with tonic_build
2. Generates new Rust types and service traits
3. Includes them via the `tonic::include_proto!()` macros in `src/lib.rs`

### 3. Commit Proto File Only

```bash
git add crates/protocol/proto/daq.proto    # Only commit the source proto files
git commit -m "proto: update message definitions"
```

Do NOT commit generated files - they are regenerated automatically on build.

## Using Generated Code

### In Server Code

```rust
use protocol::daq::*;

// Use generated message types - DeviceInfo for device metadata
let device_info = DeviceInfo {
    id: "camera".to_string(),
    name: "Photometrics Prime BSI".to_string(),
    driver_type: "pvcam".to_string(),
    category: DeviceCategory::Camera as i32,
    capabilities: vec!["frame_producer".to_string(), "exposure_controllable".to_string()],
    metadata: Some(DeviceMetadata {
        frame_width: Some(2048),
        frame_height: Some(2048),
        bits_per_pixel: Some(16),
        ..Default::default()
    }),
    ..Default::default()
};

// Use service definitions
impl HardwareService for MyServer {
    async fn list_devices(
        &self,
        request: Request<ListDevicesRequest>,
    ) -> Result<Response<ListDevicesResponse>, Status> {
        // Implementation returns DeviceInfo, not Device
        Ok(Response::new(ListDevicesResponse {
            devices: vec![device_info],
            registration_failures: vec![],
        }))
    }
}
```

### In Client Code

```rust
use protocol::daq::*;

let request = ListDevicesRequest { capability_filter: None };
let response = client.list_devices(request).await?;

// Receive DeviceInfo messages (not Device)
for device in response.into_inner().devices {
    println!("Device: {} ({})", device.id, device.name);
    println!("Capabilities: {:?}", device.capabilities);
}

// Stream frames from a camera
let frame_request = StreamFramesRequest {
    device_id: "prime_bsi".to_string(),
    max_fps: 30,
    quality: StreamQuality::Full as i32,
};
let mut frame_stream = client.stream_frames(frame_request).await?;

while let Some(frame) = frame_stream.message().await? {
    if let Some(frame_data) = frame {
        println!("Frame {} at timestamp {}", frame_data.frame_number, frame_data.timestamp_ns);
    }
}
```

## Proto Structure

### Packages

All messages are in the `daq` package:

```protobuf
package daq;

message DeviceInfo { ... }
message FrameData { ... }
```

This maps to Rust modules:
- `protocol::daq::DeviceInfo`
- `protocol::daq::FrameData`
- etc.

### Service Definitions

Services use tonic for gRPC compilation:

```protobuf
service HardwareService {
    rpc ListDevices(ListDevicesRequest) returns (ListDevicesResponse);
    rpc StreamFrames(StreamFramesRequest) returns (stream FrameData);
}

service RunEngineService {
    rpc QueuePlan(QueuePlanRequest) returns (QueuePlanResponse);
    rpc StreamDocuments(StreamDocumentsRequest) returns (stream Document);
}
```

These are compiled by tonic_build into service traits:

```rust
#[tonic::async_trait]
pub trait HardwareService: Send + Sync + 'static {
    async fn list_devices(
        &self,
        request: tonic::Request<ListDevicesRequest>,
    ) -> Result<tonic::Response<ListDevicesResponse>, tonic::Status>;

    async fn stream_frames(
        &self,
        request: tonic::Request<StreamFramesRequest>,
    ) -> Result<tonic::Response<tonic::Streaming<FrameData>>, tonic::Status>;
    // ...
}
```

## API Versioning

The protobuf API can be versioned by:

1. **Message versioning** - Add new fields with default values (backward compatible)
2. **Deprecated fields** - Mark with `[deprecated = true]`
3. **Service versioning** - Add new service methods (old clients still work)

Example:

```protobuf
message Device {
    string id = 1;
    string name = 2;
    string deprecated_field = 3 [deprecated = true];  // Old field
    string new_field = 4;  // New field with default
}
```

## Build Process

### build.rs Workflow

The build script (`crates/protocol/build.rs`) handles protobuf compilation:

1. Detects the target architecture (skips server code for WASM)
2. Locates `proto/` directory
3. Runs tonic_build to compile protobuf files
4. Generates Rust types and service traits
5. Returns generated code to Cargo's `OUT_DIR`

The generated code is NOT copied anywhere - it exists only in memory during compilation and is included directly via the `tonic::include_proto!()` macro.

### Rebuilding Protobuf

No manual rebuild is necessary. The protobuf is automatically recompiled whenever:
- A `.proto` file changes
- You run `cargo build -p protocol`

If you need a clean rebuild:

```bash
# Option 1: Cargo's built-in clean (recommended)
cargo clean -p protocol
cargo build -p protocol

# Option 2: Full workspace clean
cargo clean
cargo build
```

Do NOT manually delete files from `src/` - there are no generated `.rs` files to delete.

## Import Conventions

In other crates, import protocol messages from the generated modules:

```rust
// Re-export pattern (protocol crate re-exports at root)
use protocol::{DeviceInfo, FrameData, ListDevicesRequest};

// Or explicit module imports
use protocol::daq::{DeviceInfo, ListDevicesRequest, ListDevicesResponse};

// Wildcard import (less preferred, brings many names into scope)
use protocol::daq::*;
```

**Common imports by use case:**

```rust
// Hardware service client
use protocol::daq::{DeviceInfo, ListDevicesRequest, StreamFramesRequest, FrameData};

// RunEngine service (plans and documents)
use protocol::daq::{QueuePlanRequest, Document, DocumentType};

// Plugin service
use protocol::daq::{ListPluginsRequest, PluginInfo, SpawnPluginRequest};
```

## External Resources

- [Protobuf Language Guide](https://developers.google.com/protocol-buffers/docs/proto3)
- [Tonic gRPC Framework](https://github.com/hyperium/tonic)
- [Prost Protobuf Compiler](https://github.com/tokio-rs/prost)

## Related Documentation

- [gRPC API Reference](../../docs/reference/grpc-api.md) - Full API documentation
- [Server Implementation](../server) - Uses protocol definitions
- [Client Library](../client) - Consumes protocol definitions

## See Also

- `server` crate - gRPC service implementations
- `client` crate - gRPC client using protocol messages
- `common` crate - Shared types that map to protobuf messages
