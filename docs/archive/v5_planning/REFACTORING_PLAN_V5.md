# Refactoring Plan: Rust DAQ V5 Architecture

**Epic Reference:** `bd-37tw` (Refactor rust-daq Monolith)

This document serves as the architectural blueprint for refactoring the `rust-daq` project from a monolithic crate into a modular, layered system. It is based on a deep analysis of the codebase and architectural patterns observed in the reference `planmeca` repository.

**Status:** Living Document
**Last Updated:** 2025-12-09

> **⚠️ UPDATE (Dec 2025):** The `bd-232k` epic has completed major portions of this refactoring plan:
> - ✅ `rust-daq` transformed into thin integration layer (removed 6,877 lines of implementation code)
> - ✅ Created `prelude` module for organized re-exports
> - ✅ Made `daq-server` and `daq-scripting` optional dependencies
> - ✅ GUI separated into `daq-egui` crate
> - ✅ Dead code removed (data/, metadata.rs, session.rs, measurement/, procedures/, gui_main.rs)
> - ⚠️ Feature flag duplication partially addressed (high-level profiles created)
> - ⏳ Remaining work: Further binary specialization
>
> See ARCHITECTURE.md "Code Smells & Recommendations" section for current status.

---

## 1. Architectural Vision

The goal is to transition `rust-daq` from a "Kitchen Sink" monolith (containing hardware, gRPC, config, and app logic) into a set of focused, independent crates. This improves:
-   **Build Times:** Compiling the core doesn't require compiling heavy vendor SDKs.
-   **Testability:** Drivers and logic can be tested in isolation.
-   **Maintainability:** Clear separation of concerns prevents domain leakage.

### Target Crate Structure

```text
/crates
  ├── daq-core/             # (Existing) Pure data types, traits, errors.
  ├── daq-proto/            # (New) Generated Protobuf + FlatBuffers + type mappers.
  ├── daq-hardware/         # (New) Registry, capabilities, drivers, plugin loader, resource pool.
  ├── daq-storage/          # (New) Ring buffer, writers, storage factory, tap registry.
  ├── daq-scripting/        # (New) ScriptEngine trait, Rhai/PyO3 backends, plan bindings.
  ├── daq-driver-*/         # (New) Heavy drivers (e.g., daq-driver-pvcam) isolated by SDK.
  ├── daq-server/           # (New) gRPC server + orchestration; depends only on core/proto/hardware/storage/scripting.
  ├── rust-daq/             # (Legacy/Shell) Thin facade; should shrink or be retired.
  └── daq-bin/              # (Existing) CLI / entrypoint depending on daq-server or headless stack.
```

---

## 2. Architectural Pillars

### A. Pipeline Pattern (Data Flow)
*Reference Task: `bd-37tw.7`*

**Current Problem:** Data uses `broadcast::channel` in a fan-out topology. Backpressure is hard to manage, and processing chains are ad-hoc.

**Target Pattern (Inspired by `planmeca/frame-traits`):**
Adopt a formal `Source` -> `Processor` -> `Sink` model in `daq-core`.

*   **`MeasurementSource`:** Active producer (Driver, Simulation).
*   **`MeasurementProcessor`:** Pure transformer (FFT, Filter, Resizer).
*   **`MeasurementSink`:** Final consumer (Network, Disk, Display).

**Implementation:**
-   Connect nodes via bounded `mpsc` channels for strict backpressure.
-   Allow explicit pipeline construction at runtime.

### B. Driver Componentization
*Reference Task: `bd-vk11.1`*

**Current Problem:** Drivers like `PvcamDriver` are monolithic structs implementing multiple traits, mixing FFI, threading, and logic.

**Target Pattern (Inspired by `planmeca/emerald`):**
Use **Composition over Inheritance**. The Driver struct becomes a container for specialized components.

*   **`PvcamConnection`:** Handles SDK handles, initialization, and keep-alives.
*   **`PvcamAcquisition`:** Manages high-speed threads, circular buffers, and polling.
*   **`PvcamCooling`:** Manages thermal control logic.

The main driver struct simply delegates trait calls to these components.

### C. Layered Protocol Separation
*Reference Task: `bd-37tw.4` (daq-proto)*

**Current Problem:** `rust-daq` mixes domain types (`DataPoint`) with generated Proto types.

