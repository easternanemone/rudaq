# DriverRegistry & DriverFactory

<!--
last-ingested: 2026-04-19
sources:
  - crates/driver-registry/
  - crates/hardware/src/registry/
  - crates/common-traits/src/driver.rs  (DriverFactory definition)
  - docs/reference/driver-capability-matrix.md
see-also:
  - ./driver-universal.md
  - ./capability-traits.md
  - ./mock-registry.md
-->

Two distinct registries:

- **`DriverFactory` registry** (static list, compile-time): which factories are wired in. Lives in `driver-registry`.
- **`DeviceRegistry`** (runtime map): instantiated devices keyed by `DeviceId`. Lives in `hardware`.

## `DriverFactory` trait

Each driver crate implements `DriverFactory` (in `common-traits`). The
trait reports:

- `driver_type()` — string tag matched against TOML `type` fields.
- `capabilities()` — which capability traits the produced driver implements.
- `create(config)` — build a driver from parsed config.

`driver-registry` registers all factories at compile time.

## What's registered

Always on, no feature flag:

- `MockCameraFactory`, `MockStageFactory`, `MockRotatorFactory`, `MockLaserFactory`, `MockPowerMeterFactory`, `MockDAQOutputFactory` (from `driver-mock`).
- All universal factories (from `driver-universal`) — one per TOML manifest loaded at startup.

Feature-gated:

| Feature | Factories |
|---------|-----------|
| `pvcam` / `pvcam_sdk` / `pvcam_hardware` | `PvcamFactory` |
| `andor` / `andor_hardware` | `AndorCameraFactory`, `AndorSpectrographFactory` |
| `comedi` / `comedi_hardware` | `ComediAnalogInputFactory`, `ComediAnalogOutputFactory`, `ComediDigitalIOFactory`, `ComediCounterFactory` |
| `all_hardware` | pvcam + andor + comedi (mock SDK paths) |
| `full` | alias for `all_hardware` |

`driver-dover-motion` is **not wired** into `driver-registry` as of the
2026-03-13 capability matrix. Tracked via bead.

## `create_canonical_mock_registry()`

The preferred helper for tests that need the same universal-driver path
used by demos. It returns three deterministic devices:

- `stage` — universal ESP300 mock transport.
- `power_meter` — universal Newport 1830-C mock transport.
- `camera` — native `mock_camera`.

```rust
// crates/driver-registry/src/lib.rs:291
let registry =
    driver_registry::create_canonical_mock_registry(workspace_root).await?;
let stage = registry.get_movable("stage")?;
stage.move_abs(10.0).await?;
```

`workspace_root: &Path` is required to resolve universal-driver manifest
paths in the embedded `CANONICAL_MOCK_CONFIG`. The older
`hardware::registry::create_mock_registry` is `#[deprecated]` with a
pointer to this one.

See [`mock-registry.md`](./mock-registry.md) for fidelity levels and
scenario seeds.

## `DeviceRegistry`

Runtime phone book. `hardware::registry::DeviceRegistry` holds
`Arc<dyn Capability>` entries keyed by `DeviceId`. Capability lookups:

- `get_movable(&id) -> Result<Arc<dyn Movable>>`
- `get_frame_producer(&id) -> Result<Arc<dyn FrameProducer>>`
- …one accessor per capability trait.

Data plane: DashMap for lock-free reads.
Control plane: SQLite shadows desired state when the `db` feature is enabled;
the SQLite backend emits broadcast change notifications and the watch
reconciler converges the registry within ~300 ms on config change.

## Adding a new SDK driver

1. Create `crates/driver-<name>` and (if needed) `crates/driver-<name>/<name>-sys`.
2. Implement capability traits.
3. Implement `DriverFactory`.
4. Register the factory in `crates/driver-registry/src/lib.rs` behind a feature flag (`<name>` / `<name>_sdk` / `<name>_hardware`).
5. Update `Cargo.toml` feature stanzas.
6. Update `docs/reference/driver-capability-matrix.md`.
7. Create a page at [`../drivers/<name>.md`](../drivers/) and link from [`../index.md`](../index.md).

For non-SDK serial / TCP / SCPI devices, prefer
[`./driver-universal.md`](./driver-universal.md) instead.
