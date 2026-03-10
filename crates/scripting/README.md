# scripting

Rhai-based scripting support for rust-daq.

## Overview

The `scripting` crate embeds the Rhai engine and bridges synchronous script syntax to async Rust hardware operations. It is used for one-shot scripts, plan helpers, safety wrappers, and selected hardware-specific bindings.

## Binaries

This crate provides:

- `rhai-runner` — feature-gated script runner binary

`rhai-runner` requires `--features scripting_full`.

## Feature Flags

Current crate features from `Cargo.toml`:

| Feature | Purpose |
|---------|---------|
| `python` | PyO3-backed Python interop |
| `hdf5_scripting` | HDF5 helpers from Rhai |
| `comedi_scripting` | Comedi/DAQ bindings |
| `libs_scripting` | LIBS-oriented bindings (Andor/Dover) |
| `hardware_factories` | Marker feature for selected factory/binding registration |
| `scripting_full` | baseline recommended profile (`hardware_factories + hdf5_scripting`) |
| `scripting_full_libs` | `scripting_full + libs_scripting` |
| `scripting_full_comedi` | `scripting_full + comedi_scripting` |
| `polarization` | compatibility alias for `scripting_full` |

## Binding Surface

The currently documented baseline helpers are:

- `create_mock_stage()`
- `create_mock_power_meter(base_power)`
- `create_hdf5(path)` when HDF5 scripting is enabled
- `create_comedi(device_path)` when Comedi scripting is enabled
- `with_shutter_open(shutter, callback)` for safety-scoped shutter control
- plan/yield bindings for orchestration support

Older docs referred to direct serial-device factories such as `create_maitai`, `create_newport_1830c`, `create_elliptec`, and `create_generic_driver`. Those are not recommended scripting entrypoints today.

## Quick Start

```bash
# Baseline scripting + HDF5
cargo run --release -p scripting --features scripting_full --bin rhai-runner -- my_script.rhai

# Add Comedi bindings
cargo run --release -p scripting --features scripting_full_comedi --bin rhai-runner -- my_script.rhai

# Add LIBS bindings
cargo run --release -p scripting --features scripting_full_libs --bin rhai-runner -- my_script.rhai
```

## Example

```rhai
let stage = create_mock_stage();
let meter = create_mock_power_meter(1.0e-6);

stage.set_soft_limits(0.0, 180.0);
stage.move_abs(45.0);
stage.wait_settled();

let power = meter.read();
print(`Power at ${stage.position()}: ${power}`);
```

## Module Layout

Important source modules include:

- `bindings.rs` — baseline Rhai bindings
- `comedi_bindings.rs` — Comedi-specific bindings
- `libs_bindings.rs` — LIBS/Andor/Dover bindings
- `yield_bindings.rs` — yield-based plan helpers
- `plan_bindings.rs` — plan orchestration bindings
- `rhai_engine.rs` — Rhai engine implementation
- `traits.rs` — scripting trait definitions
- `script_runner.rs` — script execution entrypoints
- `shutter_safety.rs` — shutter safety registry/guards
- `path_security.rs` — path validation and security helpers

## Safety Notes

- Scripts are sandboxed and do not get arbitrary filesystem/device access by default.
- HDF5 paths and Comedi device paths are validated before use.
- `with_shutter_open(...)` should be preferred for any script that needs beam-on sections.

## Related Docs

- `docs/how-to/scripting.md`
- `docs/how-to/device-config.md`
- `crates/experiment/README.md`
- `crates/hardware/README.md`