**Target Pattern:**
-   **`daq-proto`**: Contains *only* `build.rs` for tonic/prost and the generated code.
-   **`daq-core`**: Contains the canonical Rust types (`struct Measurement`).
-   **Conversion:** `From`/`Into` traits live in `daq-proto` or `daq-server`, keeping `daq-core` dependency-free of heavy frameworks like Tonic.

---

## 3. Execution Phases

### Phase 1: Hygiene & Foundation (COMPLETED)
-   [x] Remove binary blobs (`*.bin`) from repo root and update .gitignore (`bd-g2hr`).
-   [x] Unify `Roi` and `DataPoint` types into `daq-core` (`bd-kli4`, `bd-z9rk`).
    -   Verified: no duplicate structs; all uses import from `daq-core`.

### Phase 2: Protocol Extraction & Boundary Decoupling (COMPLETED 2025-12-09)
**Focus:** Break tonic coupling BEFORE extracting other crates.

> **Done:** Module APIs no longer depend on tonic types; proto assets live in their own crate.

1.  **daq-proto (`bd-37tw.4`)** [P0]
    - ✅ Proto sources now live in `crates/daq-proto/proto/` with tonic build in `crates/daq-proto/build.rs`.
    - ✅ Domain↔proto mappers live beside the generated code in `crates/daq-proto/src/convert.rs`.
    - Dep graph: daq-proto -> daq-core; consumers import proto + converters from daq-proto.

2.  **Decouple modules/registry from tonic types (`bd-37tw.6`)** [P0]
    - ✅ Domain equivalents now reside in `crates/daq-core/src/modules.rs` (ModuleState, ModuleEvent, ModuleDataPoint, etc.).
    - ✅ Module context/registry call sites updated to use `daq_core::modules` domain types; conversions stay in daq-proto.
    - ✅ Feature flag updated: `modules = []` (no networking dependency).

### Phase 3: Crate Extraction (Core layering)
**Focus:** Split along domain boundaries; keep deps directional.
**Prerequisite:** Phase 2 complete (modules decoupled from tonic).

1.  **daq-hardware (`bd-37tw.1`)** [P1]
    - Move registry, capabilities, drivers, plugin loader, resource pool.
    - Keep feature flags: drivers-serial, drivers-visa, plugins_hot_reload.
2.  **daq-storage (`bd-37tw.2`)** [P2]
    - Move ring_buffer, hdf5_writer, storage_factory, tap_registry.
    - Feature flags per backend: storage-csv/arrow/hdf5/netcdf.
3.  **daq-scripting (`bd-37tw.3`)** [P2]
    - Move RhaiEngine, ScriptEngine trait, plan bindings, script_runner tools.
    - Optional `scripting-python` feature for PyO3 backend.
4.  **daq-server (`bd-7llb`)** [P2]
    - Host gRPC services + orchestration; depend on core/proto/hardware/storage/scripting only.
    - Remove tonic usage from other crates; keep boundary conversions here.
5.  **Cargo workspace & CI**
    - Update workspace members and `[patch]` paths.
    - Add CI jobs per crate (fast matrix: core + proto; full matrix optional features).

### Phase 4: Hardware Isolation
**Focus:** Isolate heavy vendor dependencies.

1.  **Extract `daq-driver-pvcam` (`bd-vk11`)** [P3]
    -   Move `crates/rust-daq/src/hardware/pvcam.rs` and `pvcam-sys` dependency to new crate.
    -   Refactor using **Componentization** (`bd-vk11.1`): connection/acquisition/cooling components.
    -   Dep graph: daq-driver-pvcam -> daq-hardware (capability traits) -> daq-core.
2.  **Other drivers**
    - Optionally split serial drivers later if compile times remain high.

### Phase 5: Feature Matrix Simplification
**Focus:** Make builds predictable and fast.

-   **Feature set redesign (`bd-37tw.5`)** [P1]
    - Defaults: minimal headless (`transport`, `mock-hw`, `storage-csv`).
    - Groups: `transport` (tonic/tonic-web), `gui`, `drivers-serial`, `drivers-pvcam`, `storage-hdf5`, `storage-arrow`, `scripting-python`.
    - Remove umbrellas `full`/`all_hardware`; document combos; update CI matrix.
    - Source of truth: `docs/architecture/FEATURE_MATRIX.md`

