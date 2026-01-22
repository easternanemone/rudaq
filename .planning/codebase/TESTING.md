# Testing Patterns

**Analysis Date:** 2026-01-21

## Test Framework

**Runner:**
- Default: `cargo test` (built-in Rust test framework)
- Recommended: `cargo nextest run` (better output, parallel execution)
- Async tests: `tokio` runtime via `#[tokio::test]` attribute
- Deterministic timing tests: `tokio::test(start_paused = true)` for simulated time

**Assertion Library:**
- Standard `assert!()` and `assert_eq!()` macros
- Custom assertions in `tests/common/mod.rs` for domain-specific checks:
  - `assert_duration_near()` - Timing assertions with tolerance
  - `TimingTolerance` enum for test environment awareness

**Run Commands:**
```bash
cargo nextest run                    # Run all tests (parallel)
cargo nextest run -p daq-core        # Run tests in specific crate
cargo nextest run test_name          # Run single test by name
cargo test --doc                     # Run doctests (doctests in comments/docs)
cargo nextest run --profile hardware # Run hardware tests on maitai machine
cargo nextest run -- --nocapture     # Show println! output during tests
```

**Environment:**
- Default runs mock hardware (no PVCAM SDK required)
- Hardware tests gated behind `#[cfg(feature = "hardware_tests")]`
- CI environment detected via `CI` env var (relaxes timing tolerances)
- Test isolation: serial_test crate for tests requiring exclusive hardware access

## Test File Organization

**Location:**
- **Unit tests:** Co-located in source files (`#[cfg(test)]` modules at end of .rs file)
- **Integration tests:** Separate files in `crates/{crate}/tests/` directory
- **Common utilities:** `crates/rust-daq/tests/common/mod.rs`
- **Hardware tests:** Separate test files with `_validation` suffix: `hardware_maitai_validation.rs`

**Naming:**
- Test function names descriptive and start with `test_`: `test_stream_position_success()`
- Test modules grouped by feature: `mod hardware_errors { }`, `mod configuration_errors { }`
- Hardware test files follow pattern: `hardware_{device}_validation.rs`
- Integration test files: `{feature}_integration.rs` or `e2e_{scenario}.rs`

**Structure:**
```
crates/rust-daq/tests/
├── common/
│   └── mod.rs                              # Shared test utilities
├── hardware_esp300_validation.rs           # Hardware-specific tests
├── hardware_maitai_validation.rs
├── elliptec_newport_integration.rs         # Integration between devices
├── grpc_camera_test.rs                     # Service integration tests
└── plugin_system_integration.rs
```

## Test Structure

**Suite Organization:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    mod configuration_errors {
        use super::*;

        #[test]
        fn test_example() {
            let err = DaqError::Config(...);
            assert_status_code(err, Code::InvalidArgument);
        }
    }

    mod hardware_errors {
        use super::*;

        #[test]
        fn test_another() {
            // ...
        }
    }
}
```

**Patterns:**

1. **Setup/Teardown:**
   - No explicit teardown needed (tests are isolated)
   - Mock objects drop automatically
   - Use `serial_test` for hardware tests needing exclusive access:
     ```rust
     #[tokio::test]
     #[serial]  // Prevents parallel execution
     async fn test_exclusive_hardware() { ... }
     ```

2. **Async Tests:**
   ```rust
   #[tokio::test]
   async fn test_async_operation() {
       let result = some_async_fn().await;
       assert_eq!(result, expected);
   }
   ```

3. **Deterministic Timing Tests (use `start_paused = true`):**
   ```rust
   #[tokio::test(start_paused = true)]
   async fn test_with_simulated_time() {
       tokio::time::sleep(Duration::from_millis(100)).await;
       // Time advances instantly, test finishes immediately
   }
   ```

4. **Real Timing Tests (use `TimingTolerance`):**
   ```rust
   use common::{assert_duration_near, TimingTolerance, env_timing_tolerance};

   #[test]
   fn test_real_timing() {
       let start = Instant::now();
       // Do something
       let elapsed = start.elapsed();
       assert_duration_near(
           elapsed,
           Duration::from_millis(100),
           env_timing_tolerance(),  // Adjusts for CI vs local
           "operation timing"
       );
   }
   ```

5. **Assertion Patterns:**
   ```rust
   // Simple equality
   assert_eq!(value, expected);

   // Custom message
   assert_eq!(value, expected, "context: value should be {}", expected);

   // Error cases
   assert!(result.is_err());
   match result {
       Err(DaqError::Instrument(msg)) => assert!(msg.contains("expected")),
       other => panic!("Unexpected result: {:?}", other),
   }

   // Option assertions
   assert!(result.is_some());
   let value = result.expect("reason it must be Some");
   ```

## Mocking

**Framework:** Manual mocking (no mockall crate)

**Patterns:**

1. **Mock Drivers (dedicated crate `daq-driver-mock`):**
   - Location: `crates/daq-driver-mock/src/`
   - Implementations:
     - `MockStage` - Simulated motion control with 10mm/sec speed, 50ms settling
     - `MockCamera` - Simulated 2D imaging at ~30fps (33ms frame readout)
     - `MockPowerMeter` - Configurable readings with 1% noise
   - Use: `#[cfg(feature = "mock")]` to enable

