# Rerun SDK Refactoring Guide

**Date:** December 2025 (Updated: January 2026)
**Status:** Implementation Complete (Phase 4 blocked waiting for stable Rust Blueprint API)
**Rerun Version:** 0.27.3
**egui Version:** 0.33 (upgraded from 0.31 for Rerun compatibility)

## Executive Summary

This document analyzes the current Rerun integration in rust-daq and proposes a comprehensive refactoring plan to better leverage Rerun's native capabilities. The key insight is that **Rerun already provides a production-grade gRPC infrastructure** that we are partially duplicating with our custom camera streaming implementation.

## Current Architecture Analysis

### Current Integration Points

| Component | File | Purpose |
|-----------|------|---------|
| `RerunSink` | `daq-server/src/rerun_sink.rs` | Measurement → Rerun logging |
| `main_rerun.rs` | `daq-egui/src/main_rerun.rs` | Embedded viewer + DAQ controls |
| Blueprint Generator | `daq-server/blueprints/generate_blueprints.py` | Python SDK for layouts |
| gRPC Server | `daq-server/src/grpc/` | Custom streaming impl |

### Architecture Diagram (Current)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            CURRENT ARCHITECTURE                              │
└─────────────────────────────────────────────────────────────────────────────┘

  Remote Machine (maitai)                     Local Machine
  ┌──────────────────────────────┐           ┌─────────────────────────────────┐
  │  ┌─────────────────────┐     │           │                                 │
  │  │   PVCAM Camera      │     │           │  ┌───────────────────────────┐  │
  │  │   (Prime BSI)       │     │           │  │   daq-rerun GUI           │  │
  │  └──────────┬──────────┘     │           │  │   ┌───────────────────┐   │  │
  │             │                │           │  │   │ DAQ Control Panel │   │  │
  │             ▼                │           │  │   │   (egui)          │   │  │
  │  ┌─────────────────────┐     │           │  │   └─────────┬─────────┘   │  │
  │  │   daq-server        │     │           │  │             │             │  │
  │  │   (gRPC daemon)     │     │           │  │   ┌─────────▼─────────┐   │  │
  │  │   ┌─────────────┐   │     │   SSH     │  │   │ Embedded Rerun    │   │  │
  │  │   │ HardwareService│◄─────┼───Tunnel──┼──┼───│ Viewer            │   │  │
  │  │   │ StreamFrames │   │    │  :50051   │  │   │ (re_viewer::App)  │   │  │
  │  │   └──────┬──────┘   │     │           │  │   └─────────▲─────────┘   │  │
  │  │          │          │     │           │  │             │             │  │
  │  │   ┌──────▼──────┐   │     │           │  │   ┌─────────┴─────────┐   │  │
  │  │   │ RerunSink   │───┼─────┼───spawn───┼──┼──►│ re_grpc_server    │   │  │
  │  │   │ (spawn)     │   │     │           │  │   │ :9876             │   │  │
  │  │   └─────────────┘   │     │           │  └───┴───────────────────┴───┘  │
  │  └─────────────────────┘     │           │                                 │
  └──────────────────────────────┘           └─────────────────────────────────┘