### Phase 6: Pipeline Adoption
**Focus:** Modernize data flow (optional follow-on once splits land).

1.  **Define Traits (`bd-37tw.7`)** [P2]
    -   Add `Source`/`Processor`/`Sink` traits to `daq-core`.
2.  **Refactor Server**
    -   Update `daq-server` to build pipelines instead of just broadcasting.

---

## 4. Developer Guidelines

-   **Do not add code to `rust-daq/src/lib.rs`**. This file should shrink, not grow.
-   **Domain Types First:** Always define data structures in `daq-core` first.
-   **Proto Separation:** Never put business logic in Protobuf generated files.
-   **Check `beads`:** Always check `bd ready` before starting work to see the active task in the graph.

---

## 5. Implementation Checklist (per phase)

**General**
- [ ] `cargo fmt && cargo clippy --all-targets` per crate after each move.
- [ ] Update `Cargo.toml` features + workspace members.
- [ ] Add/adjust CI jobs (fast vs full feature sets).

**Phase 2 (Protocol extraction & boundary decoupling) [COMPLETED 2025-12-09]**
- [x] `bd-37tw.4`: Extract `daq-proto` (proto + build.rs now in `crates/daq-proto/`; converters in `crates/daq-proto/src/convert.rs`).
- [x] `bd-37tw.6`: Decouple modules from tonic types.
    - [x] Domain types live in `crates/daq-core/src/modules.rs`.
    - [x] Module call sites import `daq_core::modules::*` (no tonic types).
    - [x] `IntoProto`/`FromProto` conversions housed in `crates/daq-proto/src/convert.rs`.
    - [x] Feature updated: `modules = []` (no networking dependency).
    - [x] Verify: `cargo check -p rust_daq --no-default-features --features modules`

**Phase 3 (Crate extraction)**
- [ ] `bd-37tw.1`: Extract `daq-hardware`.
- [ ] `bd-37tw.2`: Extract `daq-storage`.
- [ ] `bd-37tw.3`: Extract `daq-scripting`.
- [ ] `bd-7llb`: Extract `daq-server`.
- [ ] Add `pub use` re-exports only where stable API is desired.
- [ ] `cargo test -p daq-proto -p daq-hardware -p daq-storage -p daq-scripting -p daq-server`.

**Phase 4 (PVCAM split)**
- [ ] `bd-vk11`: New crate `crates/daq-driver-pvcam`; gate with `drivers-pvcam`.
- [ ] `bd-vk11.1`: Componentize driver; add unit tests for connection/acquisition/cooling components.

**Phase 5 (Features)**
- [ ] `bd-37tw.5`: Document new matrix in `docs/architecture/FEATURE_MATRIX.md`.
- [ ] Update examples and README build commands.

**Phase 6 (Pipeline pattern)**
- [ ] `bd-37tw.7`: Define `Source`/`Processor`/`Sink` traits in daq-core.
- [ ] Refactor data flow in daq-server to use pipeline topology.

---

## 6. Dependency Graph (Issue Execution Order)

```
Phase 1 (COMPLETED)
├── bd-g2hr ✓
├── bd-kli4 ✓
└── bd-z9rk ✓

Phase 2 (COMPLETED)
├── bd-37tw.4 ✓ ← Extract daq-proto
└── bd-37tw.6 ✓ ← Decouple modules (depends on bd-37tw.4)

Phase 3 (Crate Extraction) - can parallelize after Phase 2
├── bd-37tw.1 [P1] ← daq-hardware (depends on bd-z9rk, bd-37tw.6)
├── bd-37tw.2 [P2] ← daq-storage
├── bd-37tw.3 [P2] ← daq-scripting
└── bd-7llb   [P2] ← daq-server (depends on bd-37tw.1, bd-37tw.4)

Phase 4 (Hardware Isolation)
├── bd-vk11   [P3] ← Extract daq-driver-pvcam
└── bd-vk11.1 [P3] ← Componentize (depends on bd-vk11)

Phase 5 (Features)
└── bd-37tw.5 [P1] ← Feature matrix (depends on bd-37tw.1, bd-7llb)

Phase 6 (Pipeline)
└── bd-37tw.7 [P2] ← Pipeline pattern (after Phase 3 complete)
```