2. **Creating Mock Instances:**
   ```rust
   use daq_driver_mock::{MockCamera, MockStage, MockPowerMeter};

   let camera = Arc::new(MockCamera::new());
   let stage = Arc::new(MockStage::new());
   let meter = Arc::new(MockPowerMeter::new());
   ```

3. **Mock Registry Setup:**
   ```rust
   use daq_driver_mock::register_all;
   use daq_hardware::DeviceRegistry;

   let registry = DeviceRegistry::new();
   register_all(&registry);
   ```

4. **Test-Specific Behavior:**
   - Mock devices respond immediately to commands (no realistic timing unless needed)
   - Streams produce frames/values at configurable rate
   - Use with default mock feature in workspace (no SDK required)

**What to Mock:**
- Hardware devices (cameras, stages, sensors) → use `daq-driver-mock`
- Serial ports → implicit in mock drivers
- File I/O for data storage tests → use `tempfile` crate
- External services → manual wrapper structs with test implementations

**What NOT to Mock:**
- Core library types (`Parameter<T>`, `Observable<T>`)
- Async runtime (use `tokio::test`)
- Error types (test with real errors)
- Protocol definitions (test serialization with real buffers)

## Fixtures and Factories

**Test Data:**
```rust
fn create_mock_camera() -> Arc<MockCamera> {
    Arc::new(MockCamera::new())
}

fn create_mock_registry() -> DeviceRegistry {
    let registry = DeviceRegistry::new();
    register_all(&registry);
    registry
}

#[tokio::test]
async fn test_with_fixture() {
    let camera = create_mock_camera();
    let result = camera.read_frame().await;
    assert!(result.is_ok());
}
```

**Location:**
- Small fixtures: inline in test function
- Shared fixtures: `tests/common/mod.rs` module
- Complex setup: dedicated helper functions in same file

**Common Test Utilities** (`tests/common/mod.rs`):
- `TimingTolerance` enum for environment-aware assertions
- `assert_duration_near()` - Check timing with tolerance factors
- `env_timing_tolerance()` - Detect CI vs local environment
- `is_ci()` - Boolean check for CI environment
- Macros: `skip_if!()`, `skip_without_hardware!()` for conditional skipping

## Coverage

**Requirements:**
- No enforced minimum (coverage tracking is optional)
- Core library (`daq-core`) aims for high coverage (critical for downstream users)
- Driver implementations tested via integration tests

**View Coverage:**
```bash
# Using cargo-tarpaulin (install: cargo install cargo-tarpaulin)
cargo tarpaulin --out Html --output-dir coverage

# Using cargo-llvm-cov (install: cargo install cargo-llvm-cov)
cargo llvm-cov --html
```

