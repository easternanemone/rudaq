# Hardware machine: `maitai`

<!--
last-ingested: 2026-04-19
sources:
  - CLAUDE.md §Hardware Machines
  - docs/how-to/hardware-setup.md
  - docs/reference/driver-capability-matrix.md §Deployment Target Summary
  - config/maitai_universal.toml
see-also:
  - ../drivers/pvcam.md
  - ../drivers/comedi.md
  - ../drivers/universal.md
  - ../workflows/hardware-testing.md
-->

**SSH:** `maitai@maitai-eos`
**Daemon URL:** `http://100.117.5.12:50051`
**OS:** Ubuntu 22.04 x86_64.

## Device inventory (15)

| Category | Devices |
|----------|---------|
| Camera (SDK) | 1× PVCAM (Photometrics Prime 95B / BSI) |
| DAQ (SDK) | Comedi channels — Analog In, Analog Out, Digital IO, Counter (NI PCI-MIO-16XE-10) |
| Motion (universal) | 3× Thorlabs ELL14 rotators |
| Motion (universal) | 3× Newport ESP300 motion controllers |
| Laser (universal) | 1× Spectra-Physics MaiTai |
| Power meter (universal) | 1× Newport 1830-C |

Total: **15** (consistent with CLAUDE.md header).

## Drivers available

- Native SDK: `driver-pvcam`, `driver-comedi` (AI / AO / DIO / Counter).
- TOML manifest: `driver-universal` loading `ell14.toml`, `esp300.toml`, `maitai.toml`, `newport_1830c.toml`.
- Mocks (always).

## Building for this machine

```
bash scripts/ops/build-maitai.sh
```

**Always** use this wrapper. It sets:

- PVCAM SDK paths.
- Comedi feature gates (`comedi_hardware`).
- Correct target directory.

Never `cargo build` directly on maitai for a production binary.

## Running

```
./target/release/rust-daq-daemon daemon --hardware-config config/maitai_universal.toml
```

## Testing

```
source scripts/ops/env-check.sh
cargo nextest run --profile hardware --features hardware_tests
```

Nextest `hardware` profile: 6-minute per-test timeout.

## Safety stack active on this host

- Layer 1: Safety-heartbeat task (`crates/bin/src/safety_heartbeat_task.rs::spawn_heartbeat`, configured by `[safety_heartbeat]` in `config/maitai_universal.toml`) → Comedi DIO pulse → external interlock.
- Layer 2: `HardwareWatchdog` (`crates/common/src/health/watchdog.rs`, dedicated OS thread → 5-step shutdown on Tokio hang > 30 s).
- Layer 3: Panic hook → same 5-step shutdown sequence.

Feature: `comedi_hardware` is required for Layer 1 to actually pulse.

## Panel / port map

For the definitive serial/USB port assignments and physical layout, see
`docs/how-to/hardware-setup.md` — not duplicated here because hardware
rewiring lands in that doc first.
