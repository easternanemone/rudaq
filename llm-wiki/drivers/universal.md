# driver: universal (TOML manifests)

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-universal/
  - config/devices/
  - docs/explanation/plugin-schema.md
  - docs/reference/driver-capability-matrix.md
see-also:
  - ../concepts/driver-universal.md
  - ../crates/driver-universal.md
-->

**Crate:** `driver-universal` (always compiled).
**Config location:** `config/devices/*.toml`.
**Schema:** v3.
**Capability added across all universal devices (bd-47p2):** `StateRefreshable`.

## Shipped manifests

| File | Instrument | Capabilities | Transport | Host |
|------|------------|--------------|-----------|------|
| `ell14.toml` | Thorlabs ELL14 rotator | Movable, Parameterized, StateRefreshable | Serial (binary) | maitai |
| `esp300.toml` | Newport ESP300 motion controller | Movable, Parameterized, StateRefreshable | Serial (ASCII) | maitai |
| `maitai.toml` | Spectra-Physics MaiTai laser | Readable, WavelengthTunable, ShutterControl, EmissionControl, Parameterized, Commandable, StateRefreshable | Serial (ASCII) | maitai |
| `newport_1830c.toml` | Newport 1830-C power meter | Readable, WavelengthTunable, Parameterized, StateRefreshable | Serial (ASCII) | maitai |
| `ipg_laser.toml` | IPG YLPP-200-1-50-R | Readable, EmissionControl, Commandable, StateRefreshable | Serial (ASCII) | leabs-dev |
| `thorlabs_pm400.toml` | Thorlabs PM400 | Readable, WavelengthTunable, Commandable, StateRefreshable | Serial (SCPI) | leabs-dev |

## Templates / examples (not wired to a live device)

- `esp301_example.toml` — Newport ESP301 pattern.
- `red_pitaya_pid.toml` — Red Pitaya PID controller (TCP/SCPI).
- `generic_spectrometer.toml` — SCPI spectrometer template.
- `siglent_sdg1025.toml` — Siglent signal generator.
- `thorlabs_sc10.toml` — Thorlabs SC10 shutter.
- `modbus_example.toml` — Modbus RTU example.
- `sample_temperature_controller.toml` — temp controller pattern.
- `minimal_device_template.toml` — copy-and-fill skeleton.

## Engine bits available

- **MiniJinja** templates in command bodies.
- **Tiered response parsing**: regex / JSON / scalar / line-based.
- **evalexpr** formulas for derived / scaled values.
- Per-manifest capability declaration drives the exposed trait set.

## Adding a new device

See [`../concepts/driver-universal.md`](../concepts/driver-universal.md).
Copy `minimal_device_template.toml`, fill in, test, document in the
capability matrix.