PROBLEMS:
1. TWO gRPC channels: Custom StreamFrames (:50051) + Rerun gRPC (:9876)
2. Frame data copied: Camera → Custom Proto → GUI → Tensor → Rerun
3. Custom FrameData proto duplicates Rerun's native Tensor/Image archetypes
4. RerunSink on daemon spawns viewer - but GUI has embedded viewer
5. Memory: Frames decoded twice (proto deserialization + tensor conversion)
```

### Key Inefficiencies Identified

1. **Dual gRPC Channels**: We maintain custom gRPC streaming (`StreamFrames`) alongside Rerun's native gRPC server. This creates unnecessary complexity and doubles network traffic for the same data.

2. **Redundant Data Serialization**:
   - Camera → `FrameData` proto (custom) → gRPC → GUI → `TensorData` → Rerun gRPC
   - Should be: Camera → `Tensor` archetype → Rerun gRPC → Viewer

3. **Blueprint Generation in Python**: Rust Blueprint API is now available (since Rerun 0.24), eliminating the need for Python-generated `.rbl` files.

4. **Conflicting Rerun Instances**:
   - `RerunSink::new()` calls `spawn()` on the daemon
   - `main_rerun.rs` embeds its own `re_viewer::App` with `re_grpc_server`
   - Results in potential viewer conflicts

5. **No Multi-Sink Support**: We don't leverage Rerun 0.24's `set_sinks()` API for simultaneous visualization + recording.

6. **Raw Tensor for Images**: Using `Tensor` archetype for camera frames instead of `Image` or `EncodedImage`, missing colormap/contrast features.

## Proposed Architecture

### Target Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           PROPOSED ARCHITECTURE                              │
└─────────────────────────────────────────────────────────────────────────────┘

  Remote Machine (maitai)                     Local Machine
  ┌──────────────────────────────┐           ┌─────────────────────────────────┐
  │  ┌─────────────────────┐     │           │                                 │
  │  │   PVCAM Camera      │     │           │  ┌───────────────────────────┐  │
  │  │   (Prime BSI)       │     │           │  │   daq-rerun GUI           │  │
  │  └──────────┬──────────┘     │           │  │   ┌───────────────────┐   │  │
  │             │                │           │  │   │ DAQ Control Panel │   │  │
  │             ▼                │           │  │   │   (egui)          │   │  │
  │  ┌─────────────────────┐     │           │  │   └─────────┬─────────┘   │  │
  │  │   daq-server        │     │   Rerun   │  │             │             │  │
  │  │   (gRPC daemon)     │     │   gRPC    │  │   ┌─────────▼─────────┐   │  │
  │  │   ┌─────────────┐   │     │           │  │   │ Embedded Rerun    │   │  │
  │  │   │ RerunSink   │───┼─────┼───:9876───┼──┼──►│ Viewer            │   │  │
  │  │   │ serve_grpc()│   │     │  (native) │  │   │ (re_viewer::App)  │   │  │
  │  │   └─────────────┘   │     │           │  │   └───────────────────┘   │  │
  │  │         │           │     │           │  │                           │  │
  │  │   ┌─────▼─────┐     │     │           │  │   Control commands only:  │  │
  │  │   │ FileSink  │     │     │   SSH     │  │   ┌───────────────────┐   │  │
  │  │   │ (.rrd)    │     │◄────┼───:50051──┼──┼───│ DaqClient (gRPC)  │   │  │
  │  │   └───────────┘     │     │ (control) │  │   │ Move/Read/Status  │   │  │
  │  └─────────────────────┘     │           │  └───┴───────────────────┴───┘  │
  └──────────────────────────────┘           └─────────────────────────────────┘

BENEFITS:
1. SINGLE data path: Camera → Rerun SDK → Native gRPC → Viewer
2. Control plane separate from data plane (clean separation)
3. Built-in recording to .rrd via multi-sink
4. Native Rerun compression/batching for camera frames
5. Proper Image archetype with colormap support
```

## Refactoring Plan

### Phase 1: Consolidate gRPC Architecture

**Goal**: Use Rerun's native gRPC for all visualization data, keep custom gRPC only for control commands.

#### 1.1 Modify RerunSink to use `serve_grpc()` instead of `spawn()`

```rust
// BEFORE (rerun_sink.rs)
pub fn new() -> Result<Self> {
    let rec = RecordingStreamBuilder::new(APP_ID)
        .spawn()?;  // Spawns local viewer
    Ok(Self { rec })
}

// AFTER - Using serve_grpc_opts for remote access
use re_grpc_server::ServerOptions;
use re_memory::MemoryLimit;

pub fn new_server(bind_ip: &str, port: u16) -> Result<Self> {
    let rec = RecordingStreamBuilder::new(APP_ID)
        .serve_grpc_opts(
            bind_ip,  // "0.0.0.0" for remote access
            port,     // Default: 9876
            ServerOptions {
                // Set to 0B if server/client on same machine to avoid double-buffering
                // Otherwise use fraction of total memory
                memory_limit: MemoryLimit::from_fraction_of_total(0.25),
                ..Default::default()
            },
        )?;
    Ok(Self { rec })
}
```

**Key Documentation Notes** (from Rerun docs):
- The gRPC server buffers data so late-connecting viewers get all data
- **Critical**: Set `memory_limit` to `0B` if server and client run on same machine
- Static data is never dropped when memory limit is reached
- Clients connect via `rerun+http://{bind_ip}:{port}/proxy`

**Rationale**: `serve_grpc()` starts a gRPC server in the daemon process that remote viewers (including our embedded viewer) can connect to. This eliminates the need for custom `StreamFrames` RPC.

#### 1.2 Remove Custom Frame Streaming

**Files to modify:**
- `crates/daq-proto/proto/daq.proto` - Remove `StreamFrames` RPC and `FrameData` message
- `crates/daq-server/src/grpc/hardware_service.rs` - Remove `stream_frames` implementation
- `crates/daq-egui/src/client.rs` - Remove `stream_frames` method
- `crates/daq-egui/src/main_rerun.rs` - Remove custom frame streaming code

**Why**: Rerun's native gRPC handles frame streaming more efficiently with built-in batching, compression, and memory management.

#### 1.3 Update GUI to Connect to Daemon's Rerun Server

