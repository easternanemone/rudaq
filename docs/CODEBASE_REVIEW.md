# Codebase Architectural Review & Friction Analysis

This document provides a deep architectural review of the `rust-daq` codebase, specifically focused on identifying technical debt, code clutter, and architectural friction points that make the repository difficult for AI coding agents to navigate.

## 1. The Context Window Problem: Monolithic Files

The most significant friction point for AI coding agents is the presence of massively monolithic files. When a single file exceeds 1,500 lines, it consumes a large portion of an AI's token context window. This leads to "attention degradation" (where the AI forgets or hallucinates parts of the file) and severely slows down tool usage.

The following files are the largest offenders (The >2.5k LOC Club):

*   `crates/ui/src/app.rs` (**3,827 lines**)
*   `crates/server/src/grpc/hardware_service/mod.rs` (**3,782 lines**)
*   `crates/experiment/src/run_engine.rs` (**3,765 lines**)
*   `crates/ui/src/panels/image_viewer/mod.rs` (**3,698 lines**)
*   `crates/driver-pvcam/src/components/acquisition/mod.rs` (**3,364 lines**)
*   `crates/driver-pvcam/src/components/features/mod.rs` (**3,343 lines**)
*   `crates/hardware/src/registry.rs` (**3,216 lines**)
*   `crates/storage/src/ring_buffer.rs` (**3,133 lines**)
*   `crates/server/src/grpc/server.rs` (**3,054 lines**)
*   `crates/driver-andor-sdk3/src/camera.rs` (**2,974 lines**)
*   `crates/ui/src/panels/image_viewer/echelle_calibration.rs` (**2,636 lines**)

**Impact:** An agent trying to fix a bug in the gRPC server or the experiment runner is forced to load >3000 lines of code just to read the file.
**Recommendation:** Break these monolithic files into smaller, domain-specific modules. For instance, `run_engine.rs` should be split into `state_machine.rs`, `executor.rs`, and `task_queue.rs`.

---

## 2. The "Kitchen Sink" Crate: `common`

The `crates/common` module suffers from severe context fragmentation. It mixes low-level core primitives required by every crate with highly specialized, complex algorithmic code.

**Current State of `common/src/`:**
*   **Low-level core:** `error.rs`, `capabilities.rs`, `observable.rs`, `parameter.rs` (Used everywhere).
*   **High-level specialized logic:** `echelle_calibration_pipeline.rs`, `echelle_simulation.rs`, `echelle_optimal_extraction.rs`, etc.

**Impact:** Any simple driver (e.g., `driver-mock`) that needs to use `common::error::Error` or `common::capabilities::Movable` must inherit and compile the massive echelle spectrograph mathematical modules. This pollutes autocomplete, increases compile times, and makes the `common` crate difficult to reason about.
**Recommendation:** Extract all `echelle_*.rs` files and related spectroscopy logic into a dedicated crate (e.g., `crates/processing-echelle` or `crates/math`). `common` should strictly contain universally shared types, traits, and error definitions.

---

## 3. Tight Coupling in the gRPC Server Layer

The `crates/server/src/grpc/` directory contains massive implementation files that mix HTTP/gRPC routing concerns with core business logic.

*   `server.rs` (3,054 lines)
*   `hardware_service/mod.rs` (3,782 lines)

**Impact:** Implementing a single new gRPC endpoint requires touching a massive, highly-trafficked file, leading to constant merge conflicts. The `hardware_service` module is particularly overgrown, likely containing raw hardware manipulation logic that should belong to the `hardware` crate or lower-level orchestrators.
**Recommendation:** 
1. The `server.rs` file should only orchestrate the `tonic` server builder.
2. Individual services should be broken out into smaller files inside `hardware_service/` (e.g., `hardware_service/camera.rs`, `hardware_service/motion.rs`) rather than placing all logic directly in `mod.rs`.

---

## 4. UI Monoliths

The `crates/ui` crate relies heavily on monolithic files, a common anti-pattern in `egui` applications where state definition, state mutation, and view rendering are all crammed into a single file.

*   `app.rs` (3,827 lines)
*   `image_viewer/mod.rs` (3,698 lines)

**Impact:** Modifying a small UI component requires parsing the entire application state and all other panel rendering functions.
**Recommendation:** Separate the application state (`struct AppState`) from the rendering logic. Create dedicated files for different visual panels (e.g., `panels/control_bar.rs`, `panels/status_bar.rs`).

---

## 5. Driver Internal Complexity

Certain hardware drivers have grown to an unmanageable size, indicating that hardware feature mapping is being hardcoded rather than abstracted.

*   `driver-pvcam/src/components/acquisition/mod.rs` (3,364 lines)
*   `driver-andor-sdk3/src/camera.rs` (2,974 lines)

**Impact:** These files are attempting to map massive, complex C/C++ SDKs (PVCAM, Andor) into Rust in single files.
**Recommendation:** Use traits or macros to map SDK features. Group related SDK features into submodules (e.g., `features/temperature.rs`, `features/triggering.rs`, `features/roi.rs`) instead of dumping hundreds of getter/setter methods into a single `mod.rs` or `camera.rs` file.

---

## Summary of Action Items

1. **Extract Echelle Math:** Create `crates/processing-echelle` and move all echelle logic out of `common`.
2. **Decompose `run_engine.rs`:** Split the experiment state machine into smaller domain modules.
3. **Decompose gRPC Services:** Move logic out of `grpc/server.rs` and `grpc/hardware_service/mod.rs` into tightly scoped, endpoint-specific files.
4. **Refactor UI Rendering:** Split `app.rs` into `state.rs` and smaller `view_*.rs` component files.
5. **Break up SDK mappings:** Split `driver-pvcam` and `driver-andor-sdk3` monoliths into feature-specific submodules.