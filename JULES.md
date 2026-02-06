# Jules Agent Instructions

This file provides guidance to Jules (Google's AI coding agent) when working on this repository.

## Pre-Commit Requirements (CRITICAL)

Before creating a PR or pushing any changes, you MUST run these commands in order:

```bash
# 1. Format all code (REQUIRED)
cargo fmt --all

# 2. Check for compilation errors
cargo check --workspace

# 3. Run clippy for linting
cargo clippy --workspace --all-targets -- -D warnings

# 4. Run tests
cargo test --workspace
```

**NEVER create a PR without running `cargo fmt --all` first.**

## Project Overview

rust-daq is a Rust-based data acquisition system with V5 headless-first architecture for scientific instrumentation.

### Crate Structure

- `crates/common/` - Domain types, parameters, error handling
- `crates/hardware/` - HAL, capability traits, DeviceRegistry
- `crates/drivers/` - Metacrate aggregating all drivers (feature flags)
- `crates/driver-*/` - Standalone driver crates (pvcam, andor-sdk3, thorlabs, newport, spectra-physics, dover-motion, comedi, generic, mock, red-pitaya)
- `crates/client/` - gRPC client library
- `crates/daq-modules/` - Experiment modules and plugin system
- `crates/protocol/` - Protobuf definitions
- `crates/server/` - gRPC server implementation
- `crates/storage/` - Data persistence (CSV, HDF5, Arrow)
- `crates/ui/` - GUI application using egui
- `crates/experiment/` - RunEngine and Plan definitions
- `crates/integration-tests/` - Cross-crate integration tests

### Key Patterns

1. **Import directly from crates**: Use `common::`, `hardware::`, `storage::`, etc. (the `rust-daq` facade crate has been removed)
2. **Error handling**: Use `DaqError` from `common`
3. **Async**: All hardware methods are async, use tokio runtime
4. **Parameters**: Use `Parameter<T>` for reactive hardware state

## Code Style

- Follow existing patterns in the codebase
- Use descriptive variable names
- Add doc comments for public APIs
- Keep functions focused and small
- Use `Result<T, DaqError>` for fallible operations

## Testing

- Unit tests go in the same file as the code (`#[cfg(test)]` module)
- Integration tests go in `tests/` directory
- Use `#[tokio::test]` for async tests
- Mock hardware interactions in tests

## Issue Tracking

This project uses **bd (beads)** for issue tracking. See AGENTS.md for details.

When completing a task:
1. Reference the issue ID in your PR description (e.g., "Fixes bd-8zcu")
2. Include a clear description of changes
3. List any new issues discovered during implementation

## Commit Messages

Use conventional commits:
- `feat:` - New feature
- `fix:` - Bug fix
- `refactor:` - Code refactoring
- `test:` - Adding tests
- `docs:` - Documentation
- `chore:` - Maintenance

Example: `feat(pvcam): add ROI bounds validation`

## Feature Flags

Check `Cargo.toml` for available features. Common ones:
- `storage_csv` (default) - CSV storage
- `storage_hdf5` - HDF5 storage (requires native libs)
- `instrument_serial` (default) - Serial port support
- `pvcam_hardware` - Real PVCAM hardware support

## Hardware Constraints

- Prime BSI camera: 2048x2048 pixels max
- ROI must not exceed sensor dimensions
- Binning must divide evenly into dimensions
- See `crates/common/src/limits.rs` for size limits

## PR Checklist

Before submitting a PR, ensure:
- [ ] `cargo fmt --all` has been run
- [ ] `cargo check --workspace` passes
- [ ] `cargo clippy --workspace --all-targets` passes
- [ ] `cargo test --workspace` passes
- [ ] PR description explains the changes
- [ ] Any new public APIs have documentation
