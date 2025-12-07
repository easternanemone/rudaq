# Headless-First & Scriptable Architecture - Complete Specification

**Status**: ✅ V5 COMPLETE (V1/V2/V3/V4 Removed)
**Date**: 2025-11-18 (Original), 2025-12-07 (Updated)
**Master Epic**: bd-oq51

> **Note**: This document describes the completed V5 architecture. All phases have been
> implemented. V1/V2/V3/V4 code has been removed from the codebase.

## Executive Summary

The rust-daq project is undergoing a complete architectural restructuring to become a **headless-first, scriptable DAQ system** that surpasses existing frameworks (DynExp, PyMODAQ, ScopeFoundry) by leveraging Rust's performance advantages while maintaining the flexibility scientists expect.

**Core Innovation**: Split the system into a crash-resilient daemon (Rust) and flexible UI (any client), connected via gRPC, with hot-swappable experiment logic via Rhai scripting.

## Four Key Differentiators

### 1. Crash Resilience
**Problem**: In traditional monolithic apps (DynExp/PyMODAQ), a GUI crash kills the entire experiment.

**Solution**: Strict daemon/client separation
- **Core Daemon** (rust-daq-core): Owns hardware, runs experiments autonomously
- **Client UI**: Separate process, can crash/disconnect without affecting experiment

**Result**: Scientists can monitor long-running scans remotely without fear of accidental closures.

### 2. Hot-Swappable Logic
**Problem**: Compiled languages require edit-compile-run cycle, slow for iterative science.

**Solution**: Embedded Rhai scripting engine
- Scientists write experiment logic in `.rhai` files
- Upload and execute without recompiling Rust binary
- Safety: Automatic termination of infinite loops (10k operation limit)

**Example**:
```rhai
// experiment.rhai - No Rust compilation needed!
for i in 0..100 {
    stage.move_abs(i * 0.1);
    camera.trigger();
    sleep(0.05);
}
```

### 3. Time-Travel Debugging
**Problem**: HDF5 files locked during acquisition, can't analyze while running.

**Solution**: Memory-mapped ring buffer
- **Arrow IPC** format in shared memory (fast, zero-copy)
- Last N minutes always available in RAM
- **HDF5 translation** in background for final storage
- Python/Julia can attach and read live (via pyarrow)

**The Mullet Strategy**:
- **Arrow in Front**: Ring buffer uses Arrow (10k+ writes/sec, lock-free)
- **HDF5 in Back**: Storage Actor translates to HDF5 (scientist compatibility)
- **Scripts Never See Arrow**: Only exposed to f64/Vec<f64> (simple types)

### 4. Atomic Capabilities
**Problem**: Monolithic `Camera` trait assumes all cameras have all features → runtime errors.

**Solution**: Composable capability traits
```rust
trait Movable: Send + Sync {
    async fn move_abs(&self, pos: f64) -> Result<()>;
}

trait Triggerable: Send + Sync {
    async fn trigger(&self) -> Result<()>;
}

// Generic experiments work with ANY compatible hardware
fn scan<T: Movable, C: Triggerable>(stage: T, camera: C) {
    // Compiler guarantees stage can move and camera can trigger
}
```

**Result**: Compile-time safety, generic experiment code, easy hardware swapping.

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ Core Daemon (rust-daq-core) - Rust Binary                  │
├─────────────────────────────────────────────────────────────┤
│  ┌───────────────┐  ┌────────────────┐  ┌───────────────┐  │
│  │ Hardware      │→ │ Ring Buffer    │→ │ HDF5 Writer   │  │
│  │ Manager       │  │ (Arrow IPC)    │  │ (Background)  │  │
│  │               │  │                │  │               │  │
│  │ • ESP300      │  │ Shared Memory  │  │ Arrow→HDF5    │  │
│  │ • PVCAM       │  │ /dev/shm       │  │ Translation   │  │
│  │ • MaiTai      │  │                │  │               │  │
│  └───────────────┘  └────────────────┘  └───────────────┘  │
│          ↑                   ↓                              │
│  ┌───────────────┐  ┌────────────────┐                     │
│  │ Rhai Script   │  │ gRPC Server    │                     │
│  │ Engine        │  │ :50051         │                     │
│  │               │  │                │                     │
│  │ Safety:       │  │ ControlService │                     │
│  │ 10k op limit  │  │ (Upload/Start) │                     │
│  └───────────────┘  └────────────────┘                     │
└─────────────────────────┬───────────────────────────────────┘
                          │ gRPC/WebSocket
                          ↓
        ┌─────────────────────────────────────────┐
        │ Client (Tauri/WebAssembly/Python)       │
        ├─────────────────────────────────────────┤
        │  • Dashboard UI (real-time plots)       │
        │  • Script Editor (upload .rhai files)   │
        │  • Time-Travel Viewer (scrub timeline)  │
        └─────────────────────────────────────────┘
