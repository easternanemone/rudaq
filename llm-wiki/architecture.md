# Architecture (LLM summary)

<!--
last-ingested: 2026-04-19
sources:
  - docs/explanation/architecture.md
  - docs/explanation/newcomer-guide.md
  - README.md
see-also:
  - ./crates/index.md
  - ./concepts/parameter.md
  - ./concepts/plan-run-engine.md
  - ./concepts/ring-buffer.md
  - ../docs/explanation/architecture.md  (authoritative, longer prose)
-->

Condensed map of the 30-crate workspace. For diagrams and the long-form
narrative, read `docs/explanation/architecture.md`.

## Core design principles

1. **Crash resilience** — daemon process owns hardware; GUI is a separate process. GUI crash never interrupts a running experiment.
2. **Capability-based hardware** — drivers compose atomic traits (`Movable`, `Triggerable`, …) instead of inheriting a device hierarchy.
3. **Hot-swappable logic** — experiments are Rhai scripts uploaded via gRPC.
4. **Zero-copy data path** — frames flow through an mmap ring buffer (Arrow IPC).

## Layer map

| Layer | Crates |
|-------|--------|
| Foundation | `common`, `common-traits`, `pool`, `protocol` |
| Hardware abstraction | `hardware` (HAL + `DeviceRegistry`) |
| Driver registry / feature gating | `driver-registry` |
| Mock + universal drivers (always compiled) | `driver-mock`, `driver-universal` |
| SDK drivers (feature-gated) | `driver-pvcam` (+ `pvcam-sys`), `driver-andor-sdk3` (+ `andor-sdk3-sys`), `driver-comedi` (+ `comedi-sys`) |
| Experimental SDK driver | `driver-dover-motion` (+ `dover-motion-sys`) exists but is not wired into `driver-registry` |
| Engine | `experiment` (`RunEngine`), `scripting` (Rhai), `daq-modules` |
| Services | `server` (gRPC tonic), `client`, `db` (SQLite only, via `rusqlite` — bd-2a2ne; no SurrealDB), `storage` (HDF5 / Arrow / Zarr / Parquet / TIFF) |
| Apps | `bin` (daemon), `ui` (egui native + WASM), `ui-graph`, `ui-slint` (experimental) |
| Domain | `echelle` (spectroscopy calibration / extraction / simulation), `atomic-reference` (NIST ASD line data) |
| Testing | `integration-tests` |

Authoritative list: `Cargo.toml` workspace members.

## Control flow (top-down)

```
User / Rhai script
  → RunEngine (experiment crate)
     → Plan (generator of PlanCommand: MoveTo, Trigger, Checkpoint, EmitEvent)
        → DeviceRegistry (hardware crate)
           → Capability trait (Movable / Triggerable / …)
              → Parameter<T> setter (common crate)
                 → BoxFuture hardware-write callback
                    → Driver (driver-* crate)
                       → Vendor SDK (via *-sys) or serial/TCP transport (driver-universal)
```

## Data flow (bottom-up, the Mullet)

```
Driver produces Arc<Frame>
  → Pool<Frame> (zero-alloc, lock-free) yields Loaned<Frame>
     → RingBuffer (mmap, Arrow IPC, seqlock)
        ├── Storage path: HDF5 writer (reliable, durable)
        └── Live path: gRPC stream (server → client) → egui ImageViewerPanel
```

Frame streaming path has buffer reuse at every hop (`compress_frame_into`,
`decompress_frame_into`, `convert_frame_to_rgba_into`). See
`docs/explanation/architecture.md §Frame Streaming Pipeline` and ADR-014.

## Persistence (3-tier hybrid)

| Tier | Tech | What |
|------|------|------|
| 1 — design-time | TOML files (git-tracked) | Hardware configs (`config/devices/*.toml`), calibration profiles, device manifests. **Source of truth.** |
| 2 — runtime control plane | SQLite (embedded, `rusqlite` + `tokio-rusqlite`, bd-2a2ne) | Parameter state, run records, device features, reconciliation. Only embedded DB backend; SurrealDB was removed. |
| 3 — science data | HDF5 / Arrow IPC / Zarr V3 / Parquet / TIFF | Frames, scan datasets, spectral profiles. Writers implement `DocumentSink`. |

Reconciliation loop: TOML shadow-writes to SQLite at startup; the SQLite backend broadcasts coarse change notifications and the watch reconciler converges `DeviceRegistry` (~300 ms). ADR-015 was written for the earlier SurrealDB design and is now historical where it names LIVE SELECT.

## Safety (3-layer stack)

1. **Safety-heartbeat task** (proactive): Tokio task in `crates/bin/src/safety_heartbeat_task.rs` (entry `spawn_heartbeat`) pulses a Comedi DIO channel driven by `HeartbeatConfig` from `[safety_heartbeat]` in the hardware config TOML. Daemon death stops the pulse; external interlock cuts laser power. Feature: `comedi_hardware`. (Note: "SafetyHeartbeat" is not a Rust type — the runtime is a task + config struct, not a `struct SafetyHeartbeat`.)
2. **`HardwareWatchdog`** (reactive): `crates/common/src/health/watchdog.rs` — dedicated OS thread; if the Tokio runtime hangs > 30 s (no kick), fires a 5-step shutdown: close shutters, disable emission, stop motors, zero DAQ outputs.
3. **Panic hook**: same 5-step shutdown runs from the panic handler via bridge threads + a pre-allocated emergency runtime.

ADR-004.

## Key abstractions in one sentence each

- `DeviceId` — `Arc<str>` identity for a device.
- `Parameter<T>` — reactive state with async write callback + validation + broadcast. ([`concepts/parameter.md`](./concepts/parameter.md))
- Capability traits — 30 small focused traits; devices compose them. ([`concepts/capability-traits.md`](./concepts/capability-traits.md))
- `Plan` + `RunEngine` — Bluesky-style orchestration. ([`concepts/plan-run-engine.md`](./concepts/plan-run-engine.md))
- `RingBuffer` — mmap/seqlock Arrow IPC streaming ring. ([`concepts/ring-buffer.md`](./concepts/ring-buffer.md))
- `DriverFactory` — trait each driver crate implements; registry instantiates from TOML. ([`concepts/driver-registry.md`](./concepts/driver-registry.md))

## Pointers to subsystems

- GUI auto-composition: `ui/src/widgets/device_controls/generic_panel.rs` renders widgets keyed off reported capabilities.
- RunEngine composition: `crates/experiment/src/run_engine/` — `task_queue.rs` + `watchdog.rs`.
- Frame pipeline details: `docs/explanation/architecture.md §Frame Streaming Pipeline`.
- Monitoring: webhook alerting (`server/src/alerting.rs`) + heartbeat JSONL (`server/src/health/heartbeat_log.rs`).
