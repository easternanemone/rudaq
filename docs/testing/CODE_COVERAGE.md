# Code Coverage Guide

This document explains how test coverage is measured, enforced, and reported in rust-daq.

## Overview

Code coverage is measured in CI to maintain test quality:

- **Tool**: cargo-llvm-cov (LLVM-based coverage tool)
- **Reference**: Coverage results available in CI artifacts
- **Enforcement**: No automatic threshold enforcement

## Running Coverage Locally

### Install cargo-llvm-cov

```bash
cargo install cargo-llvm-cov
```

### Run Coverage

```bash
# Basic coverage run (HTML report)
cargo llvm-cov --workspace --html --output-dir coverage/html

# Generate LCOV format (for CI integration)
cargo llvm-cov --workspace --lcov --output-path coverage/lcov.info

# Exclude crates that require special hardware/environment
cargo llvm-cov \
  --workspace \
  --exclude ui \
  --exclude driver-pvcam \
  --exclude driver-comedi \
  --html \
  --output-dir coverage/html
```

### View Report

```bash
# Open HTML report (macOS)
open coverage/html/index.html

# Open HTML report (Linux)
xdg-open coverage/html/index.html
```

## CI Coverage Workflow

The `coverage` job in `.github/workflows/ci.yml`:

1. **Runs on**: Main branch pushes and PRs with `ci:full` label
2. **Excludes**: ui (requires X11), hardware-specific crates
3. **Outputs**:
   - LCOV format for tooling integration
   - HTML report as artifact
4. **Threshold**: No automatic enforcement (informational only)

### Coverage Artifacts

Each CI run produces:
- `coverage/lcov.info` - LCOV format for tooling integration
- `coverage/html/` - Human-readable HTML report directory

## Improving Coverage

### Focus Areas

Priority order for adding tests:

1. **Core abstractions** (`common`) - Error handling, capabilities, parameters
2. **Server logic** (`server`) - gRPC handlers, request validation
3. **Hardware abstraction** (`hardware`) - Device registry, configuration
4. **Drivers** (`driver-*`) - Mock device behavior

### Writing Effective Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Test normal operation
    #[test]
    fn test_feature_success() {
        let result = my_function(valid_input);
        assert!(result.is_ok());
    }

    // Test error conditions
    #[test]
    fn test_feature_error() {
        let result = my_function(invalid_input);
        assert!(result.is_err());
    }

    // Test edge cases
    #[test]
    fn test_feature_edge_case() {
        let result = my_function(boundary_value);
        assert_eq!(result.unwrap(), expected);
    }
}
```

### Async Test Coverage

```rust
#[tokio::test]
async fn test_async_operation() {
    let result = async_function().await;
    assert!(result.is_ok());
}

// For timing-sensitive tests
#[tokio::test(start_paused = true)]
async fn test_with_paused_time() {
    // Time is paused, use tokio::time::advance() to simulate time passing
    tokio::time::advance(Duration::from_secs(1)).await;
}
```

## Excluded Code

Some code is intentionally excluded from coverage using the `coverage` attribute:

### Hardware-Specific Code

```rust
#[cfg(not(coverage))]
fn hardware_specific_function() {
    // This requires actual hardware
}
```

### Unreachable Error Paths

```rust
#[cfg(not(coverage))]
fn handle_impossible_error() {
    unreachable!("This should never happen")
}
```

## Coverage Notes

Coverage measurement is informational and helps identify which areas need additional tests:

1. **Hardware drivers** - Many driver crates require actual hardware for meaningful tests
2. **GUI code** - ui requires X11/Wayland runtime
3. **Integration paths** - Some code paths only execute in production environments

Coverage results should inform testing priorities rather than be treated as strict metrics.

## Troubleshooting

### Coverage Run Fails

```bash
# Try with verbose output
cargo llvm-cov --workspace -v

# Build without coverage first to check for compilation issues
cargo build --workspace --exclude ui
```

### Coverage Lower Than Expected

1. **Check excluded crates**: Some crates may be excluded in CI (ui, hardware-specific)
2. **Check test isolation**: Tests may not run due to `#[ignore]`
3. **Check feature flags**: Some code is behind feature gates
4. **Check coverage attribute**: Code marked with `#[cfg(not(coverage))]` is excluded

### Slow Coverage Runs

```bash
# Run a subset of crates (faster iteration)
cargo llvm-cov --package common --html

# Skip specific crates during development
cargo llvm-cov --workspace --exclude driver-pvcam --exclude driver-comedi
```

## See Also

- [Testing Guide](../guides/testing.md) - General testing documentation
- [AGENTS.md](../../AGENTS.md) - Build and test commands
- [cargo-llvm-cov documentation](https://github.com/taiki-e/cargo-llvm-cov)
