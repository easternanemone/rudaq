# Integration Tests Guide

This document describes the integration tests for rust-daq applications.

## Overview

Integration tests verify that applications work correctly as complete units, including:
- **bin**: Daemon startup, CLI commands, configuration loading, gRPC server
- **ui**: GUI client connections, state management, data transformations

These tests complement the extensive unit and integration tests in the `rust-daq` library crate.

## Running Integration Tests

```bash
# Run all integration tests for applications
cargo nextest run -p bin --test integration_tests
cargo nextest run -p ui --test integration_tests

# Or use standard cargo test
cargo test -p bin --test integration_tests
cargo test -p ui --test integration_tests

# Run with verbose output
cargo test -p bin --test integration_tests -- --nocapture

# Run ignored tests (require manual setup)
cargo test -p bin --test integration_tests -- --ignored --nocapture
```

## Test Categories

### bin Integration Tests

Located in `crates/bin/tests/integration_tests.rs`:

#### 1. CLI Tests
- **test_daemon_help_command**: Verifies `--help` output
- **test_daemon_binary_exists**: Checks daemon binary location

#### 2. Configuration Tests  
- **test_daemon_loads_demo_config**: Validates config file parsing

#### 3. End-to-End Tests (Ignored by Default)
- **test_daemon_startup_and_grpc_connection**: Full daemon lifecycle test
- **test_daemon_run_script_command**: Script execution test

**Why ignored?** These tests require:
- Building the daemon binary first
- Managing background processes
- Proper resource cleanup

To run manually:
```bash
# Build daemon first
cargo build -p bin

# Run ignored tests
cargo test -p bin --test integration_tests -- --ignored --nocapture
```

### ui Integration Tests

Located in `crates/ui/tests/integration_tests.rs`:

#### 1. gRPC Client Tests
- **test_daemon_url_parsing**: URL validation and normalization
- **test_grpc_connection_to_invalid_daemon**: Error handling for connection failures
- **test_grpc_client_creation**: Client configuration

#### 2. State Management Tests
- **test_shared_state_updates**: Concurrent state modification
- **test_concurrent_state_reads**: Thread-safe state access

#### 3. Data Transformation Tests
- **test_frame_downsampling_calculation**: Preview/fast quality calculations
- **test_power_unit_normalization**: Unit conversion (W → mW)

#### 4. Daemon Lifecycle Tests (Ignored)
- **test_gui_can_locate_daemon_binary**: Binary discovery
- **test_gui_connects_to_running_daemon**: Full E2E connection test

## Writing Integration Tests

### Guidelines

1. **Keep tests fast**: Mock external dependencies when possible
2. **Use `#[ignore]` for slow tests**: Require external setup
3. **Test one thing**: Each test should verify a single behavior
4. **Document requirements**: Explain what setup is needed for ignored tests
5. **Handle missing resources gracefully**: Tests should skip or provide helpful messages

### Example: Testing Daemon Startup

```rust
#[tokio::test]
#[ignore = "Requires daemon to be running"]
async fn test_full_daemon_lifecycle() {
    // 1. Start daemon in background
    let mut daemon = Command::new("rust-daq-daemon")
        .arg("daemon")
        .arg("--port").arg("50052")
        .spawn()
        .expect("Failed to start daemon");
    
    // 2. Wait for startup
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // 3. Connect and test
    let client = connect_to_daemon("http://127.0.0.1:50052").await?;
    let devices = client.list_devices().await?;
    assert!(!devices.is_empty());
    
    // 4. Cleanup
    daemon.kill().expect("Failed to stop daemon");
    daemon.wait().expect("Failed to wait for daemon");
}
```

### Example: Testing GUI Components

```rust
#[tokio::test]
async fn test_state_management() {
    let state = Arc::new(RwLock::new(AppState::default()));
    
    // Simulate UI update
    {
        let mut s = state.write().await;
        s.connected = true;
        s.device_count = 5;
    }
    
    // Verify state
    {
        let s = state.read().await;
        assert!(s.connected);
        assert_eq!(s.device_count, 5);
    }
}
```

## CI Integration

Integration tests run automatically in GitHub Actions CI:

```yaml
- name: Run integration tests
  run: |
    cargo nextest run -p bin --test integration_tests
    cargo nextest run -p ui --test integration_tests
```

Ignored tests are **not** run in CI by default. They require manual setup or dedicated test infrastructure.

