## gRPC API Implementation - Phase 2 (bd-3pdi)

## Overview

The gRPC API provides a network interface for remote control of the DAQ system headless daemon. Phase 2 extends this with specialized hardware control and high-performance frame streaming.

## Implementation Status

**COMPLETED:**

1. Added gRPC dependencies (tonic, prost, tokio-stream)
2. Created Protocol Buffer definition (`crates/daq-proto/proto/daq.proto`)
3. Created build configuration for proto compilation
4. Integrated proto module into network module
5. Verified successful code generation
6. **Implemented `HardwareService`**: Generic control for `Movable`, `Readable`, and `Triggerable` devices.
7. **Implemented `CameraService`**: Specialized specialized for high-bandwidth `u16` frame streaming from `FrameProducer` devices.

## Proto File Structure

**Location:** `/Users/briansquires/code/rust-daq/crates/daq-proto/proto/daq.proto`

The proto definition includes:

### Service Definitions

- `ControlService`: Scripting and system status.
- `HardwareService`: Generic device control (position, reading, triggering).
- `CameraService`: Camera-specific controls (exposure, ROI) and frame streaming.

### Specialized RPC Methods (Phase 2)

#### Hardware Control (`HardwareService`)

1. **MoveAbs / MoveRel**: Control `Movable` devices.
2. **GetPosition**: Read current position.
3. **ReadScalar**: Get single value from `Readable` devices.
4. **Trigger**: Fire a software/hardware trigger.

#### Camera & Streaming (`CameraService`)

1. **SetExposure / GetExposure**: Exposure time in seconds.
2. **SetRoi / GetRoi**: Region of Interest.
3. **StreamFrames**: High-performance `u16` stream with metadata (timestamps, sequence IDs).

## Implementation Details

### Frame Streaming Pipeline

Frames are broadcast from the hardware driver via a `tokio::sync::broadcast` channel. The `CameraService` subscribes to this channel and forwards frames over a gRPC stream.

- **Zero-Copy**: Uses `Arc<Frame>` to minimize copying between the driver and the gRPC encoder.
- **Backpressure**: Handled by the broadcast channel capacity and gRPC flow control.

### Parameter Synchronization

Uses the V5 `Parameter<T>` system. Changes made via gRPC are validated by the `Parameter` before being written to hardware, ensuring "Split Brain" avoidance.

## Acceptance Criteria

- [x] `crates/daq-proto/proto/daq.proto` defines complete API
- [x] `CameraService` supports high-performance streaming
- [x] Metadata (timestamps, sequence numbers) is preserved through the pipeline
- [x] All RPC methods integrated with `DeviceRegistry`
