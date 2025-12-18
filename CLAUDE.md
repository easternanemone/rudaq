# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# rust-daq Project Context

**Note**: This project uses [bd (beads)](https://github.com/steveyegge/beads)
for issue tracking. Use `bd` commands instead of markdown TODOs.
See AGENTS.md for workflow details.

Rust-based data acquisition system with V5 headless-first architecture for scientific instrumentation.

**STATUS:** V5 "headless-first" architecture implementation is complete.
Legacy V1-V4 code has been removed.

**Hardware Location:** Remote machine at `maitai@100.117.5.12`

## Quick Start

**Documentation:** See docs/getting_started/ for setup, docs/architecture/ for design decisions.

**Build & Test:**
```bash
# Build (default: storage_csv, instrument_serial)
cargo build

# Build with all features (requires native deps - see note below)
cargo build --all-features

# Run all tests (excludes hardware tests)
cargo test

# Run a single test by name
cargo test test_name -- --nocapture

# Run tests for a specific crate
cargo test -p daq-core
cargo test -p daq-hardware
cargo test -p daq-storage

# Format and lint
cargo fmt --all
cargo clippy --all-targets  # For specific features only

# Lint with all features (requires native dependencies)
# NOTE: --all-features requires PVCAM_SDK_DIR and HDF5 libraries
# Set environment first: export PVCAM_SDK_DIR=/opt/pvcam/sdk
cargo clippy --all-targets --all-features

# Run with hardware connected
cargo test --features hardware_tests

# Run Rhai script (headless mode)
cargo run -- run examples/simple_scan.rhai

# Start gRPC daemon
cargo run --features networking -- daemon --port 50051
```

## Development Tools: Rust Ecosystem Integration

This project integrates three complementary tools for comprehensive Rust development support. Each serves a distinct purpose with zero overlap:

### Tool Overview

| Tool | Type | Purpose | Key Capabilities |
|------|------|---------|------------------|
| **rust-cargo** | MCP Server | Build & Package Management | cargo commands, dependency management, toolchain control |
| **rust-analyzer** | CLI Tool | Code Diagnostics | Workspace diagnostics, error checking, inactive code detection |
| **cargo-modules** | CLI Tool | Structure Visualization | Module hierarchy, dependency graphs, orphan detection |

### 1. rust-cargo MCP Server

**Purpose:** Automate cargo/rustup commands through Claude Code

**Common Operations:**
```bash
# Available as MCP tools (Claude can invoke these):
- cargo-build: Build the project
- cargo-test: Run tests
- cargo-check: Fast compilation check
- cargo-clippy: Run linter
- cargo-fmt: Format code
- cargo-add/cargo-remove: Manage dependencies
- cargo-deny: Security/license checks
- cargo-hack: Test feature combinations
- cargo-machete: Find unused dependencies
- rustup-update: Update toolchains
```

**When to Use:**
- Building and testing code
- Managing dependencies
- Running quality checks (clippy, fmt, deny)
- Creating packages for distribution
- CI/CD automation tasks

**Example Workflow:**
```
"Run clippy on all targets"
"Add tokio with features async-std and macros"
"Update all dependencies"
"Check for security vulnerabilities with cargo-deny"
```

### 2. rust-analyzer CLI Tool

**Purpose:** Provide IDE-like code intelligence and diagnostics through command-line interface

**Status:** The rust-analyzer MCP server has been disabled due to protocol compatibility issues with Claude Code. Use the CLI directly via Bash tool instead.

**Common Operations:**
```bash
# Get workspace diagnostics (errors, warnings, hints)
rust-analyzer diagnostics .

# Analyze specific file
rust-analyzer diagnostics path/to/file.rs

# LSP server mode (for IDE integration)
rust-analyzer

# Get version
rust-analyzer --version
```

**When to Use:**
- Getting comprehensive diagnostics without building
- Quick error checking across the entire workspace
- Understanding inactive code due to `#[cfg]` directives
- Finding all compilation errors and warnings

**Example Workflow:**
```bash
# Check all diagnostics in workspace
rust-analyzer diagnostics .

# Filter for errors only
rust-analyzer diagnostics . 2>&1 | grep "Error"

# Check specific crate
cd crates/daq-core && rust-analyzer diagnostics .
```

**Output Format:**
- Progress: `N/M X% processing file.rs`
- Warnings: `WeakWarning` with line/col positions
- Errors: `Error RustcHardError` with error codes (e.g., E0432)
- Inactive code: Shows which `#[cfg]` directives caused code to be disabled

**Note:** rust-analyzer is installed as a rustup component. Ensure it's available:
```bash
rustup component add rust-analyzer
```

### 3. cargo-modules CLI Tool

**Purpose:** Visualize internal crate structure and module relationships

**Common Operations:**
```bash
# CLI commands (use via Bash tool or skill):
cargo modules structure --package <crate>    # Module hierarchy tree
cargo modules dependencies --package <crate> # Internal dependency graph
cargo modules orphans --package <crate>      # Find unlinked files

# Useful flags:
--max-depth N         # Limit tree depth
--no-fns             # Hide functions for clarity
--focus-on module    # Focus on specific module
--all-features       # Analyze with all features enabled
```

**When to Use:**
- Pre-refactoring analysis
- Understanding crate architecture
- Detecting orphaned source files
- Documenting module structure
- Investigating circular dependencies

**Example Workflow:**
```
"Show me the module structure of daq-hardware"
"Generate a dependency graph for daq-core"
"Check for orphaned files in the workspace"
"What's the structure of daq-storage with all features?"
```

See `.claude/skills/cargo-modules.md` for comprehensive usage guide.

### Integrated Workflow Examples

#### Scenario 1: Adding a New Feature

```
1. rust-analyzer → "rust-analyzer diagnostics . | head -50" (check baseline)
2. cargo-modules → "cargo modules structure --package <crate> --max-depth 3"
3. [Write code]
4. rust-analyzer → "rust-analyzer diagnostics . 2>&1 | grep Error" (check errors)
5. rust-cargo → "Run clippy and fmt"
6. rust-cargo → "Run tests"
```

#### Scenario 2: Refactoring a Crate

```
1. cargo-modules → "cargo modules structure --package daq-storage > before.txt"
2. cargo-modules → "cargo modules dependencies --package daq-storage > deps-before.dot"
3. rust-analyzer → "cd crates/daq-storage && rust-analyzer diagnostics . > before-diag.txt"
4. [Perform refactoring]
5. cargo-modules → "cargo modules structure --package daq-storage > after.txt"
6. rust-analyzer → "cd crates/daq-storage && rust-analyzer diagnostics . 2>&1 | grep Error"
7. Bash → "diff before.txt after.txt"
8. rust-cargo → "cargo-test --package daq-storage"
```

#### Scenario 3: Debugging Compilation Errors

```
1. rust-analyzer → "rust-analyzer diagnostics . 2>&1 | grep Error" (find all errors)
2. rust-analyzer → "rust-analyzer diagnostics . > full-diagnostics.txt" (full report)
3. rust-cargo → "cargo-check for detailed compiler messages"
4. Grep → Search codebase for error-related symbols
5. rust-cargo → "cargo-clippy for additional suggestions"
```

#### Scenario 4: Dependency Management

```
1. rust-cargo → "Add new dependency: cargo-add tokio --features full"
2. cargo-modules → "Check how new dependency affects structure"
3. rust-cargo → "Run cargo-machete to check for newly unused deps"
4. rust-cargo → "Run cargo-deny check for security issues"
5. rust-cargo → "Update Cargo.lock: cargo-update"
```

### Tool Selection Guide

**Use rust-cargo when you need to:**
- Execute cargo commands (build, test, check)
- Manage dependencies (add, remove, update)
- Run quality tools (clippy, fmt, deny, hack)
- Perform package operations

**Use rust-analyzer when you need to:**
- Get comprehensive workspace diagnostics without building
- Check for compilation errors and warnings quickly
- See inactive code due to feature flags
- Understand cfg-conditional compilation

**Use cargo-modules when you need to:**
- Visualize module hierarchy
- Understand architectural relationships
- Generate dependency graphs
- Find orphaned source files

### Best Practices

1. **Start with Structure:** Use `cargo-modules` before major refactoring
2. **Quick Diagnostics:** Use `rust-analyzer diagnostics . 2>&1 | grep Error` for fast error checking
3. **Automated Quality:** Run `cargo-clippy` and `cargo-fmt` via `rust-cargo` regularly
4. **Feature Testing:** Use `cargo-hack` to test all feature combinations
5. **Security Scanning:** Run `cargo-deny` before releases
6. **Workspace Awareness:** All three tools understand this multi-package workspace
7. **Filter Output:** rust-analyzer can be verbose; pipe to `grep`, `head`, or save to file

### Troubleshooting

**rust-cargo issues:**
- Ensure cargo/rustup are installed and in PATH
- Check that working directory is project root
- Verify rust-mcp-server is installed: `cargo install rust-mcp-server`

**rust-analyzer issues:**
- Ensure rust-analyzer is installed: `rustup component add rust-analyzer`
- If diagnostics take too long, analyze specific crates: `cd crates/<name> && rust-analyzer diagnostics .`
- Some errors in examples/benches are expected if optional features are disabled
- Output can be verbose; use `grep`, `head`, or redirect to file for filtering

**cargo-modules issues:**
- Always specify `--package` in multi-package workspaces
- Install if missing: `cargo install cargo-modules`
- Use filters (`--no-fns`, `--no-types`) for large crates

## Documentation Search

**Hybrid search** is available for codebase exploration:
```bash
# Semantic search across 55+ indexed docs (architecture, guides, instruments)
python scripts/search_hybrid.py --query "your question" --mode auto
```

Re-index after updating docs: `python cocoindex_flows/comprehensive_docs_index.py`

## Architecture Overview (V5)

**Design Philosophy:** Headless-first + scriptable control + remote GUI

**Crate Structure:**
- `crates/daq-core/` — Domain types, parameters/observables, error handling, and module domain types.
- `crates/daq-hardware/` — Hardware Abstraction Layer (HAL), capability traits (`Movable`, `Readable`, `FrameProducer`, etc.), and drivers.
- `crates/daq-driver-pvcam/` — PVCAM camera driver (requires SDK on target machine).
- `crates/daq-proto/` — Protobuf definitions and tonic build; proto sources in `proto/` with domain↔proto conversions.
- `crates/daq-server/` — gRPC server implementation (optional dependency).
- `crates/daq-experiment/` — RunEngine and Plan definitions for experiment orchestration.
- `crates/daq-scripting/` — Rhai scripting engine integration (optional dependency).
- `crates/daq-storage/` — Data persistence (CSV, HDF5, Arrow formats), ring buffers.
- `crates/daq-egui/` — GUI application using egui.
- `crates/rust-daq/` — **Integration layer** providing `prelude` module for organized imports. Feature-gates optional components (server, scripting). Import directly from focused crates (bd-232k refactoring complete).
- `crates/daq-bin/` — Binaries and CLI entrypoints.
- `crates/daq-examples/` — Example code and usage patterns.

**Dependency Graph:**
```
                           daq-core (foundation)
                               ↑
                ┌──────────────┼──────────────┬─────────────┐
                │              │              │             │
         daq-driver-pvcam  daq-proto    daq-storage   daq-experiment
                │                            ↑             ↑
                └────────→ daq-hardware ─────┴─────────────┤
                               ↑                           │
                ┌──────────────┼───────────────────────────┘
                │              │
          daq-scripting*   daq-server*
          (optional)       (optional)
                │              ↑
                │         daq-bin, daq-egui
                │
           rust-daq (integration layer with prelude)

* = Optional dependency, enabled via feature flags
```

**Visualize actual dependencies:** See "Development Tools" section above for using cargo-modules

**Legend:**
- `daq-core`: Foundation types, errors, parameters, observables
- `daq-hardware`: HAL + drivers (depends on daq-driver-pvcam conditionally)
- `daq-experiment`: RunEngine and Plans (depends on daq-core, daq-hardware)
- `daq-scripting` (optional): Rhai integration (depends on daq-core, daq-experiment, daq-hardware)
- `daq-server` (optional): gRPC server (depends on daq-core, daq-hardware, daq-proto, daq-scripting, daq-storage)
- `daq-storage`: Data persistence (depends on daq-core, daq-proto)
- `daq-proto`: Protobuf definitions (depends on daq-core)
- `daq-bin`: CLI binaries (depends on daq-hardware, daq-proto, daq-server)
- `daq-egui`: GUI application (depends on daq-core, daq-driver-pvcam, daq-proto)
- `rust-daq`: Integration layer with `prelude` module for organized re-exports; feature-gates optional dependencies

**Key Components (by crate):**
- **Capability Traits & Hardware Registry**: Re-exported through `rust-daq::prelude` from `daq-hardware` (e.g., `Movable`, `Readable`, `FrameProducer`, `Triggerable`, `ExposureControl`).
- **Hardware Drivers**: Feature-gated drivers (ell14, esp300, pvcam, maitai, newport_1830c) compiled via `daq-hardware` and surfaced through `rust-daq::hardware`.
- **Module Domain Types**: `ModuleState`, `ModuleEvent`, etc. live in `crates/daq-core/src/modules.rs`; proto equivalents are mapped in `crates/daq-proto/src/convert.rs`.
- **Data Plane**: Ring buffer (sync and async), HDF5 writer, CSV storage, and related services in `daq-storage`.
- **Scripting Engine** (optional): Rhai-based control lives under `crates/daq-scripting/`. Enabled with `scripting` feature.
- **gRPC Remote Control** (optional): Enabled with `server` and `networking` features; proto assets come from `daq-proto`, server implementation in `daq-server`.

**Configuration:** Figment-based V4 config in `config/config.v4.toml` (see `crates/rust-daq/src/config_v4.rs`).

## bd-232k Refactoring (COMPLETE)

**Epic Goal:** Refactor monolithic `rust-daq` integration crate to eliminate architectural ambiguity and dead code.

**Status:** ✅ Complete (3 phases, -3,023 lines of dead code)

**Key Outcomes:**

1. **Import Clarity** (Phase 1):
   - Created `prelude` module with organized re-exports grouped by functional area
   - Deprecated root-level re-exports (`rust_daq::core`, `rust_daq::error`, etc.)
   - Migrated all examples, tests, benches, docs, tools to use `rust_daq::prelude::*` or direct crate imports
   - Tagged as `phase-1-bd-232k-complete` (commit af82baae)

2. **Optional Dependencies** (Phase 2):
   - Made `daq-server` and `daq-scripting` optional dependencies
   - Added explicit `server` and `scripting` feature flags
   - Feature-gated re-exports in `lib.rs` and `prelude.rs`
   - Updated high-level profiles (`cli`, `modules`) to include `scripting` dependency
   - Tagged as `phase-2-bd-232k-optional-deps` (commit d492ff1e)

3. **Dead Code Elimination** (Phase 3):
   - Deleted dead modules: `data/`, `metadata.rs`, `session.rs`, `measurement/`
   - Replaced `rust_daq::data` imports with direct `daq_storage` usage
   - Updated `daq-examples` to depend on `daq-storage` directly
   - Removed 3,023 lines of dead code
   - Tagged as `phase-3-bd-232k-dead-code-removal` (commit 63f43cf6)

**Architecture After bd-232k:**

- `rust-daq` is now an **integration layer** (not a monolithic crate)
- Provides `prelude` module for convenient organized imports
- Feature-gates optional components (`server`, `scripting`)
- **Best Practice**: Import directly from focused crates (`daq_storage`, `daq_hardware`, etc.) or use `rust_daq::prelude::*` for convenience

**Migration Guide:**

```rust
// OLD (deprecated, will be removed in 0.6.0):
use rust_daq::core;
use rust_daq::error::DaqError;
use rust_daq::data::ring_buffer::RingBuffer;

// NEW (recommended):
use rust_daq::prelude::*;          // Organized re-exports
// or direct imports:
use daq_core::core;
use daq_core::error::DaqError;
use daq_storage::ring_buffer::RingBuffer;
```

**Cargo.toml Changes:**

```toml
# If using scripting or server, enable explicitly:
[dependencies]
rust_daq = { path = "../rust-daq", features = ["scripting", "server"] }

# Or use high-level profiles:
rust_daq = { path = "../rust-daq", features = ["backend"] }  # includes server
rust_daq = { path = "../rust-daq", features = ["cli"] }      # includes scripting
```

## Feature Flags

**High-Level Profiles:**
- `backend` - Server, modules, all hardware, CSV storage
- `frontend` - GUI (egui) + networking
- `cli` - All hardware, CSV storage, Python scripting
- `full` - Most features (excludes HDF5 which needs native libs)

**Storage Backends:**
- `storage_csv` (default), `storage_hdf5` (requires libhdf5), `storage_arrow`, `storage_matlab`, `storage_netcdf`

**Hardware Drivers (daq-hardware):**
- `serial` / `tokio_serial` (default) - Serial port support
- `driver-thorlabs` - ELL14 rotators
- `driver-newport` - ESP300 motion controller
- `driver-spectra-physics` - MaiTai laser
- `driver_pvcam` - PVCAM camera support
- `pvcam_hardware` - Real PVCAM hardware (requires SDK)
- `full` - All drivers + simulator

**Hardware Drivers (rust-daq):**
- `instrument_serial` (default), `instrument_visa`, `tokio_serial`
- `instrument_thorlabs`, `instrument_newport`, `instrument_photometrics`
- `instrument_spectra_physics`, `instrument_newport_power_meter`
- `all_hardware` - Enable all V5 hardware drivers

**System:**
- `modules` - Module system (depends on `scripting`)
- `scripting` - Rhai scripting engine integration (optional, makes `daq-scripting` available)
- `networking` - gRPC networking layer
- `server` - gRPC server implementation (optional, includes `networking` and makes `daq-server` available)
- `gui_egui` - egui-based GUI application
- `hardware_tests` - Hardware-in-the-loop tests
- `plugins_hot_reload` - Hot reload for plugin configs

## Thorlabs ELL14 Rotator Setup

**CRITICAL: Always query device for calibration!**

Each ELL14 unit has device-specific calibration stored in firmware. The hardcoded default (398.2222 pulses/degree) may not match your device.

```rust
// CORRECT: Query device for actual calibration
let driver = Ell14Driver::new_async_with_device_calibration("/dev/ttyUSB0", "0").await?;

// AVOID: Uses hardcoded default that may not match your hardware
let driver = Ell14Driver::new_async("/dev/ttyUSB0", "0").await?;
```

**Protocol Note:** The `IN` command returns `PULSES/M.U.` (pulses per measurement unit). For rotation stages, M.U. = degrees, so this value is pulses/degree directly.

**Multidrop Bus:** Multiple ELL14 devices can share one serial port:
```rust
let shared = Ell14Driver::open_shared_port("/dev/ttyUSB0")?;
let rotator_2 = Ell14Driver::with_shared_port(shared.clone(), "2");
let rotator_3 = Ell14Driver::with_shared_port(shared.clone(), "3");
```

## PVCAM Camera Setup (maitai@100.117.5.12)

**CRITICAL:** PVCAM Prime BSI camera requires environment variables in `/etc/profile.d/pvcam.sh`:

```bash
export PVCAM_VERSION="3.10.2.5-12.6.1623.1.887.1.1.118"
export PVCAM_UMD_PATH="/opt/pvcam/drivers/user-mode"
export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH
```

**Without `PVCAM_VERSION`:** Error 151 (PL_ERR_INSTALLATION_CORRUPTED) occurs at runtime.
**Without `LIBRARY_PATH`:** Rust linker fails with "cannot find -lpvcam".

**Running PVCAM hardware tests:**
```bash
source /etc/profile.d/pvcam.sh
export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH
export PVCAM_SDK_DIR=/opt/pvcam/sdk
cargo test --features 'instrument_photometrics,pvcam_hardware,hardware_tests' \
  --test hardware_pvcam_validation -- --nocapture --test-threads=1
```

**Camera:** Prime BSI (GS2020 sensor), 2048x2048 pixels, PVCAM 3.10.2.5

## Remote Hardware Testing

All hardware tests must pass on the remote machine after mock tests pass locally.

**Quick SSH test command:**
```bash
ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
  export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
  cd ~/rust-daq && cargo test --features hardware_tests -- --nocapture --test-threads=1'
```

**Serial port inventory on remote:**
| Device | Port | Driver |
|--------|------|--------|
| Newport 1830-C Power Meter | `/dev/ttyS0` | `Newport1830CDriver` |
| MaiTai Laser | `/dev/ttyUSB5` | `MaiTaiDriver` |
| ELL14 Rotators (addr 2,3,8) | `/dev/ttyUSB0` | `Ell14Driver` |
| ESP300 Motion Controller | `/dev/ttyUSB1` | `Esp300Driver` |

## Critical Code Pattern: Reactive Parameters (V5)

**DO NOT** use raw `Arc<RwLock<T>>` or `Mutex<T>` for device state - creates "Split Brain" where gRPC clients can't observe hardware changes.

**USE** `Parameter<T>` with async hardware callbacks:

```rust
use crate::parameter::Parameter;
use futures::future::BoxFuture;

pub struct MyDriver {
    port: Arc<Mutex<SerialStream>>,
    wavelength_nm: Parameter<f64>,
}

impl MyDriver {
    pub async fn new_async(port_path: &str) -> Result<Self> {
        let port = Arc::new(Mutex::new(/* open serial */));

        let wavelength = Parameter::new("wavelength_nm", 800.0)
            .with_range(690.0, 1040.0)
            .connect_to_hardware_write({
                let port = port.clone();
                move |val: f64| -> BoxFuture<'static, Result<()>> {
                    Box::pin(async move {
                        port.lock().await.write_all(
                            format!("WAVELENGTH:{}\r\n", val).as_bytes()
                        ).await?;
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Ok(())
                    })
                }
            });

        Ok(Self { port, wavelength_nm: wavelength })
    }
}

#[async_trait]
impl Movable for MyDriver {
    async fn move_abs(&self, position: f64) -> Result<()> {
        self.position_param.set(position).await  // Delegates to parameter
    }
}
```

**Key Points:**
- `BoxFuture<'static, Result<()>>` for async hardware callbacks
- `Parameter::set()` validates BEFORE hardware write, then broadcasts changes
- **CRITICAL:** Validation happens BEFORE hardware write to prevent invalid device states
- Trait methods delegate to `param.set()`, NOT hardware directly
- **WARNING:** Use `tokio::time::sleep`, NOT `std::thread::sleep` in callbacks

## Critical Code Pattern: Async Ring Buffer

The synchronous `RingBuffer::read_snapshot()` can block during contention. In async contexts, use `AsyncRingBuffer`:

```rust
use daq_storage::ring_buffer::{RingBuffer, AsyncRingBuffer};

// Wrap in AsyncRingBuffer for async-safe access
let ring = Arc::new(RingBuffer::create(1024 * 1024)?);
let async_ring = AsyncRingBuffer::new(ring);

// These methods use spawn_blocking internally
let snapshot = async_ring.read_snapshot().await;
async_ring.write(&data).await?;
```

## Project Management: Issue Tracking (MANDATORY)

**REQUIRED:** All work MUST be tracked using beads (bd) issue tracker.

**At session start:**
```bash
bd ready              # Check available work
bd show <issue-id>    # View issue details
bd update <id> --status in_progress
```

**During development:**
- Create issues proactively when discovering new work: `bd create "Description"`
- Link dependencies: `bd dep add <from> <to>`
- Update design notes: `bd update <id> --design "Implementation notes"`

**When completing:**
```bash
bd close <id> --reason "What was done"
bd ready              # See unblocked work
```

**Database:** `.beads/issues.jsonl` (JSON Lines format)

**Why:** Complex architectural migrations require persistent issue tracking across sessions.

## Common Pitfalls

1. **Feature Mismatches:** Many compilation errors = missing features. Check Cargo.toml for required features. Use `cargo modules structure --package <crate> --all-features` to visualize feature-specific code (see "Development Tools" section).

2. **Async Context:** All methods are async. Ensure tokio runtime context is available.

3. **Lock-Across-Await:** NEVER hold `tokio::sync::Mutex` guards across `.await` points. Extract data, drop guard, then await:
   ```rust
   // WRONG: Holds lock across await
   let guard = mutex.lock().await;
   do_something(guard.value).await;  // Deadlock risk!

   // CORRECT: Extract and drop before await
   let value = { mutex.lock().await.clone() };
   do_something(value).await;
   ```

4. **Floating-Point Truncation:** When converting degrees/positions to pulses, use `.round()`:
   ```rust
   // WRONG: Truncates (45.0 * 398.2222 = 17919.999 → 17919)
   let pulses = (degrees * pulses_per_degree) as i32;

   // CORRECT: Rounds to nearest (17919.999 → 17920)
   let pulses = (degrees * pulses_per_degree).round() as i32;
   ```

5. **ELL14 Calibration:** Always use `new_async_with_device_calibration()` to query the device for its actual pulses/degree value. The hardcoded default may not match your hardware.

6. **Hardware Testing:** Real hardware requires `hardware_tests` feature + physical devices + drivers.

7. **PVCAM Environment:** Missing `PVCAM_VERSION` env var causes Error 151 at runtime.

8. **Ring Buffer Blocking:** `RingBuffer::read_snapshot()` uses `std::thread::sleep` during contention. Use `AsyncRingBuffer` wrapper in async code, or wrap calls in `spawn_blocking`.

9. **Scripting Soft Limits:** When creating `StageHandle` for scripts, configure `SoftLimits` to prevent hardware damage from script errors.

10. **FFI Safety:** PVCAM and other C library calls should use wrapper functions with explicit safety contracts and `debug_assert!` for invariant checking.

## Documentation Structure

```
docs/
├── architecture/       # ADRs, architectural decisions
├── getting_started/    # Setup and onboarding
├── guides/            # How-to guides (deployment, testing, CI/CD)
├── instruments/       # Hardware-specific protocols
└── project_management/# Project planning
```

---

**For ByteRover memory system usage, see global CLAUDE.md instructions.**

**Testing Protocol:** All hardware tests should be performed in mock-mode first, but once those mock-mode tests pass, the tests *MUST* be performed on the *REAL HARDWARE* located on the remote machine (maitai@100.117.5.12).
