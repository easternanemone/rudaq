# `Parameter<T>` — reactive hardware state

<!--
last-ingested: 2026-04-19
sources:
  - crates/common/src/parameter.rs
  - docs/explanation/newcomer-guide.md §Parameter System
  - CLAUDE.md §Code Style
see-also:
  - ./capability-traits.md
  - ../invariants.md
  - ../architecture.md
-->

The canonical primitive for hardware state. **All** hardware state in this
codebase is a `Parameter<T>`. Do **not** use `Arc<Mutex<T>>`.

## Shape

- Generic over `T: Clone + Send + Sync + 'static` (scalar, struct, enum).
- Built on `Observable<T>` which wraps `tokio::sync::watch`.
- Pluggable async write callback (validates, pushes to hardware, updates internal value, broadcasts).

## Why

`Parameter<T>` unifies three concerns that would otherwise get duplicated
per device:

1. **Observability** — GUI widgets, scripts, and storage subscribe to changes without polling.
2. **Validation** — setter can reject invalid values (range, NaN, device-specific rules).
3. **Persistence** — values are snapshotted to HDF5 during runs.

## Flow for `set(value)`

```
user / script / GUI
  → Parameter::set(value)
     → validation (range, NaN, custom)
     → async write callback: BoxFuture<'static, Result<()>>
        → vendor SDK call (via driver crate)
     → internal value update on success
     → broadcast via watch channel
        → GUI redraw
        → storage tap
        → script awaiters
```

## Wiring a driver

```rust
let mut exposure = Parameter::new("exposure", 10.0)
    .with_unit("ms")
    .with_range_introspectable(1.0, 1000.0);   // UI hints

exposure.connect_to_hardware_write(|val| Box::pin(async move {
    camera_driver.set_exposure_ms(val).await
}));
```

- `with_range_introspectable` supplies GUI slider bounds (via `Parameterized` capability introspection).
- `connect_to_hardware_write` takes `impl Fn(T) -> BoxFuture<'static, Result<()>>`.
- Without a connected callback, `set` is pure in-memory — useful for pure mock state.

## `BoxFuture` — why boxed

Capability traits are `Arc<dyn Trait>`, which requires dynamic dispatch,
which requires boxed futures. Hence `BoxFuture<'static, Result<()>>` rather
than native `async fn` in trait. Do **not** migrate capability traits away
from `async_trait` / boxed futures without redesigning the dispatch model.

## `Observable<T>` vs `Parameter<T>`

- `Observable<T>` — bare change broadcast, no setter hook, no validation, no metadata.
- `Parameter<T>` — `Observable` + async setter + validation + metadata (unit, range, label).

Prefer `Parameter<T>` unless you have read-only derived state.

## `Parameterized` capability

A device implements `Parameterized` to expose its parameters to the
registry for introspection. This is what lets `GenericDevicePanel` auto-
compose a control panel from reported capabilities.

## Related

- [`capability-traits.md`](./capability-traits.md) — how `Parameterized` fits in.
- [`plan-run-engine.md`](./plan-run-engine.md) — the engine reads/writes parameters during plan execution.
- [`../invariants.md`](../invariants.md) §State management — hard rule that hardware state is Parameter, never Arc<Mutex>.