## Troubleshooting

### Test Fails: "Binary not found"
- **Cause**: Daemon binary not built
- **Solution**: Run `cargo build -p bin` first

### Test Hangs: "Waiting for daemon"
- **Cause**: Daemon startup timeout or port conflict
- **Solution**: Check logs, try different port, verify no other daemon running

### Connection Error: "Failed to connect to daemon"
- **Cause**: Daemon not running or wrong address
- **Solution**: Start daemon manually: `cargo run -p bin -- daemon --port 50051`

### Universal Driver Validation Tests

Located in `crates/integration-tests/tests/hardware_universal_driver_validation.rs`:

These tests validate the `driver-universal` crate's declarative TOML-based drivers (schema v3) against both mock transports and real hardware, ensuring they behave identically to hand-coded legacy Rust drivers.

**Feature gate:** `#![cfg(feature = "universal")]`

#### Mock Tests (Run in CI — No Hardware Required)

| Test | Description |
|------|-------------|
| `test_load_newport_1830c_config` | Load Newport 1830-C TOML via `UniversalDriverFactory::from_file()` |
| `test_load_esp300_config` | Load ESP300 TOML config |
| `test_load_ell14_config` | Load ELL14 TOML config |
| `test_load_maitai_config` | Load MaiTai TOML config |
| `test_newport_readable_mock` | Readable::read() with mock transport (scientific notation parsing) |
| `test_newport_wavelength_get_mock` | WavelengthTunable::get_wavelength() with mock transport |
| `test_newport_wavelength_set_mock` | WavelengthTunable::set_wavelength() command generation |
| `test_newport_init_sequence_mock` | Init sequence sends E0 (disable echo) then U1 (set watts) |
| `test_esp300_movable_position_mock` | Movable::position() with mock transport |
| `test_esp300_movable_move_abs_mock` | Movable::move_abs() command generation |
| `test_esp300_movable_stop_mock` | Movable::stop() command generation |
| `test_ell14_movable_position_mock` | Movable::position() with hex response decoding |
| `test_ell14_movable_move_abs_mock` | Movable::move_abs() with degrees→pulses conversion |
| `test_newport_capabilities_wiring` | Verify Readable + WavelengthTunable capabilities present |
| `test_esp300_capabilities_wiring` | Verify Movable capability present |

```bash
# Run mock tests only (no hardware)
cargo nextest run -p integration-tests --features universal -- mock_tests
```

#### Hardware Tests (Run on Maitai — Requires Physical Instruments)

Gated by `#[cfg(feature = "hardware_tests")]` and `#[ignore]`. Must run sequentially since they share serial ports.

| Test | Hardware | Description |
|------|----------|-------------|
| `test_newport_read_power` | Newport 1830-C | Read power value, verify valid f64 |
| `test_newport_get_wavelength` | Newport 1830-C | Query wavelength, verify in 300–1100 nm range |
| `test_newport_set_wavelength` | Newport 1830-C | Set wavelength to 800nm, verify readback |
| `test_newport_rapid_reads` | Newport 1830-C | 5 sequential reads, all valid |
| `test_newport_wavelength_cycle` | Newport 1830-C | Cycle through 780→850→800nm, verify each |

```bash
# Run on maitai (sequential, with hardware)
cargo nextest run -p integration-tests --features "universal,hardware_tests" \
  --run-ignored all -- hardware_universal --test-threads=1
```

**Port configuration:** Set `NEWPORT_1830C_PORT` env var (default: `/dev/ttyS0`).

## Next Steps

1. **Add more end-to-end scenarios**: Multi-device workflows, error recovery
2. **Performance benchmarks**: Measure daemon startup time, connection latency
3. **Integration with CI**: Automated daemon provisioning for E2E tests
4. **Contract testing**: Verify gRPC API compatibility between daemon and GUI
5. **ESP300 hardware tests**: Identify correct serial port for ESP300 on maitai
6. **ELL14/MaiTai universal driver tests**: Add hardware validation once drivers are verified

## See Also

- [Testing Guide](../guides/testing.md) - Comprehensive testing documentation
- [Device Config Guide](../guides/device-config-guide.md) - Creating TOML device configs
- [AGENTS.md](../../AGENTS.md) - Build and test commands
- [CONTRIBUTING.md](../../CONTRIBUTING.md) - Development workflow
