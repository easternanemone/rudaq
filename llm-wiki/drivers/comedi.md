# driver: Comedi (Linux DAQ)

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-comedi/
  - docs/reference/driver-capability-matrix.md
see-also:
  - ../crates/driver-comedi.md
  - ../hardware/maitai.md
  - ../architecture.md §Safety
-->

**Vendor:** Linux Comedi project. **SDK:** `libcomedi` (C).
**Crate:** `driver-comedi` + paired `comedi-sys`.
**Feature flags:** `comedi` / `comedi_hardware`.
**Host:** maitai (Linux only). **Platform-restricted:** excluded from
default workspace / CI clippy via `--exclude`.

## Four factories

| Factory | `driver_type` | Capabilities |
|---------|---------------|--------------|
| `ComediAnalogInputFactory` | `comedi_analog_input` | `Readable`, `Parameterized` |
| `ComediAnalogOutputFactory` | `comedi_analog_output` | `Settable`, `Parameterized` |
| `ComediDigitalIOFactory` | `comedi_digital_io` | `Settable` |
| `ComediCounterFactory` | `comedi_counter` | `Readable`, `Settable` |

## Safety integration

The `comedi_digital_io` driver is the physical layer for Layer-1 of the
safety stack: the `safety_heartbeat_task` module in `bin`
(entry `spawn_heartbeat`) toggles a DIO channel per the `[safety_heartbeat]`
stanza of the hardware config; the external interlock cuts laser power
if the pulse stops. See [`../architecture.md`](../architecture.md)
§Safety. (Note: there is no `SafetyHeartbeat` Rust type — docs that
refer to one as a struct are stale naming.)

## Target hardware

NI PCI-MIO-16XE-10 and compatible boards supported by Linux Comedi kernel
modules. Other Comedi-supported cards should work without code changes —
but verify channel counts and voltage ranges per-board.

## Build caveat

- Building on non-Linux hosts fails because Comedi is kernel-module-backed. Always `--exclude comedi-sys --exclude driver-comedi` on macOS / Windows / non-Comedi Linux.
- CI clippy gate in [`../workflows/build-test-lint.md`](../workflows/build-test-lint.md) includes these exclusions.