**Coverage Strategy:**
- Unit tests cover error paths and edge cases
- Integration tests cover realistic workflows
- Hardware tests verify device communication (run on maitai only)
- Skip coverage for mock implementations (they're test code themselves)

## Test Types

**Unit Tests:**
- **Scope:** Single function or method
- **Location:** `#[cfg(test)] mod tests { }` at end of source file
- **Approach:**
  - Test both success and failure paths
  - Use simple inputs (primitives, small structs)
  - Verify side effects (parameter changes, state updates)
- **Example from `error_mapping_tests.rs` (lines 16-19):**
  ```rust
  #[test]
  fn config_error_maps_to_invalid_argument() {
      let err = DaqError::Config(config::ConfigError::Message("bad config".into()));
      assert_status_code(err, Code::InvalidArgument);
  }
  ```

**Integration Tests:**
- **Scope:** Multiple components working together (device + registry, service + database)
- **Location:** Separate files in `tests/` directory
- **Approach:**
  - Use mock devices from `daq-driver-mock`
  - Build realistic workflows (stage device → move → read → unstage)
  - Verify inter-component communication
- **Example patterns:**
  - `grpc_integration_test.rs` - Server + client communication
  - `plugin_system_integration.rs` - Plugin loading + execution
  - `data_pipeline_integration.rs` - Acquisition → storage → retrieval

**Hardware Tests:**
- **Scope:** Real devices on remote maitai machine
- **Location:** Separate files with `_validation` suffix
- **Approach:**
  - Gated by `#[cfg(feature = "hardware_tests")]`
  - Use `#[serial]` to prevent parallel device access
  - Timeout tests that might hang
  - Skip if device not available: `skip_without_hardware!("/dev/ttyS0")`
- **Environment:**
  - Run via: `PVCAM_SMOKE_TEST=1 cargo test --features pvcam_hardware`
  - Set via: `source scripts/env-check.sh` (loads PVCAM_SDK_DIR, etc.)
- **Example from docs:**
  ```rust
  #[tokio::test]
  #[serial]
  #[cfg(feature = "hardware_tests")]
  async fn test_maitai_connection() {
      skip_without_hardware!("/dev/ttyUSB5");
      // Real hardware test
  }
  ```

**End-to-End Tests:**
- **Scope:** Full system workflows (acquisition start → frames → stop)
- **Location:** `e2e_*.rs` or `full_pipeline_integration.rs`
- **Approach:**
  - Use mock devices to simulate hardware
  - Verify complete data flow
  - Check error recovery and cleanup
  - Example: `end_to_end_frames.rs` - Start streaming → receive frames → stop
  - Example: `full_pipeline_integration.rs` - Setup → scan → store → retrieve

## Common Patterns

**Async Testing with Channels:**
```rust
#[tokio::test]
async fn test_channel_backpressure() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(2);

    // Fill channel
    assert!(tx.try_send(msg1).is_ok());
    assert!(tx.try_send(msg2).is_ok());

    // Third send should fail with Full
    match tx.try_send(msg3) {
        Err(mpsc::error::TrySendError::Full(_)) => {},
        other => panic!("Expected Full, got {:?}", other),
    }

    // Drain and retry
    let _ = rx.recv().await;
    assert!(tx.try_send(msg3).is_ok());
}
```
(From: `scan_service.rs` lines 1037-1071)

**Error Testing:**
```rust
#[test]
fn test_error_conversion() {
    let input_error = DaqError::Instrument("camera failed".into());
    let grpc_status = map_daq_error_to_status(input_error);

    // Verify correct status code
    assert_eq!(grpc_status.code(), Code::Unavailable);

    // Verify error message preserved
    assert!(grpc_status.message().contains("camera"));
}
```

**Timing Assertions with Environment Awareness:**
```rust
#[test]
fn test_with_timing() {
    let start = Instant::now();

    // Do work
    std::thread::sleep(Duration::from_millis(10));

    let elapsed = start.elapsed();
    assert_duration_near(
        elapsed,
        Duration::from_millis(10),
        env_timing_tolerance(),  // Relaxes if CI detected
        "sleep timing"
    );
}
```
(From: `tests/common/mod.rs` lines 67-87)

**Conditional Test Skipping:**
```rust
#[tokio::test]
async fn test_hardware_optional() {
    skip_without_hardware!("/dev/ttyUSB0");

    // This only runs if device exists
    let result = real_hardware_operation().await;
    assert!(result.is_ok());
}
```

**Test Module Organization (by error domain):**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    mod configuration_errors { /* ... */ }
    mod hardware_errors { /* ... */ }
    mod runtime_errors { /* ... */ }
    mod feature_errors { /* ... */ }
}
```
(From: `error_mapping_tests.rs` - shows grouping by error category)

## Test Execution

**Local Development:**
```bash
# Run all tests (parallel)
cargo nextest run

# Watch mode (requires `cargo-watch`)
cargo watch -x nextest

# Run specific test
cargo nextest run device_registry

# With output
cargo nextest run -- --nocapture

# Doc tests (not in nextest by default)
cargo test --doc
```

**CI Pipeline:**
```bash
# Full validation before commit
cargo fmt --all
cargo clippy --all-targets
cargo nextest run
cargo test --doc
```

**Hardware Testing (maitai machine only):**
```bash
# Setup environment
source scripts/env-check.sh

# Run hardware tests
cargo nextest run --profile hardware --features hardware_tests -- --test-threads=1

# Or with specific test
cargo nextest run --features hardware_tests -- hardware_maitai_validation
```

## Debugging Tests

**Print Debugging:**
```bash
# Show println! output
cargo nextest run test_name -- --nocapture

# Enable tracing output
RUST_LOG=debug cargo nextest run test_name
```

**Logs in Tests:**
- Tracing output automatically captured if `tracing-subscriber` initialized
- Use `#[tracing::instrument]` in functions called by tests
- View via: `RUST_LOG=trace cargo test test_name -- --nocapture`

**Single-Threaded Execution:**
```bash
# For debugging race conditions
cargo test test_name -- --test-threads=1
```

---

*Testing analysis: 2026-01-21*
