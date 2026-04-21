# Capability Traits

<!--
last-ingested: 2026-04-19
sources:
  - crates/common-traits/src/capabilities.rs (authoritative)
  - crates/common-traits/src/lib.rs
  - docs/reference/driver-capability-matrix.md
  - docs/explanation/newcomer-guide.md §Core Capability Traits
see-also:
  - ./driver-registry.md
  - ./parameter.md
  - ../drivers/
-->

A device is **not** a struct type — it's whatever set of capabilities it
implements. **30** small focused traits, composed. This is the single most
important design choice in the codebase.

## The 30 traits (verified 2026-04-19)

| Trait | Shape | Example devices |
|-------|-------|-----------------|
| `Movable` | `move_abs`, `home`, position readback | ELL14, ESP300, Dover axis, MockStage |
| `Readable` | `read()` → scalar value | power meters, Comedi AI, MaiTai readouts |
| `ReadableWithMetadata` | `Readable` + timestamp + units | instruments reporting timestamped measurements |
| `SpectrumReadable` | return a 1D spectrum array | spectrometers, wavemeters |
| `Triggerable` | arm + trigger | cameras, pulsed lasers |
| `FrameProducer` | 2D image stream | PVCAM, Andor iStar, MockCamera |
| `FrameObserver` | tap into a frame stream without owning the producer | decimated preview, ring-buffer tap |
| `ExposureControl` | integration time | cameras, spectrometers |
| `WavelengthTunable` | set emission λ | MaiTai, Newport 1830-C, Thorlabs PM400, Andor Shamrock, MockLaser |
| `ShutterControl` | open / close beam shutter | MaiTai, Andor Shamrock, MockLaser |
| `EmissionControl` | laser on / off | MaiTai, IPG, MockLaser |
| `Stageable` | Bluesky lifecycle: `stage()` / `unstage()` | MockCamera (prepare / cleanup for acquisition) |
| `Settable` | generic param with JSON values | Comedi AO/DIO, MockDAQOutput |
| `Switchable` | binary on / off | power supplies |
| `Actionable` | single-shot action | device reset |
| `Loggable` | static metadata (serial, firmware) | any device |
| `Parameterized` | exposes `Parameter<T>` registry | most drivers |
| `Camera` | composite: `Triggerable + FrameProducer` | PVCAM, Andor iStar |
| `Commandable` | raw command-response with JSON args | MaiTai, IPG, Thorlabs PM400, PVCAM |
| `GatedCamera` | ICCD gating (DDG + MCP gain) | Andor iStar |
| `SpectrometerControl` | grating / wavelength / slit | Andor Shamrock |
| `TriggerOnPosition` | positional triggers during motion | Dover Motion (only) |
| `SafetyInterlock` | safety integration | laser interlock |
| `Reconfigurable` | runtime reconfig | modular instruments |
| `StateRefreshable` | re-read all parameters post-reconnect (bd-47p2) | **all** `driver-universal` devices |
| `CounterConfigurable` | configure pulse / edge counters | Comedi counter |
| `RangeIntrospectable` | valid-range metadata for GUI slider bounds | any parameter with bounds |
| `DeviceIntrospection` | report device metadata (serial, firmware, model) | most drivers |
| `CompositeCapability` | orchestrates multi-device operations (move+trigger+read) | `experiment` crate helpers |
| `CapabilityProvider` | typed device lookup surface, paired with `CompositeCapability` | registry façade |

Note: `docs/explanation/architecture.md` previously listed `PulseGenerator`; that trait does **not** exist in the code and has been removed from the doc. If pulse generation is added later, implement it as an explicit trait here.

Traits can compose: `Camera` is `Triggerable + FrameProducer`; `TriggerOnPosition` extends `Movable`; `GatedCamera` extends `FrameProducer`.

## Authoritative definitions

- File: **`crates/common-traits/src/capabilities.rs`** (verified 2026-04-19).
- `DriverFactory` + `DeviceLifecycle` live in `crates/common-traits/src/driver.rs`.
- Historically these lived in `crates/common/src/capabilities.rs`; that path no longer exists.
- Trait methods are `async` via `async_trait` (required because
  `Arc<dyn Trait>` forces boxed futures — see
  [`../invariants.md`](../invariants.md)).

## When to pick which trait

- Reads a value → `Readable`.
- Moves something, even abstractly (voltage, angle, mm) → `Movable`.
- Takes pictures → `Triggerable + FrameProducer + ExposureControl`.
- Raw vendor command passthrough → `Commandable`.
- Needs refresh after USB drops / power cycle → `StateRefreshable`.

**Do not invent a `MySpecificCamera` trait.** Compose existing capabilities.

## Coverage gaps (as of 2026-03-13 matrix)

- `EmissionControl` — no native SDK driver implements it.
- `Commandable` — only PVCAM among SDK drivers.
- `TriggerOnPosition` — only Dover Motion; no mock, no universal equivalent.
- `GatedCamera` / `SpectrometerControl` — test-only in `experiment` crate.
- `driver-dover-motion` — crate exists but **not registered** in `driver-registry`. Manual wiring or new feature gate needed.

Canonical per-factory matrix: `docs/reference/driver-capability-matrix.md`.
