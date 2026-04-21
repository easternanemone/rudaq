# crate: `experiment`

<!--
last-ingested: 2026-04-19
sources:
  - crates/experiment/
  - docs/explanation/architecture.md §RunEngine Composition
see-also:
  - ../concepts/plan-run-engine.md
  - ./scripting.md
-->

**Role:** Orchestration engine. Defines `Plan` and `PlanCommand` types,
hosts `RunEngine`, drives multi-device workflows.

**Key types:**

- `RunEngine` — state machine (`Idle` / `Running` / `Paused` / `Aborted`).
  Composed of:
  - `TaskQueue` (`run_engine/task_queue.rs`) — plan queue wrapping `Mutex<Vec<QueuedPlan>>`.
  - `WatchdogManager` (`run_engine/watchdog.rs`) — orphan-plan detection, 5-min default timeout.
- `Plan` — generator of `PlanCommand`.
- `PlanCommand` — `MoveTo`, `Trigger`, `Checkpoint`, `EmitEvent`, …
- `AcquisitionCoordinator` — multi-device coordination.
- `FeedbackEvent` channel + `execute_adaptive()` — adaptive scans.

**Built-in plans:** `Count`, `LineScan`, `GridScan`.

**Document stream types:** `StartDoc`, `DescriptorDoc`, `EventDoc`,
`StopDoc` (Bluesky-style). Consumed by `DocumentSink` impls in `storage`.

**Historical:** `ScanServiceImpl` (server-side) was replaced by
`RunEngineService` and deleted. Older docs describing this as a pending
deprecation are stale.

**Test-only factories** (inside `run_engine`, not production):
`GatedCamera`, `SpectrometerControl`. See the capability matrix for
coverage gaps.

**Dependents:** `server` (gRPC `RunEngineService`), `scripting` (Rhai bridge), `bin`.
