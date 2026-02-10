# Contributing to rust-daq

Thank you for your interest in contributing to rust-daq! This document provides
guidelines and instructions for contributing to the project.

## Table of Contents

- [Getting Started](#getting-started)
- [Development Environment](#development-environment)
- [Building and Testing](#building-and-testing)
- [Code Style](#code-style)
- [Pull Request Process](#pull-request-process)
- [Issue Tracking](#issue-tracking)
- [Architecture Overview](#architecture-overview)

## Getting Started

### Prerequisites

- **Rust 1.75+**: Install via [rustup](https://rustup.rs/)
- **cargo-nextest**: Recommended test runner
  ```bash
  cargo install cargo-nextest --locked
  ```
- **Optional**: HDF5 libraries for storage features
- **Optional**: PVCAM SDK for camera support (Linux only)

### Clone and Build

```bash
git clone https://github.com/your-org/rust-daq.git
cd rust-daq

# Build with default features
cargo build

# Build with all features (requires SDK installations)
cargo build --all-features

# IMPORTANT: For real hardware on maitai, use the build script:
# bash scripts/build-maitai.sh
# See CLAUDE.md for details — plain `cargo build` produces mock-mode binaries.
```

## Development Environment

### Recommended Setup

1. **Editor**: VS Code with rust-analyzer, or RustRover
2. **Formatting**: Run `cargo fmt` before committing
3. **Linting**: Run `cargo clippy` to check for issues

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `RUST_LOG` | Control log verbosity (e.g., `debug`, `daq_server=trace`) |
| `CI` | Set automatically in CI; affects test timeouts |
| `PVCAM_SDK_DIR` | Path to PVCAM SDK (for camera features) |

## Building and Testing

### Running Tests

We use [cargo-nextest](https://nexte.st/) for faster parallel test execution:

```bash
# Run all tests (recommended)
cargo nextest run

# Run with standard cargo test (fallback)
cargo test

# Run a specific test
cargo nextest run test_name

# Run tests for a specific crate
cargo nextest run -p common

# Run with verbose output
cargo nextest run -- --nocapture
```

### Nextest Profiles

Different profiles are available for different scenarios:

| Profile | Use Case | Command |
|---------|----------|---------|
| `default` | Local development (2 retries, 2 min timeout) | `cargo nextest run` |
| `ci` | GitHub Actions (3 retries, 3 min timeout) | `cargo nextest run --profile ci` |
| `hardware` | Physical hardware tests (single-threaded) | `cargo nextest run --profile hardware` |
| `coverage` | Code coverage runs | `cargo nextest run --profile coverage` |

### Testing with Hardware

Hardware tests are gated behind the `hardware_tests` feature:

```bash
# On a machine with hardware connected
cargo nextest run --profile hardware --features hardware_tests
```

### Timing Tests

For tests that involve timing:

```rust
// Use start_paused for deterministic timing
#[tokio::test(start_paused = true)]
async fn test_with_timing() {
    // Use tokio::time::Instant, not std::time::Instant
    let start = tokio::time::Instant::now();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(start.elapsed(), Duration::from_secs(1));
}
```

## Code Style

### Formatting

All code must be formatted with `rustfmt`:

```bash
cargo fmt --all
```

### Linting

Address clippy warnings before submitting:

```bash
cargo clippy --all-targets
```

The workspace has pedantic lints enabled with specific allows configured in
`Cargo.toml`. See the `[workspace.lints.clippy]` section for details.

### Documentation

- All public APIs should have doc comments
- Use `//!` for module-level documentation
- Include examples in doc comments where helpful
- Run `cargo doc --no-deps` to check documentation builds

### Async Code Guidelines

- Always use `tokio::time::sleep`, never `std::thread::sleep` in async code
- Use `BoxFuture<'static, Result<()>>` for hardware callbacks
- Prefer `tokio::sync` primitives over `std::sync` in async contexts

### Error Handling

- Use `anyhow::Result` for application errors
- Use `thiserror` for library error types
- Add context with `.context("what failed")`
- Propagate errors with `?`, don't unwrap in library code

## Debt Prevention Guardrails

These rules prevent common anti-patterns that lead to accumulated technical debt.
Violations should be caught during code review and CI.

### Banned Patterns and Alternatives

| Anti-Pattern | Why Banned | Use Instead |
|-------------|-----------|-------------|
| `.unwrap()` in library code | Panics crash the daemon | `?`, `.context("reason")`, or `.unwrap_or_default()` |
| `.expect("msg")` in library code | Same as unwrap, obscured | `?` with `.context()` or `map_err()` |
| `panic!()` / `unreachable!()` | Crashes daemon | Return `Err(...)` or use `debug_assert!` |
| `std::thread::sleep()` in async | Blocks the tokio runtime | `tokio::time::sleep()` |
| `#[allow(clippy::*)]` without comment | Silences lints invisibly | Add `// Rationale: ...` comment or fix the warning |
| `#![allow(...)]` at crate level | Suppresses all instances | Use per-item `#[allow]` with rationale |
| `TODO` / `FIXME` without issue | Untracked work | `TODO(bd-XXXX): description` with beads issue |
| `unsafe {}` without safety comment | Undocumented invariants | Add `// SAFETY: ...` explaining the contract |
| Glob imports (`use foo::*`) in lib code | Namespace pollution | Import specific items |

### Exception Process

When a guardrail exception is genuinely needed:

1. Add `#[allow(...)]` at the narrowest scope possible (item, not module)
2. Include a comment: `// Rationale: <why this is acceptable>`
3. If the exception is temporary, create a beads issue and reference it

### Lint Configuration

Workspace-level lint policy is defined in `Cargo.toml` under `[workspace.lints.clippy]`.
Crate-level `#![allow(...)]` attributes must include a rationale comment explaining why
the suppression is necessary. See `bd-m5fh.4.2` for the audit that added these.

### Deprecated Code Policy

- All deprecated items must include a `note` with replacement guidance
- Items with zero external callers should be removed immediately
- Items with callers carry `Sunset: v1.0` and are tracked in `docs/architecture/ARCHITECTURE.md`
- New code must not call deprecated items

## Pull Request Process

### Branch Naming

Use descriptive branch names:

- `feat/add-zarr-support` - New features
- `fix/camera-timeout` - Bug fixes
- `docs/update-readme` - Documentation
- `refactor/simplify-registry` - Code improvements

### Commit Messages

Follow conventional commit format:

```
feat: add Zarr V3 storage writer

- Implement ZarrWriter with chunked array support
- Add storage_zarr feature flag
- Include compression options (blosc, gzip)

Closes bd-1234
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`

### Before Submitting

1. **Run tests**: `cargo nextest run`
2. **Format code**: `cargo fmt --all`
3. **Check lints**: `cargo clippy --all-targets`
4. **Update docs**: If you changed public APIs
5. **Update CHANGELOG**: For user-facing changes

### Review Process

1. Open a PR with a clear description
2. Link related issues (e.g., "Closes bd-1234")
3. Wait for CI to pass
4. Address review feedback
5. Squash or rebase as needed before merge

## Issue Tracking

### Using Beads (bd)

This project uses **beads** (`bd`) for issue tracking. All issues are stored in
`.beads/issues.jsonl` and version-controlled with the code.

```bash
# Check for ready work
bd ready

# Create a new issue
bd create "Issue title" -t task -p 1

# Update issue status
bd update bd-123 --status in_progress

# Close when done
bd close bd-123 --reason "Completed"
```

### Issue Types

| Type | Use For |
|------|---------|
| `bug` | Something broken |
| `feature` | New functionality |
| `task` | Work items (tests, docs, refactoring) |
| `epic` | Large features with subtasks |
| `chore` | Maintenance (dependencies, tooling) |

### Priorities

| Priority | Meaning |
|----------|---------|
| 0 | Critical (security, data loss, broken builds) |
| 1 | High (major features, important bugs) |
| 2 | Medium (default, nice-to-have) |
| 3 | Low (polish, optimization) |
| 4 | Backlog (future ideas) |

### Workflow

1. Check `bd ready` for available work
2. Claim with `bd update bd-123 --status in_progress`
3. Create linked issues for discovered work
4. Close with `bd close bd-123`
5. Commit `.beads/issues.jsonl` with your changes

## Architecture Overview

### Crate Structure

```
rust-daq/
├── crates/
│   ├── common/        # Core types, traits, errors
│   ├── pool/        # Zero-allocation frame pooling
│   ├── hardware/    # Device registry, drivers
│   ├── storage/     # Ring buffers, file writers
│   ├── experiment/  # Plans, RunEngine, documents
│   ├── scripting/       # Rhai scripting integration
│   ├── server/          # gRPC services
│   ├── bin/             # CLI and daemon
│   ├── ui/              # GUI application
│   └── driver-*/        # Hardware drivers
```

### Key Abstractions

1. **Capability Traits** (`common`): `Movable`, `Readable`, `FrameProducer`
2. **Parameter System** (`common`): Reactive parameters with hardware sync
3. **Device Registry** (`hardware`): Central device management
4. **Plans** (`experiment`): Declarative experiment sequences
5. **Document Model** (`experiment`): Bluesky-style structured metadata

### Adding a New Driver

See `docs/architecture/NEWCOMER_GUIDE.md` for detailed instructions.

Quick summary:
1. Create a new crate `driver-mydevice`
2. Implement capability traits (`Movable`, `Readable`, etc.)
3. Implement `DriverFactory` for registration
4. Add to `drivers` for automatic registration

## Getting Help

- **Documentation**: `docs/` directory
- **Examples**: `examples/`
- **Issues**: Use `bd` to check and create issues

## License

By contributing, you agree that your contributions will be licensed under the
same license as the project (see LICENSE file).
