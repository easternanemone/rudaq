# `Plan` + `RunEngine` (Bluesky-style orchestration)

<!--
last-ingested: 2026-04-19
sources:
  - crates/experiment/src/run_engine/mod.rs
  - crates/experiment/src/run_engine/task_queue.rs
  - crates/experiment/src/run_engine/watchdog.rs
  - docs/explanation/architecture.md §RunEngine Composition
  - docs/explanation/newcomer-guide.md §Plan/RunEngine Orchestration
see-also:
  - ./capability-traits.md
  - ./parameter.md
  - ../architecture.md
-->

Declarative experiments. Inspired by Bluesky (NSLS-II).

## Plan

A `Plan` is a generator that yields `PlanCommand`s:

- `MoveTo(device_id, position)`
- `Trigger(device_id)`
- `Checkpoint` — safe pause point (user can resume or abort cleanly).
- `EmitEvent(...)` — produces an `EventDoc` in the document stream.
- …others (read, wait, set).

Concrete plan generators live under `crates/experiment/src`: `Count`,
`LineScan`, `GridScan`.

## RunEngine

Located at `crates/experiment/src/run_engine/mod.rs`. Executes plan
commands against a `DeviceRegistry`.

**State machine:** `Idle` → `Running` → `Paused` → `Running` or `Aborted`.

**Composition** (the RunEngine delegates rather than owning everything):

| Sub-component | File | Responsibility |
|---------------|------|----------------|
| `TaskQueue` | `task_queue.rs` | enqueue / dequeue / inspect / clear. Wraps `Mutex<Vec<QueuedPlan>>`. |
| `WatchdogManager` | `watchdog.rs` | Orphan-plan detection. Tracks last meaningful activity; background task auto-aborts after timeout (default 5 min). |

Keeps the RunEngine struct focused on state-machine transitions.

## Document stream (Bluesky model)

Each plan execution produces an ordered stream of typed documents:

1. **StartDoc** — metadata, plan arguments, run UID.
2. **DescriptorDoc** — schema of the data streams (field names, dtypes, shapes).
3. **EventDoc** — one per data point: `{time, data: {...}, ...}`.
4. **StopDoc** — success or failure status + exit reason.

Consumers implement `DocumentSink` (in `storage`). Built-in sinks:

- `HdfDocumentSink` — writes to HDF5.
- `ArrowDocumentSink` — Arrow IPC.
- `ZarrSink` (feature `storage_zarr`) — maps `scan_indices` to Zarr chunk coords.

This decoupling means the engine doesn't know which format is persisted —
just that sinks consume documents.

## Adaptive / feedback-driven execution

- `AcquisitionCoordinator` — multi-device coordination.
- `FeedbackEvent` channel — adaptive scans react to measurements.
- `execute_adaptive()` — feedback-driven plan execution.

## Capability check at engine boundaries

Plans reference devices by `DeviceId`. The engine looks up the device and
checks the requested capability at runtime:

```rust
let stage = registry.get_movable(&device_id)?;
stage.move_abs(position).await?;
```

A device missing the required capability yields a typed error *before*
hardware touches happen.

## Watchdog semantics

- Activity types counted: `MoveTo`, `Read`, `Trigger`, `EmitEvent`, `Checkpoint`.
- Pure waits (`Sleep`) **do** reset the timer — otherwise long dwell scans would misfire.
- Timeout default 5 min, configurable per-plan.
- Fire path: mark plan `Aborted`, run cleanup, emit `StopDoc { exit_status: TimedOut }`.

## Rhai integration

`scripting` crate exposes synchronous-looking Rhai wrappers over RunEngine
async calls. Scripts run in a sandbox with a 10 k operation limit and a
timeout. Hot-swap: upload via gRPC, execute immediately.
