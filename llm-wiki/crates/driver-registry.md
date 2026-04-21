# crate: `driver-registry`

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-registry/Cargo.toml
  - crates/driver-registry/src/
  - docs/reference/driver-capability-matrix.md
see-also:
  - ../concepts/driver-registry.md
  - ../concepts/mock-registry.md
-->

**Role:** Concrete factory registration + hardware feature gating for the
whole workspace.

**Always-on registrations:** `driver-mock`, `driver-universal`.

**Feature-gated:**

| Feature | Enables |
|---------|---------|
| `pvcam` / `pvcam_sdk` / `pvcam_hardware` | `PvcamFactory` |
| `andor` / `andor_hardware` | `AndorCameraFactory`, `AndorSpectrographFactory` |
| `comedi` / `comedi_hardware` | Comedi AI / AO / DIO / Counter factories |
| `all_hardware` | pvcam + comedi + andor (mock SDK paths) |
| `full` | alias for `all_hardware` |

**Key exports:**

- `create_canonical_mock_registry(workspace_root)` — deterministic 3-device registry for tests (`stage`, `power_meter`, `camera`). See [`../concepts/mock-registry.md`](../concepts/mock-registry.md).
- `register_all_factories(...)` — wiring called by `bin`.

**Not wired as of 2026-03-13 capability matrix:** `driver-dover-motion`.
Requires manual wiring or a dedicated feature gate.