```rust
// BEFORE (main_rerun.rs)
// GUI spawns its own re_grpc_server and uses custom StreamFrames
let (data_rx, _) = re_grpc_server::spawn_with_recv(...);

// AFTER
// GUI connects to daemon's Rerun gRPC server
// NOTE: URL format must include /proxy suffix
let rec = RecordingStreamBuilder::new("rust-daq")
    .connect_grpc_opts("rerun+http://100.117.5.12:9876/proxy")?;

// Or use default local connection:
let rec = RecordingStreamBuilder::new("rust-daq")
    .connect_grpc()?;  // Connects to rerun+http://127.0.0.1:9876/proxy
```

**Important URL Format** (from Rerun 0.27 docs):
- URLs must include the `rerun://`, `rerun+http://`, or `rerun+https://` scheme **and** end with `/proxy`.
- Local default: `rerun+http://127.0.0.1:9876/proxy`.
- Remote: `rerun+http://{host}:{port}/proxy`.
- TLS: `rerun://{host}:{port}/proxy` (with certs configured).

**Viewer always starts a gRPC server:** every native Rerun viewer process hosts a gRPC server; you cannot start a viewer without a server. Avoid spawning multiple servers on the same port (e.g., daemon + embedded viewer) unless you intend a multi-hop proxy.

**Server memory limits:**
- Default `serve_grpc/serve_web` memory cap is 25% of system RAM; otherwise `0B` if no serve flags are used.
- Set `memory_limit: 0B` when the viewer runs co-located with the logger to avoid double buffering; otherwise set an explicit fraction/size to prevent unbounded buffering for late subscribers.

### Phase 2: Improve Image Logging

**Goal**: Use proper Rerun archetypes for camera data.

#### 2.1 Tensor vs DepthImage vs Image Decision

**Status:** Resolved - Keeping `Tensor` archetype for 16-bit scientific imaging.

