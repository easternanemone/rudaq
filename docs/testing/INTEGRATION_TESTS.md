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

## Next Steps

1. **Add more end-to-end scenarios**: Multi-device workflows, error recovery
2. **Performance benchmarks**: Measure daemon startup time, connection latency
3. **Integration with CI**: Automated daemon provisioning for E2E tests
4. **Contract testing**: Verify gRPC API compatibility between daemon and GUI

## See Also

- [Testing Guide](../guides/testing.md) - Comprehensive testing documentation
- [AGENTS.md](../../AGENTS.md) - Build and test commands
- [CONTRIBUTING.md](../../CONTRIBUTING.md) - Development workflow
