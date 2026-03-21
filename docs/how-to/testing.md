# Testing Guide

This guide covers testing practices, tools, and patterns for the rust-daq project.

## Table of Contents

- [Quick Start](#quick-start)
- [Test Runner: cargo-nextest](#test-runner-cargo-nextest)
- [Test Categories](#test-categories)
- [Running Tests](#running-tests)
- [Writing Tests](#writing-tests)
- [Timing Tests](#timing-tests)
- [Hardware Tests](#hardware-tests)
- [CI/CD Integration](#cicd-integration)
- [Linting & Code Quality](#linting--code-quality)
- [Troubleshooting](#troubleshooting)
- [Best Practices](#best-practices)
- [Linting & Code Quality](#linting--code-quality)

---

## Quick Start

```bash
# Install nextest (recommended test runner)
cargo install cargo-nextest --locked

# Run all tests
cargo nextest run

# Run tests for a specific crate
cargo nextest run -p common

# Run a single test by name
cargo nextest run test_name

# Run with verbose output
cargo nextest run -- --nocapture

# Run doctests (not supported by nextest)
cargo test --doc
```

---

## Test Runner: cargo-nextest

We use [cargo-nextest](https://nexte.st/) as our primary test runner. It provides:

- **Faster execution** - Tests run in parallel processes
- **Better isolation** - Each test runs in its own process
- **Automatic retries** - Flaky tests are retried automatically
- **Rich output** - Better failure reporting and progress display

### Installation

```bash
cargo install cargo-nextest --locked
```

### Configuration

Nextest is configured via `.config/nextest.toml`. We have five profiles:

| Profile | Use Case | Retries | Timeout |
|---------|----------|---------|---------|
| `default` | Local development | 2 | 2 min |
| `ci` | GitHub Actions | 3 | 3 min |
| `hardware` | Physical hardware tests | 3 | 6 min |
| `libs-hardware` | LIBS integration tests | 3 | 6 min (inherits hardware) |
| `coverage` | Code coverage runs | 0 | 6 min |

```bash
# Use a specific profile
cargo nextest run --profile ci
cargo nextest run --profile hardware
```

### Test Groups

Hardware tests that share resources are serialized using test groups:

- `serial-hardware` - ESP300, MaiTai, Newport 1830-C tests (max-threads=1)
- `pvcam-hardware` - PVCAM camera tests (max-threads=1)
- `andor-hardware` - Andor SDK3 camera tests (max-threads=1)
- `elliptec-hardware` - ELL14/Elliptec rotator tests (max-threads=1)
- `daemon-e2e` - Daemon subprocess end-to-end tests (max-threads=1)

---

## Test Categories

### Unit Tests

Located inline in source files (`#[cfg(test)]` modules):

```bash
cargo nextest run -p common
cargo nextest run -p hardware
cargo nextest run -p storage
```

### Integration Tests

Located in `crates/integration-tests/`:

| Test File | Description |
|-----------|-------------|
| `mock_hardware.rs` | Mock device behavior verification |
| `e2e_acquisition.rs` | End-to-end acquisition workflows |
| `grpc_*.rs` | gRPC service integration tests |
| `data_pipeline_integration.rs` | Data flow and storage tests |

```bash
# Run all integration tests
cargo nextest run --test '*'

# Run specific integration test
cargo nextest run --test mock_hardware
```

### Hardware Validation Tests

Require physical hardware (run on maitai machine):

| Test File | Hardware Required |
|-----------|-------------------|
| `hardware_elliptec_validation.rs` | ELL14 rotators on RS-485 |
| `hardware_esp300_validation.rs` | Newport ESP300 controller |
| `hardware_maitai_validation.rs` | MaiTai laser |
| `hardware_pvcam_validation.rs` | Prime BSI camera |
| `hardware_universal_driver_validation.rs` | TOML-driven drivers (Newport 1830-C, ESP300) |
| `hardware_newport1830c_validation.rs` | Newport 1830-C power meter |

The universal driver validation tests also include **mock tests** (15 tests) that run in CI without hardware, validating TOML config loading, capability wiring, and response parsing for all four device configs.

```bash
# Run hardware tests (requires hardware)
cargo nextest run --profile hardware --features hardware_tests

# Run universal driver mock tests only (no hardware needed)
cargo nextest run -p integration-tests --features universal -- mock_tests

# Run universal driver hardware tests (on maitai, sequential)
cargo nextest run -p integration-tests --features "universal,hardware_tests" \
  --run-ignored all -- hardware_universal --test-threads=1
```

### Documentation Tests

Nextest does not support doctests. Run them separately:

```bash
cargo test --doc
```

---

## Running Tests

### Basic Commands

```bash
# Run all tests
cargo nextest run

# Run with filter expression
cargo nextest run -E 'test(grpc)'

# Run tests matching pattern
cargo nextest run test_mock

# Show output for passing tests too
cargo nextest run --success-output immediate

# Don't stop on first failure
cargo nextest run --no-fail-fast
```

### Filter Expressions

Nextest supports powerful filter expressions:

```bash
# Tests matching regex pattern
cargo nextest run -E 'test(/timing/)'

# Tests in specific package
cargo nextest run -E 'package(common)'

# Combine filters
cargo nextest run -E 'test(/grpc/) & not test(/streaming/)'

# All integration tests
cargo nextest run -E 'kind(test)'
```

### Feature-Gated Tests

Many tests require crate-specific features. Prefer package-qualified commands so it is clear which crate owns the feature:

```bash
# CI-parity default workspace run
cargo nextest run --profile ci

# Storage backends
cargo nextest run -p integration-tests --features storage_hdf5
cargo nextest run -p integration-tests --features storage_arrow

# Universal driver smoke tests
cargo nextest run -p integration-tests --features universal --profile ci

# PVCAM hardware-gated tests
cargo nextest run -p driver-pvcam --features "pvcam_sdk,hardware_tests"

# Andor hardware-gated tests
cargo nextest run -p driver-andor-sdk3 --features hardware_tests

# Real hardware integration profile on Maitai
source scripts/ops/env-check.sh && cargo nextest run --profile hardware --features hardware_tests
```

---

## Writing Tests

### Standard Async Test

```rust
#[tokio::test]
async fn test_basic_operation() {
    let device = MockDevice::new();
    device.connect().await.unwrap();
    assert!(device.is_connected());
}
```

### Test with Deterministic Timing

For tests that verify timing behavior, use `start_paused = true`:

```rust
#[tokio::test(start_paused = true)]
async fn test_operation_timing() {
    use tokio::time::Instant;

    let device = MockDevice::new();
    let start = Instant::now();

    device.wait_for_ready().await.unwrap();

    let elapsed = start.elapsed();
    // With start_paused, time is deterministic
    assert_eq!(elapsed.as_millis(), 100);
}
```

### Test with Retries (via Nextest)

Tests matching timing/throughput patterns automatically get retries:

```rust
// This test will be retried up to 3 times if it fails
#[tokio::test]
async fn test_throughput_measurement() {
    // ... timing-sensitive test
}
```

### Hardware Test (Ignored by Default)

```rust
#[tokio::test]
#[ignore] // Requires physical hardware
async fn test_real_device_communication() {
    skip_without_hardware!("/dev/ttyUSB0");
    // ... hardware test
}
```

---

## Timing Tests

### The Problem with Wall-Clock Timing

Tests that measure wall-clock time are inherently flaky:

```rust
// BAD: Flaky in CI due to system load
#[tokio::test]
async fn test_timing_flaky() {
    let start = std::time::Instant::now();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let elapsed = start.elapsed();

    // May fail under load: actual could be 150ms+
    assert!(elapsed.as_millis() >= 95 && elapsed.as_millis() <= 105);
}
```

### Solution: Tokio Time Mocking

Use `start_paused = true` for deterministic timing:

```rust
// GOOD: Deterministic with simulated time
#[tokio::test(start_paused = true)]
async fn test_timing_deterministic() {
    use tokio::time::Instant;  // Use tokio's Instant!

    let start = Instant::now();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let elapsed = start.elapsed();

    // Always exactly 100ms in simulated time
    assert_eq!(elapsed.as_millis(), 100);
}
```

### How `start_paused` Works

1. Time starts frozen at a virtual epoch
2. Time only advances during `tokio::time::sleep()` or `timeout()`
3. All sleeps complete "instantly" in wall-clock time
4. **Must use `tokio::time::Instant`**, not `std::time::Instant`

### When to Use Each Approach

| Scenario | Approach |
|----------|----------|
| Testing mock device timing | `start_paused = true` |
| Testing async state machines | `start_paused = true` |
| Benchmarking real performance | Wall-clock (with relaxed tolerance) |
| Testing hardware timeouts | Wall-clock (hardware profile) |

### Tolerance Helpers

For tests that must use wall-clock time, use generous tolerances:

```rust
use crate::common::{assert_duration_near, TimingTolerance, env_timing_tolerance};

#[tokio::test]
async fn test_with_tolerance() {
    let start = std::time::Instant::now();
    // ... operation ...
    let elapsed = start.elapsed();

    assert_duration_near(
        elapsed,
        Duration::from_millis(100),
        env_timing_tolerance(),  // Relaxed in CI, Normal locally
        "operation timing"
    );
}
```

---

## Hardware Tests

### Remote Hardware Setup

Hardware tests run on the maitai machine (100.117.5.12):

```bash
# SSH test command
ssh maitai@100.117.5.12 'cd ~/rust-daq && \
  cargo nextest run --profile hardware --features hardware_tests'
```

### Serial Port Inventory

| Device | Port | Driver |
|--------|------|--------|
| ELL14 Rotators | `/dev/serial/by-id/usb-FTDI_FT230X_Basic_UART_DK0AHAJZ-if00-port0` | `driver-universal` (ell14.toml) |
| ESP300 Controller | `/dev/ttyUSB0` | `driver-universal` (esp300.toml) |
| MaiTai Laser | `/dev/serial/by-id/usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_Controller_0001-if00-port0` | `driver-universal` (maitai.toml) |
| Newport 1830-C | `/dev/ttyS0` | `driver-universal` (newport_1830c.toml) |
| NI PCI-MIO-16XE-10 | `/dev/comedi0` | `driver-comedi` (comedi feature) |

### PVCAM Tests

Require SDK and environment setup:

```bash
source scripts/ops/env-check.sh
export PVCAM_SMOKE_TEST=1

cargo nextest run --profile hardware \
  --features "pvcam_hardware" \
  --test pvcam_hardware_smoke
```

### Test Serialization

Hardware tests sharing resources are automatically serialized:

```toml
# .config/nextest.toml
[test-groups.elliptec-hardware]
max-threads = 1

[[profile.default.overrides]]
filter = "test(/elliptec/) | test(/ell14/)"
test-group = "elliptec-hardware"
```

---

## CI/CD Integration

### GitHub Actions Workflow

Tests run on every push/PR via `.github/workflows/ci.yml`:

```yaml
- name: Install cargo-nextest
  uses: taiki-e/install-action@v2
  with:
    tool: cargo-nextest

- name: Run tests
  run: cargo nextest run --workspace --profile ci

- name: Run doctests
  run: cargo test --workspace --doc
```

### CI Profiles

The `ci` profile provides:
- 3 retries for flaky tests
- 3-minute timeout per test
- No fail-fast (runs all tests)
- Detailed failure output

### Hardware Tests in CI

Hardware tests only run on main branch pushes (not PRs):

```yaml
hardware-tests:
  if: github.ref == 'refs/heads/main' && github.event_name == 'push'
  # ... SSH to maitai and run tests
```

---

## Troubleshooting

### Test Hangs

If a test hangs, nextest will terminate it after the slow-timeout period:

```bash
# See which tests are slow
cargo nextest run --status-level slow
```

### Flaky Tests

If a test passes on retry, it's marked as flaky:

```bash
# See retry information
cargo nextest run --failure-output immediate-final
```

### Missing Features

Compilation errors often mean missing features:

```bash
# Check what features are available
cargo metadata --format-version 1 | jq '.packages[] | select(.name == "rust-daq") | .features'
```

### Hardware Not Found

```rust
// Skip test if hardware unavailable
#[tokio::test]
async fn test_with_hardware() {
    if !std::path::Path::new("/dev/ttyUSB0").exists() {
        eprintln!("Skipping: hardware not available");
        return;
    }
    // ... test code
}
```

### Timing Test Failures

If timing tests fail in CI:
1. Check if using `start_paused = true` for mock timing
2. If wall-clock is required, use `TimingTolerance::Relaxed`
3. Consider if the test should be in the `hardware` profile

---

## Best Practices

1. **Use `start_paused = true`** for any test involving `tokio::time::sleep`
2. **Use `tokio::time::Instant`** (not `std::time::Instant`) with paused time
3. **Add `#[ignore]`** to tests requiring physical hardware
4. **Use filter expressions** to run focused test sets
5. **Run `cargo test --doc`** separately for doctests
6. **Use the `ci` profile** in GitHub Actions
7. **Use the `hardware` profile** for physical device tests

---

## Linting & Code Quality

### Clippy Unwrap Enforcement

The project enforces `clippy::unwrap_used` in all library code via CI. This prevents silent panics in production code.

**What's enforced:**

| Code Type | Allows `.unwrap()` | Allows `expect()` | CI Check |
|-----------|-------------------|-----------------|----------|
| Library code (src/*.rs) | NO | YES | `-D clippy::unwrap_used` |
| Test code (tests/, #[cfg(test)]) | YES | YES | Exempt |
| Binary code (bin/*.rs) | YES | YES | Exempt |

**Why this matters:**

- `.unwrap()` panics silently if a Result is Err - hard to debug in production
- `expect()` requires a message explaining why the Result should never fail
- Tests can use `.unwrap()` for simplicity; the test harness catches failures
- Binaries can use `.unwrap()` for CLI errors; the process exits anyway

### Running the Check Locally

To match the CI check before pushing:

```bash
# Full library check (matches CI)
cargo clippy --lib --features full -- -D clippy::unwrap_used

# Specific crate
cargo clippy -p common --lib -- -D clippy::unwrap_used

# All clippy checks with full warnings
cargo clippy --workspace --all-targets --features full --exclude ui -- -D warnings
```

### Fixing Unwrap Violations

**Pattern 1: Result that should never fail**

Use `expect()` with a clear explanation:

```rust
// WRONG: Silent panic
let file = std::fs::read_to_string("config.toml").unwrap();

// RIGHT: Clear justification
let file = std::fs::read_to_string("config.toml")
    .expect("config.toml must exist at startup");
```

**Pattern 2: Recoverable error**

Handle the Result:

```rust
// WRONG: Panics on bad input
pub fn parse_config(input: &str) -> Config {
    let data = serde_json::from_str(input).unwrap();
    Config::from_json(data)
}

// RIGHT: Propagate the error
pub fn parse_config(input: &str) -> Result<Config> {
    let data = serde_json::from_str(input)?;
    Ok(Config::from_json(data))
}
```

**Pattern 3: Already validated**

Use early returns:

```rust
// WRONG: Panics if validation missed
if let Some(value) = maybe_value {
    return Some(value.unwrap());
}

// RIGHT: Safe extraction
if let Some(value) = maybe_value {
    return Some(value);
}
```

**Pattern 4: Truly impossible path**

Use `expect()` in tests or with detailed comments:

```rust
// In production code
fn process_device(path: &Path) -> Result<()> {
    let port = std::fs::read_to_string(path)
        .context("Failed to read device config")?;
    // ...
}

// In test code (unwrap is fine)
#[test]
fn test_device_processing() {
    let result = process_device("test/config.json").unwrap();
    assert!(result.is_ok());
}
```

### Common Exceptions

Some legitimate uses of `expect()`:

| Scenario | Example | Justification |
|----------|---------|---------------|
| Hardcoded config | `serde_json::from_str(r#"{"x":1}"#).expect("valid JSON")` | Compile-time constant |
| Initialization | `PORT.parse().expect("PORT env var must be valid u16")` | Startup-time validation |
| Test setup | `device.connect().expect("test device must connect")` | Test infrastructure |
| Invariant guarantee | `state.get(KEY).expect("KEY always set in invariant")` | Code contract verified |

---

## Code Coverage

Code coverage is measured using [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov):

```bash
# Install
cargo install cargo-llvm-cov

# HTML report
cargo llvm-cov --workspace --html --output-dir coverage/html

# LCOV format (for CI)
cargo llvm-cov --workspace --lcov --output-path coverage/lcov.info

# Exclude hardware-specific crates
cargo llvm-cov --workspace --exclude ui --exclude driver-pvcam --exclude driver-comedi --html --output-dir coverage/html
```

Coverage is informational — no automatic threshold enforcement. Focus testing effort on core abstractions (`common`), server logic, and hardware abstraction before driver code.

---

## Pre-Commit Hooks

```bash
# Install hooks (one-time)
bash scripts/ops/install-hooks.sh

# Quick hooks for rapid iteration (format only)
bash scripts/ops/install-hooks.sh quick

# Run manually without committing
pre-commit run --all-files

# Emergency bypass (use sparingly)
git commit --no-verify -m "Emergency fix"
```

Full hooks check: formatting, clippy, unit tests, file issues, secret detection, TOML validation.
Quick hooks check: formatting only.

---

## See Also

- [cargo-nextest documentation](https://nexte.st/)
- [Tokio testing guide](https://tokio.rs/tokio/topics/testing)
- [Device Config Guide](device-config.md) — Creating TOML device configs (schema v3)
