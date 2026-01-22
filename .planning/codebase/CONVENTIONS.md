# Coding Conventions

**Analysis Date:** 2026-01-21

## Naming Patterns

**Files:**
- Kebab-case for file names: `mock_stage.rs`, `hardware_service.rs`
- Module files use `mod.rs` for directory modules
- Private implementation files prefixed with context: `error_mapping_tests.rs`, `pattern.rs`
- Test helper files: `tests/common/mod.rs`

**Functions:**
- snake_case for all functions: `connect_to_hardware_write()`, `read_value()`, `stream_position()`
- Builder methods use `with_*` prefix: `with_description()`, `with_range()`, `with_validator()`, `with_metadata()`
- Getter methods often omit prefix: `get()`, `name()`, `position()`, `description()`
- Async operations named without special prefix, but return futures: `read()`, `set()`, `stage()`
- Hardware-specific methods are explicit: `connect_to_hardware_read()`, `read_from_hardware()`

**Variables:**
- snake_case for local variables and parameters: `device_id`, `frame_producer`, `registry`, `observer_handle`
- Underscore prefix for intentionally unused bindings: `let _ = rx.recv().await`
- Field names in structs use snake_case

**Types:**
- PascalCase for types and traits: `Observable<T>`, `Parameter<T>`, `FrameProducer`, `Readable`, `Movable`
- Generic type parameters: single uppercase letter (`T`), or descriptive (`T: Send + Sync`)
- Error types use `Error` or `DaqError` suffix
- Enum variants follow their container's naming: `ScanType::SnakeScan`

**Constants:**
- SCREAMING_SNAKE_CASE: `MAX_STREAMS_PER_CLIENT`, `RPS_TIMEOUT`, `FPS_WINDOW`
- Defined in `crate::limits` module

## Code Style

**Formatting:**
- Edition: 2021
- Default formatter: `cargo fmt --all` (Rust standard formatter)
- Line wrapping follows Rust style guidelines (100-120 char soft limit)
- Brace style: opening brace on same line (Rust convention)

**Linting:**
- Primary linter: `cargo clippy --all-targets`
- Workspace-level lints configured in `rust_daq/Cargo.toml` (lines 17-32):
  - `rust` lints:
    - `unsafe_code = "warn"` - unsafe blocks flagged for review
    - `missing_docs = "warn"` - public items should have doc comments
  - `clippy` lints:
    - `unwrap_used = "warn"` - unwrap() flagged, prefer `?` or `.map_err()`
    - `expect_used = "warn"` - expect() flagged, use Result handling
    - `panic = "warn"` - panic!() flagged for production code
    - `large_futures = "warn"` - large Future allocations flagged
    - `module_name_repetitions = "allow"` - no warning for `ModuleModule`
    - `must_use_candidate = "warn"` - functions should return values

**Quality Gates:**
- Code must pass `cargo fmt --all` (no formatting changes needed)
- Code must pass `cargo clippy --all-targets` without warnings (fixed by lints above)
- Doc comments required for public API items

## Import Organization

**Order:**
1. Standard library imports (`std::`, `core::`)
2. External crates (alphabetical): `anyhow`, `async_trait`, `futures`, `tokio`, `tonic`, `tracing`
3. Internal crates (alphabetical): `daq_core::`, `daq_hardware::`, `daq_proto::`
4. Traits and prelude imports (after concrete types)
5. Re-exports with `pub use` statements follow module declarations

**Path Aliases:**
- Use `use` statements to avoid repetition; avoid full paths in function bodies
- Group related imports: `use std::collections::{HashMap, VecDeque};`
- Crate-level re-exports in `lib.rs` provide convenient access
- Example from `rust_daq` (lib.rs lines 5-22):
  ```rust
  pub use daq_core::capabilities;
  pub use registry::{DeviceRegistry, DeviceInfo, register_all_factories};
  pub use factory::DriverFactory;
  ```

**Module Organization:**
- Public modules re-export important types
- Implementation modules can be marked `#[allow(dead_code)]` if internal utilities
- Private modules (not `pub`) hide implementation details
- Use `mod.rs` for directory modules with `pub use` to flatten API

## Error Handling

**Patterns:**
- Use `Result<T>` (alias: `AppResult<T>`) for fallible operations
- Return `DaqError` enum for all application errors (defined in `daq_core::error`)
- Use `?` operator to propagate errors
- Use `#[from]` attribute for automatic conversion: `#[error("...")]` on enum variants
- Example from `daq_core/error.rs`:
  ```rust
  #[derive(Error, Debug)]
  pub enum DaqError {
      #[error("Configuration error: {0}")]
      Config(#[from] config::ConfigError),
      #[error("Instrument error: {0}")]
      Instrument(String),
  }
  ```

**Hardware Errors:**
- Hardware operations (serial, device communication) return `Result<T, DaqError>`
- Errors map to appropriate gRPC status codes in server layer
- Transient errors (network glitch) vs. permanent (hardware failure) distinguished in error message

**Unwrap/Expect Rules:**
- **Avoid in production code** (triggers clippy warning)
- **Acceptable when:**
  - Initializing test fixtures: `let value = some_operation().unwrap()`
  - Guaranteed by logic: `result.map_err(...)?` makes success certain
  - Code documentation: comment explaining why it can't fail
- **Preferred alternatives:**
  - `ok_or_else()` for optional errors
  - `context()` (from anyhow) for error messages
  - `.unwrap_or_default()` for recoverable cases

## Logging

**Framework:** `tracing` crate (structured logging)

