# gRPC API Implementation - Task G (bd-3z3z)

## Overview

The gRPC API provides a network interface for remote control of the DAQ system headless daemon. This enables remote clients to upload and execute scripts, monitor system status, and stream live measurement data.

## Implementation Status

**COMPLETED:**
1. Added gRPC dependencies (tonic, prost, tokio-stream)
2. Created Protocol Buffer definition (`src/network/proto/daq.proto`)
3. Created build configuration for proto compilation
4. Integrated proto module into network module
5. Verified successful code generation

## Proto File Structure

**Location:** `/Users/briansquires/code/rust-daq/src/network/proto/daq.proto`

The proto definition includes:

### Service Definition
- `ControlService` - Main gRPC service with 6 RPC methods

### RPC Methods

#### Script Management (4 methods)
1. **UploadScript** - Upload a script to the daemon
   - Input: `UploadRequest` (script_content, name, metadata)
   - Output: `UploadResponse` (script_id, success, error_message)

2. **StartScript** - Start execution of an uploaded script
   - Input: `StartRequest` (script_id)
   - Output: `StartResponse` (started, execution_id)

3. **StopScript** - Stop a running script
   - Input: `StopRequest` (execution_id)
   - Output: `StopResponse` (stopped)

4. **GetScriptStatus** - Query script execution status
   - Input: `StatusRequest` (execution_id)
   - Output: `ScriptStatus` (execution_id, state, error_message, timestamps)

#### Live Data Streaming (2 methods)
5. **StreamStatus** - Stream system status updates
   - Input: `StatusRequest` (execution_id)
   - Output: stream of `SystemStatus` (state, memory, live_values, timestamp)

6. **StreamMeasurements** - Stream measurement data
   - Input: `MeasurementRequest` (instrument)
   - Output: stream of `DataPoint` (instrument, value [scalar|image], timestamp)

### Message Types

**Request Messages:**
- `UploadRequest` - Script upload with metadata
- `StartRequest` - Script execution trigger
- `StopRequest` - Script execution stop
- `StatusRequest` - Status query
- `MeasurementRequest` - Measurement stream subscription

**Response Messages:**
- `UploadResponse` - Upload result
- `StartResponse` - Start result with execution_id
- `StopResponse` - Stop confirmation
- `ScriptStatus` - Script execution state (IDLE, RUNNING, COMPLETED, ERROR)
- `SystemStatus` - System state with live values
- `DataPoint` - Measurement data (scalar or image bytes)

## Generated Rust Modules

**Build Output:** `/Users/briansquires/code/rust-daq/target/debug/build/rust_daq-*/out/daq.rs`

The `tonic-build` tool generates approximately 35KB of Rust code including:

**Server-side:**
- `proto::control_service_server::ControlService` trait
- `proto::control_service_server::ControlServiceServer` struct

**Client-side:**
- `proto::control_service_client::ControlServiceClient` struct

**Message structs:**
- All request/response types with serde serialization
- Proper timestamp handling (u64 nanoseconds)
- Map support for metadata and live_values
- Oneof support for DataPoint values (scalar vs image)

## Module Accessibility

The proto module is integrated into the library via `src/network/mod.rs`:

```rust
pub mod proto {
    tonic::include_proto!("daq");
}

// Re-exported for convenience
pub use proto::control_service_server::{ControlService, ControlServiceServer};
pub use proto::control_service_client::ControlServiceClient;
pub use proto::{
    DataPoint, MeasurementRequest, ScriptStatus, StartRequest, StartResponse,
    StatusRequest, StopRequest, StopResponse, SystemStatus, UploadRequest, UploadResponse,
};
```

## Build Verification

**Build Command:**
```bash
cargo build --lib
```

**Result:** Successfully generates proto code during build process

**Generated Types:** All 11 message structs + 2 service modules (client + server)

## Next Steps (for future tasks)

1. **Task H** - Implement `ControlService` trait (server implementation)
2. **Task I** - Integrate with script engine for execution
3. **Task J** - Implement streaming endpoints for live data
4. **Task K** - Add authentication/authorization
5. **Task L** - Create client SDK/examples

## Acceptance Criteria - COMPLETED

- [x] `src/network/proto/daq.proto` defines complete API
- [x] `build.rs` successfully compiles proto files
- [x] `cargo build` generates Rust code from proto
- [x] Module `network::proto` is accessible
- [x] All 6 RPC methods defined (Upload, Start, Stop, GetStatus, StreamStatus, StreamMeasurements)

## Dependencies Added

```toml
[dependencies]
tonic = "0.10"
prost = "0.12"
tokio-stream = "0.1"

[build-dependencies]
tonic-build = "0.10"
```

## Files Created/Modified

**Created:**
- `/Users/briansquires/code/rust-daq/src/network/proto/daq.proto` - Protocol definition
- `/Users/briansquires/code/rust-daq/build.rs` - Build script with proto compilation

**Modified:**
- `/Users/briansquires/code/rust-daq/Cargo.toml` - Added dependencies
- `/Users/briansquires/code/rust-daq/src/network/mod.rs` - Added proto module and exports
