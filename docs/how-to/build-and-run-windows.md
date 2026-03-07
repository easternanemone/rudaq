# Windows Build Requirements for LIBS Drivers

This document describes the Windows-specific build requirements for LIBS hardware drivers.

## Overview

The LIBS hardware integration requires Windows for hardware compatibility. All LIBS drivers must build and test on Windows using the MSVC toolchain.

## Build Environment

### Required Components

1. **Rust Toolchain**
   - Target: `x86_64-pc-windows-msvc`
   - Minimum version: Stable (1.75+)

2. **Visual Studio Build Tools**
   - Required for MSVC linker
   - Can be installed via Visual Studio Installer
   - Only Build Tools are needed, not full VS IDE

3. **LLVM/Clang**
   - Required for bindgen FFI generation
   - Version: 17+ recommended
   - Used to parse C header files for Rust bindings

### Installation

```bash
# Install Rust target
rustup target add x86_64-pc-windows-msvc

# Set LIBCLANG_PATH for bindgen
# This is set automatically in CI via KyleMayes/install-llvm-action
$env:LIBCLANG_PATH = "C:\path\to\llvm\bin"
```

## CI/CD Pipeline

Windows checks are now opt-in and live in `.github/workflows/ops.yml` (`windows-driver-check` job).

### Job

1. **windows-driver-check** - Manually triggered Windows-targeted `cargo check` for optional LIBS crates

### Triggers

The job runs only via manual dispatch (`workflow_dispatch`) to avoid always-on GitHub Actions cost.

### Caching

The manual job uses `Swatinem/rust-cache@v2` to cache:
- Cargo registry
- Target directory
- Build artifacts

Cache key is scoped to `ops-windows-driver-check`.

## Local Development

### Building on Windows

```bash
# Build all LIBS drivers (mock mode)
cargo build --package driver-dover-motion --target x86_64-pc-windows-msvc
cargo build --package driver-andor-sdk3 --target x86_64-pc-windows-msvc
# Note: SPIRIT laser and other serial/TCP devices use driver-universal (always compiled)
```

### Running Tests

```bash
# Run tests (mock mode - no hardware required)
cargo nextest run --package driver-dover-motion --target x86_64-pc-windows-msvc
cargo nextest run --package driver-andor-sdk3 --target x86_64-pc-windows-msvc
```

### Linting

```bash
# Run clippy
cargo clippy --package driver-dover-motion --target x86_64-pc-windows-msvc --all-targets -- -D warnings
# ... repeat for other driver packages
```

## Hardware Mode vs Mock Mode

**Mock Mode (Default)**
- No hardware SDKs required
- Builds with default features
- Suitable for CI and development without hardware
- FFI bindings still compile and are type-checked

**Hardware Mode**
- Requires vendor SDKs installed
- Enabled via feature flags (e.g., `--features hardware`)
- Only required on machines with actual LIBS hardware
- Not used in CI

## Driver Crates

The LIBS integration includes these driver crates:

| Crate | Purpose | SDK Required |
|-------|---------|--------------|
| `dover-motion-sys` | FFI bindings for Dover Motion API | Yes (hardware mode) |
| `driver-dover-motion` | High-level driver wrapper | No (mock available) |
| `andor-sdk3-sys` | FFI bindings for Andor SDK3 | Yes (hardware mode) |
| `driver-andor-sdk3` | High-level driver wrapper | No (mock available) |

**Note:** Serial/TCP devices (SPIRIT laser, etc.) use `driver-universal` TOML manifests instead of dedicated crate packages.

## Troubleshooting

### Bindgen Errors

If you see bindgen errors about missing `clang.dll` or `libclang`:
1. Ensure LLVM is installed
2. Set `LIBCLANG_PATH` environment variable
3. Verify path points to directory containing `libclang.dll`

### MSVC Linker Errors

If you see linker errors:
1. Ensure Visual Studio Build Tools are installed
2. Verify `x86_64-pc-windows-msvc` target is installed
3. Check that `link.exe` is in PATH

### Caching Issues

If builds are slow or caches seem stale:
1. Clear local target directory: `cargo clean`
2. In CI: Workflow will automatically cache on `main` branch pushes

## Related Documentation

- [LIBS Integration Epic](/.beads/issues/bd-3yb8.jsonl) - Overall integration plan
- [Driver Development Guide](./hardware-drivers.md) - How to create drivers
- [FFI Conventions]() - FFI binding patterns
