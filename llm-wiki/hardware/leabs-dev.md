# Hardware machine: `leabs-dev`

<!--
last-ingested: 2026-04-19
sources:
  - CLAUDE.md §Hardware Machines
  - docs/how-to/hardware-setup.md
  - docs/reference/driver-capability-matrix.md §Deployment Target Summary
see-also:
  - ../drivers/andor-sdk3.md
  - ../drivers/universal.md
  - ../workflows/hardware-testing.md
-->

**SSH:** `ssh leabs-dev`
**Daemon URL:** `http://100.109.21.118:50051`
**OS:** Ubuntu 22.04 x86_64.

## Device inventory (3)

| Category | Device |
|----------|--------|
| Camera / spectrograph (SDK) | Andor iStar ICCD + Shamrock spectrograph |
| Laser (universal) | IPG YLPP-200-1-50-R |
| Power meter (universal) | Thorlabs PM400 |

Total: **3** (consistent with CLAUDE.md header).

## Drivers available

- Native SDK: `driver-andor-sdk3` (`AndorCameraFactory`, `AndorSpectrographFactory`).
- TOML manifest: `driver-universal` loading `ipg_laser.toml`, `thorlabs_pm400.toml`.
- Mocks (always).

## Notable capabilities only on this host

- `GatedCamera` — Andor iStar (DDG + MCP gain). No other production driver provides this.
- Full `WavelengthTunable` + `ShutterControl` set on Andor Shamrock.

## Running

```
cargo run -p bin --release --features andor_hardware -- daemon --hardware-config config/leabs_hardware.toml
```

Canonical hardware config: `config/leabs_hardware.toml` (sibling of
`config/maitai_universal.toml`). See also
`docs/how-to/leabs-universal-db-signoff.md` for the universal-DB
sign-off notes specific to this host.

## Testing

Same pattern as maitai:

```
source scripts/ops/env-check.sh
cargo nextest run --profile hardware --features hardware_tests
```

## Safety stack

Comedi is **not** present on this host, so the Layer-1 safety-heartbeat
task (`spawn_heartbeat`) has no pulse target. Layers 2 (`HardwareWatchdog`)
and 3 (panic hook) still run. Any laser emission control here relies on
the IPG laser's own interlock wiring.