**Patterns:**
- Use `#[instrument]` macro on important async functions
  - Example: `#[instrument(skip(self, request), fields(method = "read_value"))]`
  - `skip()` excludes large types from logging
  - `fields()` adds custom context
- Log levels by scenario:
  - `tracing::info!()` - Important events (device staged, stream started)
  - `tracing::debug!()` - Implementation details (internal state changes)
  - `tracing::warn!()` - Recoverable issues (backpressure detected, device removed)
  - `tracing::error!()` - Unexpected failures (mutex poisoned, critical operations)
- Structured fields with `=` syntax: `tracing::info!(device_id = %device_id, "event")`
- Example from `hardware_service.rs` (lines 1826-1834):
  ```rust
  tracing::info!(
      device_id = %device_id_clone,
      exit_reason = exit_reason,
      frames_sent = frames_sent,
      frames_dropped = frames_dropped,
      client_ip = %client_ip,
      "Tap-based frame stream forwarding task ended"
  );
  ```

## Comments

**When to Comment:**
- Explain *why*, not *what* (code shows what; comments explain intent)
- Complex algorithms: outline approach before diving into code
- Workarounds and hacks: reference issue tracking ID (e.g., `bd-4x6q`)
- Non-obvious safety justifications: why a pattern is safe despite appearance
- TODO comments moved to `beads` issue tracking (not left as code comments)

**JSDoc/TSDoc/Doc Comments:**
- **Public items require doc comments** (enforced by `missing_docs = "warn"`)
- **Format:** Start with one-line summary, optionally followed by blank line and details
- **Code examples:** Use ` ```rust,ignore` block for examples (ignore flag if not compilable)
- **Error documentation:** Document what `Result<T>` can return via `#[error]` attributes
- Example from `parameter.rs` (lines 1-65):
  ```rust
  //! Parameter<T> - Declarative parameter management (ScopeFoundry pattern)
  //!
  //! Inspired by ScopeFoundry's LoggedQuantity, this module provides...
  //!
  //! # Architecture
  //!
  //! Parameter<T> **composes** Observable<T> to avoid code duplication:
  //! - Observable<T> handles: watch channels, subscriptions, validation
  //! - Parameter<T> adds: hardware write/read callbacks, change listeners
  ```

**Example Block Format:**
- Mark examples as `ignore` if they won't compile standalone
- Include doc comments on trait methods explaining behavior
- Show common usage patterns in library crate docs

## Function Design

**Size:**
- Functions should be small enough to understand at a glance (ideally < 50 lines)
- Async functions that spawn tasks may be longer (50-100 lines)
- Helper functions extract complex sub-operations

**Parameters:**
- Prefer small parameter lists (< 5 params)
- Use builder pattern for many optional configurations: `Parameter::new().with_range().with_unit()`
- Pass `Arc<T>` for shared state (owned by spawned tasks)
- Use `Request<T>` for gRPC handlers (wraps protobuf message)
- Immutable references (`&T`) preferred over mutable (`&mut T`)

**Return Values:**
- Always return `Result<T>` for fallible operations (never throw/panic in libraries)
- Use tuple returns only for related values: `(position, is_moving)`
- Use struct returns for multiple related fields (better for API stability)
- Stream return types use associated types: `type StreamPositionStream = ReceiverStream<Result<PositionUpdate, Status>>`

**Async Functions:**
- Mark with `async fn` (not `.map()` chains)
- Use `tokio::spawn()` for background tasks
- Use `#[instrument]` for tracing
- Timeouts via `tokio::time::timeout()` for external calls

## Module Design

**Exports:**
- Use `pub use` to flatten API at module level
- Hide implementation: keep driver modules private, expose via registry
- Export types needed by users; hide internal structures
- Example from `daq_driver_mock/lib.rs` (lines 38-44):
  ```rust
  pub use mock_camera::{MockCamera, MockCameraFactory};
  pub use mock_power_meter::{MockPowerMeter, MockPowerMeterFactory};
  pub use mock_stage::{MockStage, MockStageFactory};
  pub use pattern::generate_test_pattern;
  ```

**Barrel Files:**
- `lib.rs` acts as barrel/facade (re-exports public API)
- Organize exports by category (drivers, traits, config)
- Avoid exporting private implementation modules
- Use feature-gated exports for optional functionality

**Separation of Concerns:**
- Test code (`#[cfg(test)]`) stays in same file, after implementation
- Protocol definitions in separate `proto` crate (tonic/protobuf)
- Driver implementations in dedicated crates (`daq-driver-*`)
- Hardware abstraction traits in `daq-core::capabilities`

## Async/Await Patterns

**Lock Management:**
- **Never hold locks across `.await` points:**
  ```rust
  // WRONG - deadlock risk
  let guard = mutex.lock().await;
  do_something(guard.value).await;

  // CORRECT - release lock before await
  let value = { mutex.lock().await.clone() };
  do_something(value).await;
  ```
- Use `spawn_blocking()` for CPU-bound work under lock
- Prefer `tokio::sync::RwLock` for read-heavy workloads
- Use `tokio::sync::Mutex` for write workloads

**Channel Patterns:**
- Create channels with appropriate buffer: `tokio::sync::mpsc::channel(100)`
- Handle backpressure: `try_send()` with error handling vs `send().await`
- Spawn task to consume channel: `tokio::spawn(async move { loop { ... } })`
- Close by dropping sender: implicit when scope exits

**Timeouts:**
- Apply timeouts to external calls: `tokio::time::timeout(Duration, future)`
- Use `RPC_TIMEOUT` constant for consistency
- Return `Status::deadline_exceeded()` for timeout errors

---

*Convention analysis: 2026-01-21*