```

## Implementation Roadmap (4 Phases)

### Phase 1: Core Clean-Out (Weeks 1-2)
**Objective**: Delete V1/V2/V4, stabilize on capability-based architecture.

**Epic**: bd-9s4c (Phase 1: Core Clean-Out)

**Tasks**:
- **Task A** (bd-9si6): The Reaper - Delete Legacy Architectures
  - Remove src/app_actor.rs, v4-daq/, crates/daq-core/
  - Remove Kameo dependency
  - **Errors Fixed**: 50%+ reduction (from 87 to < 45)

- **Task B** (bd-bm03): Trait Consolidation - Define Atomic Capabilities
  - Create src/hardware/capabilities.rs
  - Define: `Movable`, `Triggerable`, `FrameProducer`, `Readable`
  - Migrate from V3 traits

- **Task C** (bd-wsaw): Mock Driver Implementation
  - Implement MockStage, MockCamera
  - Use tokio::time::sleep (not blocking)
  - Comprehensive async tests

**Success Criteria**: Project compiles with < 50 errors, capability traits defined.

### Phase 2: Scripting Engine (Weeks 3-4)
**Objective**: Run hardware loops without recompiling Rust.

**Epic**: bd-z3l8 (Phase 2: Scripting Engine)

**Tasks**:
- **Task D** (bd-jypq): Rhai Setup and Integration
  - Add rhai dependency
  - Create ScriptHost wrapper
  - Safety callback (10k operation limit)

- **Task E** (bd-m9bs): Hardware Bindings for Rhai
  - Bridge async Rust ↔ sync Rhai (tokio::task::block_in_place)
  - Expose: move_abs(), trigger(), sleep()
  - Register StageHandle, CameraHandle types

- **Task F** (bd-hiu6): CLI Rewrite for Script Execution
  - `rust-daq run experiment.rhai`
  - `rust-daq daemon --port 50051`
  - Example scripts in examples/

**Success Criteria**: Scientist can write .rhai script, execute without recompiling.

### Phase 3: Network Layer (Weeks 5-6)
**Objective**: Separate UI from Core with gRPC communication.

**Epic**: bd-679l (Phase 3: Network Layer)

**Tasks**:
- **Task G** (bd-3z3z): API Definition with Protocol Buffers
  - Create src/network/proto/daq.proto
  - Define ControlService (Upload/Start/StreamStatus)
  - Configure tonic-build

- **Task H** (bd-8gsx): gRPC Server Implementation
  - Implement ControlService trait
  - Upload script validation
  - Background script execution
  - WebSocket streaming (100ms updates)

- **Task I** (bd-2kon): Client Prototype (Python)
  - clients/python/daq_client.py
  - grpcio integration
  - End-to-end remote script execution

**Success Criteria**: Python client uploads script, monitors remotely.

### Phase 4: Data Plane (Weeks 7+)
**Objective**: High-performance zero-copy data streaming.

**Epic**: bd-4i9a (Phase 4: Data Plane)

**Tasks**:
- **Task J** (bd-q2we): Memory-Mapped Ring Buffer Implementation
  - #[repr(C)] header for cross-language compat
  - Atomic operations (lock-free)
  - Arrow IPC format
  - Python pyarrow reader

- **Task K** (bd-fspl): HDF5 Background Writer
  - THE MULLET STRATEGY: Arrow→HDF5 translation
  - Background thread (1 Hz flush)
  - No blocking of hardware loop
  - Scientists get standard .h5 files

**Success Criteria**: 10k+ writes/sec, zero-copy Python access, HDF5 output.

## Agent Delegation

**6 Specialized Agents** for parallel development:

| Agent | Epic | Responsibilities | Tasks Assigned |
|-------|------|------------------|----------------|
| **Cleaner** | bd-1ioz | Delete V1/V2/V4 code | Task A (bd-9si6) |
| **Architect** | bd-2g4l | Define capability traits | Task B (bd-bm03) |
| **Driver** | bd-407t | Mock hardware implementation | Task C (bd-wsaw) |
| **Scripting** | bd-r9mw | Rhai integration | Tasks D,E,F (bd-jypq, bd-m9bs, bd-hiu6) |
| **Network** | bd-ni8q | gRPC server/client | Tasks G,H,I (bd-3z3z, bd-8gsx, bd-2kon) |
| **Data** | bd-b9hf | Ring buffer + HDF5 | Tasks J,K (bd-q2we, bd-fspl) |

## Critical Design Decisions

### Decision 1: Rhai (Not Python Embedding)
**Rationale**:
- No GIL contention with hardware threads
- Synchronous API (easier mental model for scientists)
- Rust integration (no FFI boundaries)
- Safety: Sandboxed execution with automatic termination

**Tradeoff**: Scientists must learn Rhai syntax (but very similar to Rust/Python).

### Decision 2: Arrow for Ring Buffer (Not HDF5)
**Rationale**:
- HDF5 has global lock → single-threaded writes → jitter at 1kHz+
- Arrow is just memory layout → zero overhead
- Lock-free reads (multiple clients can attach)

**Tradeoff**: Complexity hidden via translation layer (scientists never see Arrow).

### Decision 3: gRPC (Not REST/JSON)
**Rationale**:
- Type-safe contracts (protobuf)
- Streaming support (WebSocket alternative)
- Code generation (Rust + Python clients)
- HTTP/2 multiplexing

**Tradeoff**: Slightly steeper learning curve vs REST API.

## Comparison to Target Frameworks

| Feature | DynExp/PyMODAQ/ScopeFoundry | rust-daq (Headless-First) |
|---------|----------------------------|---------------------------|
| **Architecture** | Monolithic Desktop App | Daemon + Client (crash-resilient) |
| **Flexibility** | Python Scripts (slow loops) | Rust Core + Rhai Scripts (fast loops) |
| **Reliability** | GUI crash kills experiment | Daemon survives GUI crash |
| **Data Access** | File-based (locked) | Memory-mapped (zero-copy live access) |
| **Safety** | Runtime errors (Python) | Compile-time capability checks (Rust) |
| **Remote Access** | VNC/TeamViewer (laggy) | Native gRPC/WebSocket |
| **Performance** | Python/Qt overhead | Rust zero-cost abstractions |
| **Storage** | HDF5 (standard) | HDF5 (Arrow translation layer) |

## Success Criteria

### Immediate (Weeks 1-2)
- [x] Headless-First master epic created (bd-oq51)
- [ ] Great Flattening deprecated (bd-kal8 closed)
- [ ] V1/V2/V4 deleted (Task A complete)
- [ ] Capability traits defined (Task B complete)
- [ ] Compilation errors < 50 (from 87)

### Mid-term (Weeks 3-6)
- [ ] Rhai scripts control mock hardware
- [ ] CLI: `rust-daq run experiment.rhai` works
- [ ] gRPC server running, Python client connects
- [ ] Remote script upload/execution working

### Long-term (Weeks 7-12)
- [ ] Ring buffer operational (10k+ writes/sec)
- [ ] HDF5 translation layer working
- [ ] Python pyarrow can read live data
- [ ] GUI can time-travel through data
- [ ] End-to-end: Remote client → Script → Hardware → Storage

### Production (Weeks 12+)
- [ ] Hardware drivers migrated to capabilities (ESP300, PVCAM, MaiTai)
- [ ] Real experiment workflows validated
- [ ] Performance benchmarks: < 1ms script→hardware latency
- [ ] Documentation: Scientist onboarding guide
- [ ] Community: Example scripts repository

## Risk Mitigation

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Rhai too slow for tight loops | Medium | High | Keep tight loops in Rust; Rhai only orchestrates |
| Arrow complexity leaks to users | Low | Medium | Strict abstraction: Scripts see f64, not Arrow |
| gRPC learning curve | Low | Low | Provide Python/JS client libraries |
| Ring buffer race conditions | High | Critical | Extensive atomic operation testing |
| Scientists resist Rhai syntax | Medium | Medium | Provide PyO3 alternative in parallel |

## Documentation Roadmap

**Week 1**: `/docs/headless/`
- [x] phase1_core_cleanout.md (Tasks A,B,C)
- [x] phase2_scripting_engine.md (Tasks D,E,F)
- [x] phase3_network_layer.md (Tasks G,H,I)
- [x] phase4_data_plane.md (Tasks J,K)
- [x] agent_delegation.md (6 agent epics)
- [x] HEADLESS_FIRST_ARCHITECTURE.md (this file)

**Week 2**: `/examples/`
- [ ] simple_scan.rhai - Basic stage movement
- [ ] triggered_acquisition.rhai - Camera triggering
- [ ] multi_instrument.rhai - Coordinated control

**Week 4**: `/docs/guides/`
- [ ] scripting_guide.md - Rhai syntax for scientists
- [ ] hardware_integration.md - Adding new drivers
- [ ] remote_control.md - Using Python client

**Week 8**: `/docs/architecture/`
- [ ] ring_buffer_design.md - Memory layout details
- [ ] grpc_api_reference.md - Full API documentation
- [ ] capability_traits.md - Trait composition patterns

## Team Communication

**Message to Contributors**:

The V1/V2/V3/V4 "Quad-Core Schism" has been resolved. We are now executing a concrete, week-by-week implementation plan toward a modern headless-first architecture.

**Key Changes**:
1. **Delete aggressively**: V1/V2/V4 are gone. No backward compatibility.
2. **Rhai scripts**: Experiments are scripts, not compiled Rust.
3. **Daemon/client split**: UI can crash without killing hardware.
4. **Arrow→HDF5 translation**: Fast internally, compatible externally.

**Work Assignment**:
- See bd-oq51 (master epic) for full roadmap
- Agents: Check your delegation epic (bd-1ioz through bd-b9hf)
- All new work follows phase structure (Phase 1→2→3→4)

**Status**: Phase 1 ready to begin. Tasks A,B,C are the critical path.

---

**Last Updated**: 2025-11-18
**Total Issues Created**: 1 master epic + 4 phase epics + 11 tasks + 6 agent epics = 22 issues
**Priority**: All P0 (critical path)
**Next Milestone**: Task A (The Reaper) - Delete V1/V2/V4
