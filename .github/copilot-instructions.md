# GitHub Copilot Instructions for rust-daq

## Project Overview

**rust-daq** is a modular, high-performance, headless-first Data Acquisition (DAQ) system written in Rust for scientific instrumentation.

**Key Features:**
- Capability-based Hardware Abstraction Layer (HAL)
- Bluesky-inspired experiment orchestration (Plans + RunEngine)
- gRPC remote control with Rhai scripting
- Apache Arrow / HDF5 data storage
- 25 workspace crates (see CLAUDE.md for full architecture)

## Tech Stack

- **Language**: Rust 1.75+
- **Async Runtime**: Tokio
- **Serialization**: Serde, protobuf (tonic)
- **Testing**: Rust standard + hardware-in-the-loop
- **CI/CD**: GitHub Actions

## Coding Guidelines

### Critical Pattern: Reactive Parameters

**DO NOT** use raw `Arc<RwLock<T>>` or `Mutex<T>` for device state.

**USE** `Parameter<T>` with async hardware callbacks:
```rust
use common::parameter::Parameter;
use futures::future::BoxFuture;

let mut param = Parameter::new("wavelength_nm", 800.0)
    .with_range(690.0, 1040.0);

param.connect_to_hardware_write(move |val| -> BoxFuture<'static, Result<()>> {
    Box::pin(async move {
        // Write to hardware here
        Ok(())
    })
});
```

### Testing

- Run `cargo test` before committing
- Use `--features hardware_tests` only on remote machine with hardware
- Never use `std::thread::sleep` in async code - use `tokio::time::sleep`

### Code Style

- Run `cargo fmt --all` and `cargo clippy --all-features` before committing
- All methods are async - ensure tokio runtime context
- Use capability traits (`Movable`, `Readable`, etc.) for hardware abstraction

## Issue Tracking with bd (beads)

**CRITICAL**: This project uses **bd** for ALL task tracking. Do NOT create markdown TODO lists.

### Essential Commands

```bash
scripts/bd-safe.sh ready                           # Unblocked issues (canonical DB)
scripts/bd-safe.sh create "Title" -t task -p 2     # Create issue
scripts/bd-safe.sh update <id> --status in_progress
scripts/bd-safe.sh close <id> --reason "Done"
```

### Workflow

1. Check ready work: `scripts/bd-safe.sh ready`
2. Claim task: `scripts/bd-safe.sh update <id> --status in_progress`
3. Work on it
4. Complete: `scripts/bd-safe.sh close <id> --reason "Done"`
5. Commit `.beads/issues.jsonl` with code changes

### Worktree Safety

When using git worktrees, avoid local `.beads` runtime drift:

```bash
scripts/beads-worktree-hygiene.sh status
scripts/beads-worktree-hygiene.sh cleanup --apply
```

## Build Commands

```bash
cargo build                        # Default features
cargo build --all-features         # All features
cargo test -p common             # Test specific crate
cargo clippy --all-targets --all-features
```

## Feature Flags

**Hardware features** (gated in `driver-registry` crate):
- **Hardware**: `pvcam`, `pvcam_sdk`, `pvcam_hardware`, `comedi`, `comedi_hardware`, `andor`, `andor_hardware`, `all_hardware`
- **Profiles**: `maitai` (all real hardware), `full` (all mock drivers + storage)
- **System**: `networking` (gRPC), `hardware_tests`

**Note:** Serial/SCPI devices (Thorlabs, Newport, Spectra-Physics, Red Pitaya) are defined as TOML manifests in `config/devices/` and use `driver-universal` (no feature flags needed).

## Important Rules

- Use `Parameter<T>` for all hardware state (not raw Mutex/RwLock)
- Use bd for ALL task tracking
- Test with mock hardware first, then real hardware on remote
- Remote hardware machine: `maitai@100.117.5.12`

---

**For detailed documentation, see [CLAUDE.md](../CLAUDE.md)**
