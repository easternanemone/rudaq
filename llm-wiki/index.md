# Wiki Index

Catalog of every page in `llm-wiki/`. One-line summaries; follow links for
depth. Read this first on every query.

## Schema & meta

- [`README.md`](./README.md) — Human-facing pointer.
- [`schema.md`](./schema.md) — **Read first if editing.** Ingest / query / lint workflows, page conventions.
- [`log.md`](./log.md) — Append-only record of ingests, queries, lint passes.
- [`sources.md`](./sources.md) — Pointers to raw sources (docs/, ADRs, commits, handoffs, beads).

## Foundational

- [`architecture.md`](./architecture.md) — 30-crate workspace map + data flow (daemon / client / drivers / storage).
- [`glossary.md`](./glossary.md) — Domain vocabulary: DeviceId, Parameter<T>, DriverFactory, Plan, RunEngine, Bluesky, Rhai, bd, nextest, etc.
- [`invariants.md`](./invariants.md) — Hard rules that must hold (Parameter<T> not Arc<Mutex>, Rust 1.92.0 pinned, branch-before-commit, bd-only task tracking, etc.).

## Workflows

- [`workflows/build-test-lint.md`](./workflows/build-test-lint.md) — `cargo check` / nextest / clippy / fmt, CI parity slices.
- [`workflows/beads.md`](./workflows/beads.md) — `bd` (beads) issue tracker: ready, claim, close, dolt push.
- [`workflows/pr-workflow.md`](./workflows/pr-workflow.md) — Feature branch → commit → push -u → draft PR.
- [`workflows/hardware-testing.md`](./workflows/hardware-testing.md) — `hardware_tests` feature, nextest `hardware` profile, maitai / leabs-dev loops.

## Concepts (domain primitives)

- [`concepts/parameter.md`](./concepts/parameter.md) — `Parameter<T>` reactive state + `BoxFuture` callbacks. The default hardware-state primitive.
- [`concepts/plan-run-engine.md`](./concepts/plan-run-engine.md) — Bluesky-style `Plan` + `RunEngine` + document stream.
- [`concepts/capability-traits.md`](./concepts/capability-traits.md) — `Movable`, `Readable`, `FrameProducer`, `Triggerable`, `ExposureControl`, …  (24 traits).
- [`concepts/driver-universal.md`](./concepts/driver-universal.md) — TOML-manifest driven serial/TCP/SCPI devices. **Forward path** for new devices.
- [`concepts/driver-registry.md`](./concepts/driver-registry.md) — `DriverFactory` registration + feature gating + `create_canonical_mock_registry()`.
- [`concepts/ring-buffer.md`](./concepts/ring-buffer.md) — mmap + seqlock Arrow IPC ring for zero-copy streaming.
- [`concepts/device-id.md`](./concepts/device-id.md) — `DeviceId` (`Arc<str>`-backed) identity semantics.
- [`concepts/mock-registry.md`](./concepts/mock-registry.md) — Canonical mock registry, no feature flags, deterministic testing.

## Crates (one page per workspace member)

- [`crates/index.md`](./crates/index.md) — Crate graph + one-line summaries.
- Per-crate pages: see `crates/<name>.md` (29 entries).

## Hardware machines

- [`hardware/maitai.md`](./hardware/maitai.md) — 15 devices: PVCAM, Comedi, ELL14×3, MaiTai, ESP300×3, Newport PM.
- [`hardware/leabs-dev.md`](./hardware/leabs-dev.md) — 3 devices: Andor iStar, IPG YLPP-200, Thorlabs PM400.

## Drivers (per-driver implementation notes)

- [`drivers/pvcam.md`](./drivers/pvcam.md) — Photometrics PVCAM cameras.
- [`drivers/andor-sdk3.md`](./drivers/andor-sdk3.md) — Andor iStar + Shamrock.
- [`drivers/comedi.md`](./drivers/comedi.md) — Linux Comedi DAQ boards.
- [`drivers/dover-motion.md`](./drivers/dover-motion.md) — Dover Motion SmartStage (experimental, not wired into registry).
- [`drivers/universal.md`](./drivers/universal.md) — driver-universal (TOML manifest).
- [`drivers/mock.md`](./drivers/mock.md) — Mock drivers (always available).

## How this wiki is maintained

See [`schema.md`](./schema.md). TL;DR: agents ingest new sources (merged PRs,
ADRs, handoffs), update relevant pages, append to `log.md`. Periodic lint
passes catch stale claims and orphan pages.
