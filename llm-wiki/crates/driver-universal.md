# crate: `driver-universal`

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-universal/Cargo.toml
  - crates/driver-universal/src/
  - docs/explanation/plugin-schema.md
  - config/devices/*.toml
see-also:
  - ../concepts/driver-universal.md
  - ../drivers/universal.md
-->

**Role:** Schema v3 TOML-driven driver. Instantiates drivers for
serial / TCP / SCPI instruments from declarative manifests in
`config/devices/`. **Always compiled.** **Forward path** for new
non-SDK devices.

**Key features:**

- Transports: serial (binary + ASCII), TCP, SCPI.
- MiniJinja templating for commands.
- Tiered response parsing.
- `evalexpr` formulas for derived fields.
- Per-manifest capability declaration.
- `StateRefreshable` implemented by all universal devices (bd-47p2).

**Manifests currently in `config/devices/`:** ell14, esp300, esp301_example,
generic_spectrometer, ipg_laser, maitai, minimal_device_template,
modbus_example, newport_1830c, red_pitaya_pid,
sample_temperature_controller, siglent_sdg1025, thorlabs_pm400,
thorlabs_sc10. See [`../drivers/universal.md`](../drivers/universal.md)
for per-device details.

**Adding a new device:** copy `minimal_device_template.toml`, fill in,
test under `integration-tests --features universal`, update
`docs/reference/driver-capability-matrix.md`. No Rust code. See
[`../concepts/driver-universal.md`](../concepts/driver-universal.md).
