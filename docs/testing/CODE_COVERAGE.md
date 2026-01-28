# Code Coverage Guide

This document explains how test coverage is measured, enforced, and reported in rust-daq.

## Overview

Code coverage is enforced in CI to maintain test quality:

- **Tool**: cargo-tarpaulin (Rust coverage tool)
- **Threshold**: 40% minimum line coverage
- **Enforcement**: PRs failing coverage threshold are blocked

## Running Coverage Locally

### Install cargo-tarpaulin

```bash
cargo install cargo-tarpaulin
```

### Run Coverage

```bash
# Basic coverage run
cargo tarpaulin --workspace --out Html --output-dir coverage

# Exclude crates that require special hardware/environment
cargo tarpaulin \
  --workspace \
  --exclude daq-egui \
  --exclude daq-driver-pvcam \
  --exclude daq-driver-comedi \
  --out Html \
  --output-dir coverage
```

### View Report

```bash
# Open HTML report (macOS)
open coverage/tarpaulin-report.html

# Open HTML report (Linux)
xdg-open coverage/tarpaulin-report.html
```

## CI Coverage Workflow

The `.github/workflows/coverage.yml` workflow:

1. **Runs on**: All PRs and pushes to main
2. **Excludes**: daq-egui (requires X11), hardware-specific crates
3. **Outputs**: 
   - Cobertura XML for tooling integration
   - HTML report as artifact
   - PR comment with coverage summary
4. **Threshold**: 40% minimum (configurable via `COVERAGE_THRESHOLD` env var)

### Coverage Artifacts

Each CI run produces:
- `coverage/cobertura.xml` - Machine-readable coverage data
- `coverage/tarpaulin-report.html` - Human-readable report
- PR comment with coverage summary

## Improving Coverage

### Focus Areas

Priority order for adding tests:

1. **Core abstractions** (`common`) - Error handling, capabilities, parameters
2. **Server logic** (`daq-server`) - gRPC handlers, request validation
3. **Hardware abstraction** (`daq-hardware`) - Device registry, configuration
4. **Drivers** (`daq-driver-*`) - Mock device behavior

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

Some code is intentionally excluded from coverage:

### Hardware-Specific Code

```rust
#[cfg(not(tarpaulin_include))]
fn hardware_specific_function() {
    // This requires actual hardware
}
```

### Unreachable Error Paths

```rust
#[cfg(not(tarpaulin_include))]
fn handle_impossible_error() {
    unreachable!("This should never happen")
}
```

## Coverage Thresholds

The current threshold is 40%, which is intentionally low because:

1. **Hardware drivers** - Many driver crates require actual hardware for meaningful tests
2. **GUI code** - daq-egui requires X11/Wayland runtime
3. **Integration paths** - Some code paths only execute in production environments

The threshold should be raised as more mock-based tests are added.

## Codecov Integration

If `CODECOV_TOKEN` is configured, coverage is also uploaded to Codecov for:

- Historical tracking
- PR decorations
- Coverage trend visualization

### Setting Up Codecov

1. Go to [codecov.io](https://codecov.io) and connect your repository
2. Get the repository upload token
3. Add `CODECOV_TOKEN` as a GitHub repository secret

## Troubleshooting

### Coverage Run Fails

```bash
# Try with verbose output
cargo tarpaulin --workspace -v

# Skip problematic tests
cargo tarpaulin --workspace --skip-clean --ignore-tests
```

### Coverage Lower Than Expected

1. **Check excluded crates**: Some crates may be excluded in CI
2. **Check test isolation**: Tests may not run due to `#[ignore]`
3. **Check feature flags**: Some code is behind feature gates

### Slow Coverage Runs

```bash
# Use multiple threads
cargo tarpaulin --workspace -j 4

# Skip clean build
cargo tarpaulin --workspace --skip-clean
```

## See Also

- [Testing Guide](../guides/testing.md) - General testing documentation
- [AGENTS.md](../../AGENTS.md) - Build and test commands
- [cargo-tarpaulin documentation](https://github.com/xd009642/tarpaulin)
