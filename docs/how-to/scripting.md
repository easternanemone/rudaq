# Rhai Scripting Guide

This guide documents the current Rhai scripting surface for rust-daq.

## Overview

The `scripting` crate embeds Rhai and exposes selected helpers for experiment automation. Scripts use synchronous syntax while the underlying hardware/runtime paths remain async Rust.

## Build Profiles

Use one of the supported scripting feature sets:

```bash
# Baseline scripting + HDF5 helpers
cargo build --release -p scripting --features scripting_full

# Add Comedi bindings
cargo build --release -p scripting --features scripting_full_comedi

# Add LIBS-focused bindings
cargo build --release -p scripting --features scripting_full_libs
```

The `rhai-runner` binary requires `scripting_full` or one of the profiles built on top of it.

## Run a Script

```bash
./target/release/rhai-runner my_experiment.rhai

# or via cargo
cargo run --release -p scripting --features scripting_full --bin rhai-runner -- my_experiment.rhai
```

## Current Binding Surface

The current baseline helpers are:

| Function | Returns | Notes |
|----------|---------|-------|
| `create_mock_stage()` | `Stage` | Mock stage for local scripts and tests |
| `create_mock_power_meter(base_power)` | `PowerMeter` | Mock readable device |
| `create_hdf5(path)` | `Hdf5File` | Requires HDF5 scripting support; path is validated |
| `with_shutter_open(shutter, callback)` | callback result | Safety wrapper that closes the shutter on exit/error |
| `create_comedi(device_path)` | `ComediDAQ` | Requires Comedi scripting support; validates `/dev/comedi*` |

For LIBS/Andor/Dover-specific handles, build with `scripting_full_libs`.

## Important Change from Older Docs

Older docs referred to direct serial-device factory functions such as:

- `create_maitai`
- `create_maitai_tunable`
- `create_newport_1830c`
- `create_elliptec`
- `create_generic_driver`

Those are not recommended scripting entrypoints today.

For serial/TCP/SCPI devices defined by `driver-universal`, the supported workflow today is:

1. define the device manifest under `config/devices/`
2. load it through a hardware config and daemon startup
3. control it through the runtime or gRPC path

See `docs/how-to/device-config.md` for manifest authoring.

## Examples

### Mock Stage + Mock Power Meter

```rhai
let stage = create_mock_stage();
let meter = create_mock_power_meter(1.0e-6);

stage.set_soft_limits(0.0, 180.0);

for angle in [0.0, 45.0, 90.0, 135.0, 180.0] {
    stage.move_abs(angle);
    stage.wait_settled();
    let power = meter.read();
    print(`angle=${angle}, power=${power}`);
}
```

### Comedi Example

```rhai
let daq = create_comedi("/dev/comedi0");
print("Board: " + daq.board_name());
let voltage = daq.read_voltage(0);
print("AI0: " + voltage);
```

Build with:

```bash
cargo build --release -p scripting --features scripting_full_comedi
```

### HDF5 Output Example

```rhai
let hdf5 = create_hdf5("output.h5");
hdf5.write_attr("experiment", "demo");
hdf5.write_attr_f64("wavelength_nm", 800.0);
hdf5.close();
```

## Safety

### Use `with_shutter_open()` for beam-on sections

If a script needs a shuttered beam path, wrap the beam-on work in `with_shutter_open(...)` so closure is attempted even when the callback errors.

```rhai
let result = with_shutter_open(shutter, || {
    // beam-on work here
    do_measurement()
});
```

### Path Validation

- `create_hdf5(path)` validates the output path
- `create_comedi(device_path)` validates the device path

## Module Map

Important source files in `crates/scripting/src/`:

- `bindings.rs`
- `comedi_bindings.rs`
- `libs_bindings.rs`
- `plan_bindings.rs`
- `yield_bindings.rs`
- `rhai_engine.rs`
- `traits.rs`
- `script_runner.rs`
- `shutter_safety.rs`
- `path_security.rs`

## Related Docs

- `crates/scripting/README.md`
- `docs/how-to/device-config.md`
- `crates/experiment/README.md`
- `crates/hardware/README.md`
