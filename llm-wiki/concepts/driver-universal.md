# driver-universal (TOML manifests)

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-universal/
  - docs/explanation/plugin-schema.md
  - docs/reference/driver-capability-matrix.md
  - config/devices/*.toml
see-also:
  - ../drivers/universal.md
  - ../invariants.md
  - ./capability-traits.md
-->

The **forward path** for new serial / TCP / SCPI devices. Write TOML, not Rust.

## Scope

- Transports: serial (binary or ASCII), TCP, SCPI.
- Templating: MiniJinja for command bodies.
- Response parsing: tiered (regex / JSON / scalar / …).
- Expressions: `evalexpr` formulas for derived fields.
- Capabilities: declared per-manifest (Movable / Readable / WavelengthTunable / …).
- Post-reconnect refresh: every universal device implements `StateRefreshable` (bd-47p2).

## Schema v3 outline

```toml
schema_version = 3

[device]
name = "My Instrument"
capabilities = ["Readable", "WavelengthTunable"]

[connection]
type = "serial"                # or "tcp" / "scpi"
baud_rate = 9600
terminator = "\n"

[commands.read_value]
template = "READ?"
response_type = "float"

[capabilities.readable]
read = { command = "read_value" }

[capabilities.wavelength_tunable]
set_wavelength = { command = "WAV {{ value }}" }
get_wavelength = { command = "WAV?", response_type = "float" }
```

Full schema in `docs/explanation/plugin-schema.md`.

## Where manifests live

`config/devices/` — loaded by `load_all_factories()` at daemon startup.

Currently shipped (as of 2026-04):

| Manifest | Instrument | Machine |
|----------|------------|---------|
| `ell14.toml` | Thorlabs ELL14 rotator | maitai |
| `esp300.toml` | Newport ESP300 motion controller | maitai |
| `esp301_example.toml` | Example ESP301 | (template) |
| `maitai.toml` | Spectra-Physics MaiTai laser | maitai |
| `newport_1830c.toml` | Newport 1830-C power meter | maitai |
| `ipg_laser.toml` | IPG YLPP-200 laser | leabs-dev |
| `thorlabs_pm400.toml` | Thorlabs PM400 | leabs-dev |
| `red_pitaya_pid.toml` | Red Pitaya PID controller | (TCP/SCPI) |
| `generic_spectrometer.toml` | Example SCPI spectrometer | (template) |
| `siglent_sdg1025.toml` | Siglent signal generator | (template) |
| `thorlabs_sc10.toml` | Thorlabs SC10 shutter | (template) |
| `modbus_example.toml` | Modbus RTU example | (template) |
| `sample_temperature_controller.toml` | Example temp controller | (template) |
| `minimal_device_template.toml` | Skeleton for new devices | (template) |

## Rule of thumb

- Needs a vendor C SDK? → new SDK driver crate + paired `*-sys`.
- Otherwise → manifest under `config/devices/` + PR.

No new `driver-<serial-thing>` crates. See `invariants.md` §Drivers.

## Writing a new manifest

1. Copy `minimal_device_template.toml`.
2. Fill in `[device]`, `[connection]`, `[commands.*]`, `[capabilities.*]`.
3. Use MiniJinja for parameterized commands: `template = "WAV {{ value }}"`.
4. Declare capabilities explicitly; the factory derives the trait impls.
5. Run `cargo nextest run -p integration-tests --features universal --profile ci`.
6. Add an entry to `docs/reference/driver-capability-matrix.md`.
7. Update [`../drivers/universal.md`](../drivers/universal.md).