**Analysis:**
- `DepthImage` is semantically incorrect for intensity data (it's for depth/distance measurements)
- `Image` only supports 8-bit grayscale (Y8), not 16-bit
- `Tensor` is the correct choice for 16-bit scientific camera data (Prime BSI)

**Current implementation** (rerun_sink.rs):
```rust
Measurement::Image { width, height, buffer, metadata, .. } => {
    let shape = vec![*height as u64, *width as u64];
    match buffer {
        PixelBuffer::U16(data) => {
            let tensor_data = rerun::TensorData::new(shape, rerun::TensorBuffer::U16(data.clone().into()));
            let tensor = Tensor::new(tensor_data)
                .with_dim_names(["height", "width"]);  // Added for better visualization
            let _ = rec.log(entity_path.clone(), &tensor);
        }
        PixelBuffer::U8(data) => {
            let tensor_data = rerun::TensorData::new(shape, rerun::TensorBuffer::U8(data.clone().into()));
            let tensor = Tensor::new(tensor_data)
                .with_dim_names(["height", "width"]);
            let _ = rec.log(entity_path.clone(), &tensor);
        }
        _ => {}
    }
    // Log image metadata as separate scalars
    Self::log_image_metadata(rec, &entity_path, metadata);
}
```

**Key improvements made:**
- Added `.with_dim_names(["height", "width"])` for better Rerun viewer interpretation
- Added metadata logging (exposure, gain, temperature, readout, binning)

#### 2.2 Consider EncodedImage for Bandwidth Reduction

For scenarios with limited bandwidth:

```rust
// Optional: JPEG compression for bandwidth-limited scenarios
let encoded = rerun::archetypes::EncodedImage::from_file("frame.jpg")?;
rec.log("/camera/image", &encoded)?;
```

**Trade-off**: ~100x smaller file sizes vs. lossless quality.

### Phase 3: Multi-Sink Recording

**Goal**: Enable simultaneous visualization and recording.

#### 3.1 Implement Multi-Sink Support

The Rerun SDK provides `set_sinks()` for streaming to multiple destinations simultaneously. From the official docs example:

```rust
// Official Rerun example: docs/snippets/all/howto/set_sinks.rs
use rerun::sink::{GrpcSink, FileSink};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create recording with multiple sinks using tuple syntax
    let rec = rerun::RecordingStreamBuilder::new("rust-daq").set_sinks((
        // Connect to a local viewer using the default URL
        GrpcSink::default(),
        // Write data to a file simultaneously
        FileSink::new("recording.rrd")?,
    ))?;

    // All logged data goes to both sinks
    rec.log("camera/image", &depth_image)?;

    Ok(())
}
```

**For RerunSink integration:**

```rust
use rerun::sink::{GrpcSink, FileSink};

impl RerunSink {
    /// Create a new RerunSink with optional file recording
    pub fn new_with_recording(
        recording_path: Option<&Path>,
    ) -> Result<Self> {
        let rec = if let Some(path) = recording_path {
            // Multi-sink: gRPC viewer + file recording
            RecordingStreamBuilder::new(APP_ID).set_sinks((
                GrpcSink::default(),
                FileSink::new(path)?,
            ))?
        } else {
            // Single sink: gRPC only
            RecordingStreamBuilder::new(APP_ID).set_sinks((
                GrpcSink::default(),
            ))?
        };

        Ok(Self { rec })
    }

    /// Create a server that remote viewers can connect to
    pub fn new_server_with_recording(
        bind_ip: &str,
        port: u16,
        recording_path: Option<&Path>,
    ) -> Result<Self> {
        // For serve_grpc mode, we need to use serve_grpc_opts
        // Multi-sink with serve_grpc requires manual sink setup
        let builder = RecordingStreamBuilder::new(APP_ID);

        let rec = builder.serve_grpc_opts(
            bind_ip,
            port,
            re_grpc_server::ServerOptions::default(),
        )?;

        // Note: For serve_grpc + file, you may need to handle this differently
        // as serve_grpc creates its own sink internally

        Ok(Self { rec })
    }
}
```

**Supported sink tuples** (from Rerun source):
- Single: `(GrpcSink,)` or `(FileSink,)`
- Dual: `(GrpcSink, FileSink)`
- Up to 6 sinks supported via tuple syntax

### Phase 4: Native Rust Blueprints

**Goal**: Eliminate Python dependency for blueprint generation.

**Status Note** (from Rerun docs): "Blueprints are currently an experimental part of the Rust SDK."

#### 4.1 Rust Blueprint API (Experimental)

The Rust Blueprint API uses `ViewBlueprint` and related types:

```rust
// crates/daq-server/src/blueprints.rs
use rerun::blueprint::archetypes::ViewBlueprint;
use rerun::blueprint::components::{SpaceViewClass, SpaceViewOrigin, Visible};

impl RerunSink {
    /// Log blueprint configuration for DAQ visualization
    pub fn configure_blueprint(&self) -> Result<()> {
        // Blueprints are logged as static data to a special path
        self.rec.log_static(
            "blueprint/views/camera",
            &ViewBlueprint::new("2D")  // Space view class
                .with_display_name("Camera Live")
                .with_space_origin("/camera"),
        )?;

        self.rec.log_static(
            "blueprint/views/timeseries",
            &ViewBlueprint::new("TimeSeries")
                .with_display_name("Measurements")
                .with_space_origin("/device"),
        )?;

        Ok(())
    }
}
```

**Current Approach (Recommended)**: Continue using Python-generated `.rbl` files until the Rust Blueprint API stabilizes. The current system works well:

```rust
// Current working approach - load pre-generated blueprint
let sink = RerunSink::new()?;
sink.load_blueprint("crates/daq-server/blueprints/daq_default.rbl")?;
```

**Migration path:**
1. Keep Python blueprint generation for now (it works)
2. Monitor Rerun releases for Blueprint API stabilization
3. Migrate to Rust blueprints when API is stable

**Files to remove after Rust Blueprint API stabilizes:**
- `crates/daq-server/blueprints/generate_blueprints.py`
- `crates/daq-server/blueprints/*.rbl` (generated files)
- `scripts/regenerate_blueprints.sh`

### Phase 5: Optimize Memory and Latency

**Goal**: Fine-tune Rerun SDK for real-time camera streaming.

#### 5.1 Understand Micro-Batching

From Rerun docs: "Internally, the stream will automatically micro-batch multiple log calls to optimize transport. The SDK now defaults to 8ms long microbatches instead of 50ms. This makes the default behavior more suitable for use-cases like real-time video feeds."

Key insight: The 8ms default is already optimized for real-time video. Fine-tuning may not be necessary.

```rust
// The SDK handles micro-batching automatically
// For GrpcSink, you can configure flush_timeout:
use rerun::sink::GrpcSink;

let grpc_sink = GrpcSink::new_with_flush_timeout(
    "rerun+http://127.0.0.1:9876/proxy",
    Some(Duration::from_millis(4)),  // Lower for reduced latency
)?;
```

**Note**: `flush_timeout` is the minimum time the GrpcSink will wait during a flush. Setting it too low can increase CPU usage; too high increases latency.

#### 5.2 Memory Limit for gRPC Server

```rust
use re_grpc_server::ServerOptions;
use re_memory::MemoryLimit;

// Prevent unbounded memory growth
let opts = ServerOptions {
    memory_limit: MemoryLimit::from_bytes(512 * 1024 * 1024),  // 512 MB
    ..Default::default()
};

// CRITICAL: If server & client on same machine, use 0B to avoid double-buffering
let opts_same_machine = ServerOptions {
    memory_limit: MemoryLimit::from_bytes(0),
    ..Default::default()
};

let rec = RecordingStreamBuilder::new(APP_ID)
    .serve_grpc_opts("0.0.0.0", 9876, opts)?;
```

**Memory limit behavior** (from Rerun docs):
- Once limit reached, earliest logged data is dropped
- Static data is **never** dropped (blueprints, calibration, etc.)
- Set to 0B when server and client are on same machine

#### 5.3 Consider VideoStream for High-FPS (Future)

For sustained high-framerate streaming, Rerun now supports `VideoStream` (H.264/H.265):

```rust
// VideoStream for encoded video streaming (H.264 only currently)
use rerun::archetypes::VideoStream;

// Note: Adds ~0.5s latency due to encoding/decoding
// Reduces bandwidth by ~20x (1080p: 10GB/min → 500MB/min)
// B-frames not yet supported

// For now, DepthImage/Image is recommended for scientific cameras
// VideoStream is better for compressed video streams
```

**When to use VideoStream:**
- Network bandwidth is severely limited
- Acceptable latency of ~0.5s
- Standard video codecs (H.264) are suitable

**When to use Image/DepthImage (current approach):**
- Scientific imaging requiring lossless data
- Low-latency requirements (<50ms)
- 16-bit depth data

### Phase 6: Simplify GUI Architecture

**Goal**: Clean separation between control and visualization.

#### 6.1 Understanding Rerun Viewer Connection Patterns

From the Rerun source code (`re_grpc_client/src/read.rs`), connecting to a remote Rerun server:

```rust
// re_grpc_client::stream() - How viewer connects to remote servers
pub fn stream(uri: re_uri::ProxyUri) -> re_log_channel::LogReceiver {
    // Creates a LogReceiver that streams from the remote gRPC server
    let (tx, rx) = re_log_channel::log_channel(LogSource::MessageProxy(uri.clone()));
    // ... async streaming implementation
    rx
}
```

#### 6.2 Two Approaches for Embedded Viewer

**Approach A: Spawn local gRPC server (current approach)**

From `examples/rust/custom_callback/src/viewer.rs`:

```rust
// Current pattern - GUI hosts its own gRPC server
let rx_log = re_grpc_server::spawn_with_recv(
    "0.0.0.0:9876".parse()?,
    Default::default(),
    re_grpc_server::shutdown::never(),
);

let mut rerun_app = re_viewer::App::new(
    main_thread_token,
    re_viewer::build_info(),
    app_env,
    startup_options,
    cc,
    None,
    re_viewer::AsyncRuntimeHandle::from_current_tokio_runtime_or_wasmbindgen()?,
);

// Add the receiver - viewer will display data sent to :9876
rerun_app.add_log_receiver(rx_log);
```

**Approach B: Connect to remote gRPC server (proposed)**

```rust
// Connect to daemon's Rerun server instead of hosting our own
use re_grpc_client;
use re_uri::ProxyUri;

// Parse the remote URI
let uri = ProxyUri::parse("rerun+http://100.117.5.12:9876/proxy")?;

// Create a LogReceiver that streams from the remote server
let rx_log = re_grpc_client::stream(uri);

let mut rerun_app = re_viewer::App::new(
    main_thread_token,
    re_viewer::build_info(),
    app_env,
    startup_options,
    cc,
    None,
    re_viewer::AsyncRuntimeHandle::from_current_tokio_runtime_or_wasmbindgen()?,
);

// Add the remote receiver - viewer displays data from daemon
rerun_app.add_log_receiver(rx_log);
```

#### 6.3 Refactored main_rerun.rs

```rust
// Simplified architecture - GUI connects to remote daemon's Rerun server
use re_grpc_client;
use re_uri::ProxyUri;
use rerun::external::{eframe, egui, re_viewer, re_log, re_crash_handler, re_memory};

pub struct DaqRerunApp {
    rerun_app: re_viewer::App,
    daq_client: Option<DaqClient>,  // Control commands only (:50051)
    runtime: tokio::runtime::Runtime,
}

impl DaqRerunApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        main_thread_token: re_viewer::MainThreadToken,
        daemon_rerun_uri: &str,  // e.g., "rerun+http://100.117.5.12:9876/proxy"
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        // Connect to daemon's Rerun gRPC server
        let uri = ProxyUri::parse(daemon_rerun_uri)?;
        let rx_log = re_grpc_client::stream(uri);

        let mut rerun_app = re_viewer::App::new(
            main_thread_token,
            re_viewer::build_info(),
            re_viewer::AppEnvironment::Custom("DAQ Control Panel".to_owned()),
            re_viewer::StartupOptions::default(),
            cc,
            None,
            re_viewer::AsyncRuntimeHandle::from_current_tokio_runtime_or_wasmbindgen()?,
        );

        // Add remote data stream - NO local gRPC server needed!
        rerun_app.add_log_receiver(rx_log);

        Ok(Self {
            rerun_app,
            daq_client: None,
            runtime,
        })
    }
}
```

**Key insight**: The GUI doesn't need to handle frame data at all. It uses `re_grpc_client::stream()` to connect to the daemon's Rerun server and receives visualization data automatically. No custom `StreamFrames` RPC needed!

## Migration Checklist

### Pre-Migration

- [x] Update Rerun dependency to latest stable (0.27+)
- [x] Review Rerun changelog for breaking changes since 0.27.3
- [x] Set up test environment with remote camera access

### Phase 1: gRPC Consolidation (COMPLETE)

- [x] Modify `RerunSink` to use `serve_grpc_opts()`
- [x] Update daemon startup to configure Rerun server address
- [x] Remove `StreamFrames` from proto definitions
- [x] Remove `stream_frames` from `HardwareService`
- [x] Remove frame streaming code from `main_rerun.rs`
- [x] Update GUI to connect to daemon's Rerun server via `re_grpc_client::stream()`
- [x] Test remote camera visualization

### Phase 2: Image Improvements (COMPLETE)

- [x] Evaluated `Tensor` vs `Image`/`DepthImage` - kept Tensor (correct for 16-bit scientific imaging)
- [x] Add timestamps/resolution metadata via `log_image_metadata()`
- [x] Added dimension names to Tensor (`.with_dim_names(["height", "width"])`)
- [x] Batching/compression handled by Rerun SDK defaults (8ms microbatching)
- [ ] Consider `EncodedImage` if bandwidth becomes tight (deferred - not needed currently)
- [x] Test visualization quality

### Phase 3: Multi-Sink (COMPLETE)

- [x] Implement recording + visualization via `new_server_with_recording()`
- [x] Recording path passed as optional parameter
- [x] Test .rrd file generation

### Phase 4: Rust Blueprints (BLOCKED)

- [ ] Implement Rust blueprint API - **Waiting for stable API (see rerun-io/rerun#5521)**
- [ ] Migrate existing blueprints
- [ ] Remove Python generation code
- [ ] Test all blueprint configurations

**Current approach:** Continue using Python-generated `.rbl` files until Rust API stabilizes.

### Phase 5: Optimization (COMPLETE)

- [x] Microbatching uses Rerun defaults (8ms, optimized for real-time video)
- [x] Memory limits: 0 bytes for same-machine, 25% RAM for remote
- [x] Added heartbeat logging (`log_heartbeat()`, `start_heartbeat_task()`)
- [x] Documented flush configuration via `RERUN_FLUSH_TICK_SECS` env var
- [ ] Evaluate VideoStream if needed (deferred - not needed for scientific imaging)

### Phase 6: GUI Simplification (COMPLETE)

- [x] Refactor `DaqRerunApp` to remove frame handling (camera_device_id removed)
- [x] Clean up unused code
- [x] Update documentation

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Rerun gRPC API changes | Low | Medium | Pin version, review changelog |
| Network latency over SSH tunnel | Medium | Medium | Optimize batching, test latency |
| Blueprint API not feature-complete | Medium | Low | Fall back to Python if needed |
| Memory issues with 8MB frames | Low | High | Configure memory limits, test |
| Breaking embedded viewer | Medium | High | Keep fallback code path |

## Performance Expectations

| Metric | Current | Target |
|--------|---------|--------|
| Frame latency | ~100ms | <50ms |
| Memory (GUI) | 500MB+ | <300MB |
| Network (frames) | 8MB/frame raw | 8MB (no change) or 400KB compressed |
| Code complexity | 2 gRPC channels | 1 unified channel |

## Key Insights from Rerun Documentation

### Operating Mode Selection

| Mode | When to Use | Our Use Case |
|------|-------------|--------------|
| `spawn()` | Local development, single-user | Current default |
| `connect_grpc()` | Client connecting to existing server | GUI connecting to daemon |
| `serve_grpc()` | Server hosting data for remote clients | **Daemon should use this** |
| `save()` | File recording only | Optional recording |

**Recommendation**: Daemon uses `serve_grpc()`, GUI uses `connect_grpc()`.

### URL Format (Rerun 0.23+)

```
rerun+http://{host}:{port}/proxy   # Unencrypted
rerun://{host}:{port}/proxy        # TLS encrypted
```

Default port: 9876

### Image Archetype Selection

| Archetype | Use Case | Our Use Case |
|-----------|----------|--------------|
| `Image` | RGB/grayscale photos (8-bit only) | - |
| `DepthImage` | Depth/distance cameras | - (not intensity data) |
| `EncodedImage` | JPEG/PNG compressed | Bandwidth-limited (deferred) |
| `Tensor` | N-dimensional data, 16-bit scientific imaging | **PVCAM camera (Prime BSI)** |
| `VideoStream` | H.264/H.265 video | High-FPS compressed (deferred) |

**Decision rationale:** `Tensor` is correct for 16-bit scientific camera data. `DepthImage` is semantically incorrect (designed for depth/distance measurements, not intensity). `Image` only supports 8-bit grayscale.

### Memory Management Best Practices

1. **Same machine**: Set `memory_limit: 0B` to avoid double-buffering
2. **Remote clients**: Use `MemoryLimit::from_fraction_of_total(0.25)`
3. **Static data**: Never dropped (blueprints, calibration)
4. **Dynamic data**: Oldest dropped first when limit reached

### Threading Safety

From Rerun docs: "RecordingStream can be cheaply cloned and used freely across any number of threads. Internally, all operations are linearized into a pipeline."

This means our current `rec.clone()` pattern is correct and efficient.

### Deployment: Remote Daemon Setup

When running the daemon on a remote machine (e.g., `maitai@100.117.5.12`) with the GUI on a local machine:

**1. Firewall Configuration (Required)**

Open both gRPC ports on the remote machine:
```bash
# Using firewall-cmd (RHEL/Fedora/CentOS)
sudo firewall-cmd --add-port=9876/tcp --permanent   # Rerun gRPC
sudo firewall-cmd --add-port=50051/tcp --permanent  # DAQ gRPC
sudo firewall-cmd --reload

# Using iptables (alternative)
sudo iptables -I INPUT -p tcp --dport 9876 -j ACCEPT
sudo iptables -I INPUT -p tcp --dport 50051 -j ACCEPT
```

**2. Environment Variables (Optional)**

The daemon supports configurable Rerun server binding:
```bash
export RERUN_BIND="0.0.0.0"  # Default: bind to all interfaces
export RERUN_PORT="9876"     # Default: standard Rerun port
```

**3. Launch GUI with Remote URL**
```bash
export RERUN_URL="rerun+http://100.117.5.12:9876/proxy"
cargo run -p daq-egui --bin daq-rerun --features rerun_viewer
```

**4. Tailscale Note**

If using Tailscale (100.x.x.x IP range), you may need to add an iptables rule to the `ts-input` chain to allow your Tailscale IP:
```bash
sudo iptables -I ts-input 1 -s YOUR_TAILSCALE_IP -j ACCEPT
```

### PVCAM Feature Flag Configuration

**Critical:** The `pvcam_hardware` feature must propagate through the entire dependency chain to enable real camera hardware. Without proper propagation, the driver falls back to mock mode (gradient pattern).

**Feature Chain (all must be enabled):**
```
rust_daq/pvcam_hardware
  → daq-hardware/pvcam_hardware
    → daq-driver-pvcam/pvcam_hardware
      → pvcam-sys/pvcam-sdk (links to libpvcam.so)
```

**Cargo.toml Configuration:**

1. `crates/rust-daq/Cargo.toml`:
   ```toml
   pvcam_hardware = ["daq-hardware/pvcam_hardware"]
   ```

2. `crates/daq-hardware/Cargo.toml`:
   ```toml
   pvcam_hardware = ["driver_pvcam", "daq-driver-pvcam/pvcam_hardware"]
   ```

3. `crates/daq-driver-pvcam/Cargo.toml`:
   ```toml
   pvcam_hardware = ["dep:pvcam-sys", "pvcam-sys/pvcam-sdk"]
   ```

**Build Command (Remote Machine):**
```bash
source /etc/profile.d/pvcam.sh
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
export PVCAM_SDK_DIR=/opt/pvcam/sdk
cargo build -p rust_daq -p daq-bin --release \
    --features 'rust_daq/pvcam_hardware,rust_daq/instrument_photometrics'
```

**Verification:**

1. Check feature resolution:
   ```bash
   cargo metadata --format-version 1 --features 'rust_daq/pvcam_hardware' \
     | jq '.resolve.nodes[] | select(.id | test("daq-driver-pvcam")) | .features'
   # Should show: ["default", "mock", "pvcam_hardware"]
   ```

2. Verify binary links to PVCAM:
   ```bash
   ldd target/release/rust-daq-daemon | grep pvcam
   # Should show: libpvcam.so.2 => /opt/pvcam/library/x86_64/libpvcam.so.2
   ```

3. Runtime log check (set `RUST_LOG=info,daq_driver_pvcam=debug`):
   ```
   PvcamDriver::new_async called for camera: PMUSBCam00
   pvcam_hardware feature enabled: true
   Initializing PVCAM SDK...
   PVCAM SDK initialized, opening camera: PMUSBCam00
   Camera opened successfully, handle: Some(0)
   ```

**Troubleshooting Mock Mode:**

If you see gradient patterns instead of real camera data:
1. Feature chain broken - check all Cargo.toml files above
2. Build cached without feature - run `cargo clean -p daq-driver-pvcam -p daq-hardware`
3. `conn.handle().is_none()` - camera open failed, check PVCAM_SDK_DIR and camera name
4. PVCAM_VERSION env var missing - causes Error 151 at runtime

**Known Issues:**
- bd-bgmv: Ring buffer contention during high-throughput streaming - FIXED
  - Increased default timeout from 100ms to 500ms
  - Made timeout configurable via `DAQ_RINGBUFFER_TIMEOUT_MS` environment variable
  - Changed log level from `error!` to `warn!` (non-fatal timeout)

**Ring Buffer Configuration for High-Throughput Streaming:**
If you see `read_snapshot timed out` warnings during camera streaming, increase the timeout:
```bash
export DAQ_RINGBUFFER_TIMEOUT_MS=1000  # Increase to 1 second for very high frame rates
```
The default is 500ms which handles most scenarios. For 30+ fps with 2048x2048 frames, consider 1000ms or higher.

## egui 0.33 Upgrade (January 2026)

The daq-rerun binary was blocked by egui version conflicts. Rerun 0.27.3 requires egui 0.33, but our codebase was on egui 0.31. This was resolved by upgrading the entire egui ecosystem:

### Dependency Changes

| Package | Old Version | New Version |
|---------|-------------|-------------|
| eframe | 0.31 | 0.33 |
| egui | 0.31 | 0.33 |
| egui_extras | 0.31 | 0.33 |
| egui_dock | 0.16 | 0.18 |
| egui_plot | 0.31 | 0.34 |

**Note:** egui_plot 0.33 uses egui 0.32 (not 0.33), so we had to use egui_plot 0.34.

### API Breaking Changes

18 mechanical changes were required across 6 files:

1. **`close_menu()` → `close()`** (14 occurrences)
   - app.rs, main_rerun.rs, analog_input.rs, instrument_manager.rs

2. **`Line/VLine/HLine::new(data)` → `Line/VLine/HLine::new(name, data)`** (4 occurrences)
   - oscilloscope.rs, signal_plotter.rs

3. **`menu::bar()` deprecated** (2 occurrences, kept as-is)
   - app.rs, main_rerun.rs - works but shows deprecation warning

### Verification

Both binaries build and run successfully:
```bash
# Standalone GUI
cargo build --bin rust-daq-gui --features standalone

# Integrated Rerun GUI
cargo build --bin daq-rerun --features rerun_viewer
```

Verified on remote hardware (maitai) with Prime BSI camera streaming via Rerun gRPC.

## References

- [Rerun SDK Operating Modes](https://rerun.io/docs/reference/sdk/operating-modes)
- [Rerun Rust API](https://docs.rs/rerun/latest/rerun/)
- [Rerun 0.24 Release - Multi-Sink](https://rerun.io/blog/release-0.24)
- [RecordingStreamBuilder](https://docs.rs/rerun/latest/rerun/struct.RecordingStreamBuilder.html)
- [Rerun Video Documentation](https://rerun.io/docs/reference/video)
- [GitHub: rerun-io/rerun](https://github.com/rerun-io/rerun)
- [Rerun SDK Micro-Batching](https://www.rerun.io/docs/reference/sdk/micro-batching)
- [DepthImage Archetype](https://rerun.io/docs/reference/types/archetypes/depth_image)

## Appendix: Current vs Proposed Code Comparison

### Frame Logging (Current)

```rust
// main_rerun.rs - Lines 278-312 (current implementation)
let tensor = match frame.pixel_format.as_str() {
    "u16_le" | "u16" => {
        let u16_data: &[u16] = bytemuck::cast_slice(&frame.pixel_data);
        let shape = vec![frame.height as u64, frame.width as u64];
        let tensor_data = rerun::TensorData::new(
            shape,
            rerun::TensorBuffer::U16(u16_data.to_vec().into()),
        );
        Tensor::new(tensor_data)
    }
    // ... more formats
};
rec.log("/camera/image", &tensor)?;
```

### Frame Logging (Proposed)

```rust
// rerun_sink.rs - Daemon-side, no GUI involvement
fn log_camera_frame(&self, frame: &CameraFrame) {
    let image = match frame.bit_depth {
        16 => rerun::archetypes::DepthImage::new(
            rerun::datatypes::TensorData::new(
                vec![frame.height as u64, frame.width as u64],
                rerun::TensorBuffer::U16(frame.data_u16().into()),
            ),
        ).with_colormap(rerun::components::Colormap::Viridis),
        8 => rerun::archetypes::Image::from_gray8(
            frame.data_u8(),
            [frame.width as u32, frame.height as u32],
        ),
    };

    let _ = self.rec.log("/camera/live", &image);
}
```

The GUI simply connects to the daemon's Rerun server and displays whatever is logged - no frame handling code needed.
