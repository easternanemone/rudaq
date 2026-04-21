# crate: `driver-comedi`

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-comedi/
  - docs/reference/driver-capability-matrix.md
see-also:
  - ../drivers/comedi.md
  - ./comedi-sys.md
-->

**Role:** Linux Comedi DAQ boards (e.g. NI PCI-MIO-16XE-10). Analog / digital I/O.

**Feature gates:** `comedi` / `comedi_hardware`.

**Factories:**

| Factory | `driver_type` | Capabilities |
|---------|---------------|--------------|
| `ComediAnalogInputFactory` | `comedi_analog_input` | Readable, Parameterized |
| `ComediAnalogOutputFactory` | `comedi_analog_output` | Settable, Parameterized |
| `ComediDigitalIOFactory` | `comedi_digital_io` | Settable |
| `ComediCounterFactory` | `comedi_counter` | Readable, Settable |

**Deployment target:** **maitai** (Linux + Comedi kernel drivers).

**Platform-restricted:** this crate and `comedi-sys` are excluded from
default workspace builds + CI clippy via `--exclude`. Only compiled on
Linux hosts with Comedi available.

**Paired sys crate:** `comedi-sys`.

**Safety integration:** Comedi DIO channel is the physical-layer pulse
target for the safety-heartbeat task (Layer 1 of the 3-layer safety stack).
See [`../architecture.md`](../architecture.md) §Safety.
