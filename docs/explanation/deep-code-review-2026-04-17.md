# Deep Code Review: rust-daq

Date: 2026-04-17
Reviewer: Codex
Scope: repository-wide architectural and maintainability review
Primary deliverables:
- a validated review of the current codebase state
- a staged refactoring program focused on maintainability, performance, reliability, and user experience
- a beads execution graph that decomposes this work into independently actionable epics and tasks

---

## Executive Summary

`rust-daq` does not need a foundational rewrite. The workspace already has several strong properties:

- the workspace split is directionally correct
- the capability-trait model is a good fit for the domain
- the codebase is heavily tested
- the daemon, UI, and hardware layers are conceptually separate
- there is visible effort to reduce legacy coupling, especially through `driver-registry`

The main problem is not "bad architecture" in the abstract. The problem is that several critical seams are only partially finished, and a handful of oversized modules now carry too much system complexity. That has four practical effects:

1. Changes are harder than they should be because engineers must reason across giant files and mixed concerns.
2. Hot-path code pays avoidable overhead because design cleanup stopped short of execution-path cleanup.
3. Some abstractions remain doubled or transitional, which creates behavior drift across device implementations.
4. UI composition is becoming stateful and branch-heavy enough that user-facing consistency will degrade unless the shell is simplified.

The highest-leverage work is therefore not feature work and not ecosystem upgrades. It is targeted refactoring in five areas:

- complete the `hardware` / `driver-registry` boundary
- decompose PVCAM into generated patterns plus smaller modules
- collapse parameter setting onto a single contract
- remove per-frame `spawn_blocking` from ring-buffer writes
- simplify the UI app shell around a panel-controller model

This review also invalidates several recommendations from the earlier April refactoring memo. Some items in that plan are already fixed in source, and some were based on readings that no longer hold. That matters because stale refactor plans are expensive: they burn engineering time while appearing rigorous.

---

## Review Method

This review used direct source inspection rather than only prior planning artifacts.

Files inspected during the review included:

- `Cargo.toml`
- `docs/explanation/refactoring-plan-2026-04.md`
- `crates/driver-pvcam/src/components/features/mod.rs`
- `crates/driver-pvcam/src/lib.rs`
- `crates/hardware/src/registry.rs`
- `crates/server/src/grpc/server.rs`
- `crates/common-traits/src/capabilities.rs`
- `crates/experiment/src/run_engine/command_dispatch.rs`
- `crates/experiment/src/run_engine/mod.rs`
- `crates/experiment/src/run_engine/state_machine.rs`
- `crates/server/src/alerting.rs`
- `crates/ui/src/app/mod.rs`
- `crates/ui/src/app/tabs.rs`
- `crates/ui/src/panels/mod.rs`
- `crates/storage/src/ring_buffer.rs`
- `crates/driver-registry/src/lib.rs`

The goal of the review was not to find isolated bugs. It was to identify structural decisions that will compound positively or negatively over the next large set of features.

---

## What Is Healthy

Before describing refactors, it is worth stating what should be preserved.

### 1. Workspace shape is mostly correct

The workspace split in `Cargo.toml` is not arbitrary. The repository already distinguishes:

- foundational/shared crates
- hardware and driver concerns
- execution/orchestration
- persistence
- server
- multiple UI frontends
- integration tests

That is a good starting point. The next step is to finish the boundaries that are already implied by the crate layout.

### 2. Capability traits are the right abstraction level

The capability model in `crates/common-traits/src/capabilities.rs` is still the correct center of gravity for this codebase. The problem is not the existence of capability traits. The problem is transitional duplication around them.

### 3. The codebase has real testing discipline

Large inline test suites are inconvenient for maintainability, but they are also evidence that the project values behavior preservation. That matters because it makes aggressive decomposition feasible.

### 4. `driver-registry` is the right direction

The repository has already recognized that `hardware` should not be the concrete-driver god crate. That architectural instinct is correct. The refactor is simply incomplete.

---

## Validated Hotspots

The following module sizes and patterns were validated directly in the current tree.

| Area | File | Size / Evidence | Why it matters |
|---|---|---:|---|
| PVCAM feature logic | `crates/driver-pvcam/src/components/features/mod.rs` | 4482 LOC, 104 `get_` / `set_` accessors | Excessive duplication, difficult review surface, weak locality |
| PVCAM driver shell | `crates/driver-pvcam/src/lib.rs` | 3599 LOC | Capability impls, lifecycle, and driver glue remain co-located |
| Registry core | `crates/hardware/src/registry.rs` | 3685 LOC | Core registry, health, plugin loading, mock support, and tests in one file |
| gRPC server | `crates/server/src/grpc/server.rs` | 3069 LOC | Startup, services, storage plumbing, and tests remain interwoven |
| Common capabilities | `crates/common-traits/src/capabilities.rs` | 1975 LOC | Foundational API file still mixes many unrelated concerns |
| UI shell | `crates/ui/src/app/mod.rs` | 1133 LOC | App state becoming centralized and branch-heavy |
| UI tab router | `crates/ui/src/app/tabs.rs` | large panel dispatch match | UI behavior fragmented across control-flow branches |
| Storage tests | `crates/storage/src/ring_buffer.rs` | 3120 LOC total, tests start at line 1769 | Core implementation is harder to navigate than necessary |

These are not just large files. They are files where the size corresponds to mixed responsibilities rather than a single dense algorithm.

---

## Detailed Findings

## A. `hardware` and `driver-registry` still disagree about ownership

### Evidence

`driver-registry` explicitly documents the intended split:

- `hardware` should expose abstract APIs and `DeviceRegistry`
- `driver-registry` should own concrete factory wiring and feature-gated registration

But `hardware/src/registry.rs` still contains:

- `create_mock_registry()`
- `register_mock_factories()`
- large amounts of config/population logic
- plugin-manifest resolution
- health broadcasting
- a large inline test module

### Why this is a real problem

This is not just file-length discomfort. It creates architectural ambiguity:

- Where should new driver bootstrap logic live?
- Where should test-only registry constructors live?
- Which crate is allowed to know about concrete mock factories?
- How much of registry population is core behavior versus environment bootstrapping?

When a boundary is partially complete, every future change becomes a mini-architecture decision. That slows work and encourages more convenience coupling.

### Recommendation

Finish this boundary in a deliberate sequence:

1. Move mock-registry bootstrap out of `hardware/src/registry.rs`.
2. Keep `DeviceRegistry` and core capability-provider behavior in `hardware`.
3. Move registration/bootstrap composition into `driver-registry` or a dedicated support crate.
4. Split `registry.rs` into focused modules:
   - `core.rs`
   - `registration.rs`
   - `health.rs`
   - `plugin_loading.rs`
   - `test_support.rs`

### Expected payoff

- clearer ownership model
- easier onboarding for new contributors
- lower risk when changing device registration flow
- smaller blast radius for tests and plugin loading changes

---

## B. PVCAM has too much repeated control logic

### Evidence

`crates/driver-pvcam/src/components/features/mod.rs` is 4482 LOC and contains 104 `get_` / `set_` accessors. The repeated pattern is obvious:

- check parameter availability
- call PVCAM getter or setter
- map error
- fall back to mock state

This is repeated with small variations for temperature, fan speed, gain, readout port, scan parameters, exposure modes, and Prime/PP-related features.

### Why this is a real problem

This kind of repetition harms the project in three ways:

1. correctness changes are expensive because the same SDK pattern must be edited in many places
2. reviews become pattern-matching exercises rather than logic reviews
3. future engineers will be reluctant to clean it up because it looks "hardware delicate"

### Recommendation

Do not rewrite PVCAM behavior. Instead:

1. Introduce a declarative macro for the standard accessor pattern.
2. Group residual non-macro logic into smaller modules by domain:
   - `temperature`
   - `cooling`
   - `gain`
   - `readout`
   - `scan`
   - `prime_features`
   - `diagnostics`
3. Move capability implementations out of `driver-pvcam/src/lib.rs` into a `capabilities/` directory.

### Expected payoff

- large LOC reduction without semantic churn
- easier auditing of all SDK interactions
- much faster future feature work on camera parameters

### Important correction

The previous April memo’s suggestion that PVCAM mock locking still needed `parking_lot::Mutex` is stale. The mock state already uses `parking_lot::Mutex`. That should not be scheduled again.

---

## C. Parameter-setting still has two active contracts

### Evidence

`crates/experiment/src/run_engine/command_dispatch.rs` still handles parameter setting like this:

1. try `Settable`
2. return early if present
3. otherwise use `Parameterized`

At the same time, `crates/common-traits/src/capabilities.rs` still defines both:

- `Settable`
- `Parameterized`

And there are still multiple concrete `Settable` implementations in the tree.

### Why this is a real problem

This creates invisible behavior bifurcation:

- two devices can respond to the same `Set` command through different internal mechanisms
- metadata and coercion behavior can diverge
- future fixes must often be applied twice
- debugging user-facing parameter issues requires knowing device implementation details

This is exactly the sort of transitional abstraction that survives too long and becomes permanent accidental complexity.

### Recommendation

Make `Parameterized` the canonical path.

Implementation strategy:

1. keep `Settable` only as an adapter target
2. implement a compatibility adapter where needed
3. remove `Settable`-first dispatch from `RunEngine`
4. make all user-facing parameter writes flow through `ParameterSet`

### Expected payoff

- one code path for setting parameters
- more predictable coercion behavior
- easier UI and server integration
- better long-term trait hygiene

---

## D. The gRPC storage path still spends too much time scheduling

### Evidence

`crates/server/src/grpc/server.rs` still performs per-frame ring-buffer writes through:

`tokio::task::spawn_blocking(move || rb.write(&frame))`

This occurs in at least two storage paths.

### Why this is a real problem

If ring-buffer writes are frequent, `spawn_blocking` per frame is the wrong unit of work. Even if each write is individually cheap, repeatedly offloading trivial work creates:

- executor overhead
- additional latency variability
- harder-to-reason-about backpressure behavior

The write path should be designed around the real blocking characteristics of the ring buffer, not around a blanket assumption that every write must leave the async context immediately.

### Recommendation

Replace per-frame offload with one of these models:

- a dedicated sink task that owns the ring-buffer writer
- an inline fast path plus explicit fallback when true blocking/backpressure occurs

### Expected payoff

- lower steady-state overhead
- more predictable high-rate streaming behavior
- simpler profiling story

---

## E. The UI shell is centralizing too much state

### Evidence

`crates/ui/src/app/mod.rs` now owns:

- the app shell
- connection state
- daemon startup modes
- dock state
- per-panel state objects
- many separate maps for docked device panel variants
- native-only and WASM-only bootstrap logic

There are also two large constructor paths:

- native app initialization
- WASM app initialization

These share a large block of duplicated setup logic.

Meanwhile, `crates/ui/src/app/tabs.rs` contains a broad dispatch match that chooses panel behavior based on:

- layout mode
- advanced panel kind
- gRPC-driven config
- local TOML config
- generic panel fallback

### Why this is a real problem

This is not just a code cleanliness issue. It will affect user experience:

- reconnect behavior can drift across panel kinds
- native/WASM parity becomes harder to maintain
- unavailable panels are handled through stubs instead of a consistent availability model
- panel persistence and restoration remain coupled to branching logic rather than panel capabilities

### Recommendation

Refactor the UI shell around a panel-controller registry.

Concretely:

1. define a panel controller abstraction for docked device panels
2. collapse the many per-panel HashMaps into a single typed store
3. extract shared constructor/bootstrap logic for native and WASM
4. make "not available on this platform" a panel capability/state, not a special stub module

### Expected payoff

- more consistent UX across platforms
- simpler reconnect and refresh semantics
- easier addition of new panel types
- less app-shell fragility

---

## F. Large inline test modules are now a maintainability tax

### Evidence

Very large test sections exist inline in:

- `crates/hardware/src/registry.rs`
- `crates/storage/src/ring_buffer.rs`
- `crates/server/src/grpc/server.rs`
- `crates/driver-pvcam/src/lib.rs`

### Why this is a real problem

Inline tests are useful when they clarify local behavior. They become harmful when they dominate file size and obscure the implementation shape.

The issue here is not test quantity. It is test placement.

### Recommendation

Move large behavioral suites into `crates/integration-tests` or targeted test modules under `tests/`.

Keep only genuinely local unit tests inline.

### Expected payoff

- production files become easier to navigate
- module boundaries become easier to see
- integration behavior remains covered without hiding the implementation

---

## G. Some earlier refactor recommendations are stale or already done

This matters because the new beads plan should not duplicate work.

### Already fixed or not worth scheduling again

- alerting map pruning appears already implemented in `crates/server/src/alerting.rs`
- PVCAM mock state already uses `parking_lot::Mutex`
- some previously flagged PVCAM streaming-state concerns are documented as intentional
- `subscribe_frames()` is already explicitly deprecated in `common-traits`

### Implication

The next planning artifact should only create work for validated gaps, not for older theoretical concerns.

---

## Recommended Refactoring Program

## Phase 0: establish ownership and sequencing

Goal:
- convert this review into a durable execution graph

Deliverables:
- one parent epic
- several sub-epics by subsystem
- gate issues for cross-phase signoff
- child tasks with clear acceptance criteria and dependencies

This phase is planning and coordination only.

---

## Phase 1: architecture boundary completion

Primary target:
- `hardware` / `driver-registry`

Goals:

- finish the boundary the workspace already intends
- isolate bootstrap/test support from core registry logic
- reduce future architectural ambiguity

Suggested order:

1. move mock registry helpers out of `hardware/src/registry.rs`
2. split `registry.rs`
3. audit imports and crate ownership
4. update tests to match the new ownership model

---

## Phase 2: PVCAM maintainability reduction

Primary target:
- repeated accessor logic and monolithic driver shell

Goals:

- reduce LOC
- centralize SDK interaction patterns
- make hardware feature work safer to review

Suggested order:

1. add parameter accessor macro
2. split feature domains
3. split capability implementations from driver shell
4. move large behavior tests out of `lib.rs`

---

## Phase 3: execution and data-path simplification

Primary targets:

- parameter write path
- ring-buffer hot path
- possibly state-notification cleanup

Goals:

- one canonical parameter setting contract
- cheaper frame persistence path
- lower reasoning cost in orchestration and server layers

Suggested order:

1. migrate away from `Settable`-first dispatch
2. define compatibility adapter strategy
3. replace per-frame `spawn_blocking`
4. benchmark and validate latency/throughput changes

---

## Phase 4: UI shell cleanup

Primary target:
- `crates/ui/src/app`

Goals:

- reduce shell complexity
- improve native/WASM parity
- make panel state management more regular

Suggested order:

1. extract shared bootstrap
2. define panel-controller abstraction
3. unify docked panel state storage
4. replace platform stubs with explicit availability handling where practical

---

## Phase 5: test and polish sweep

Primary target:
- move oversized test suites
- align dependency/version inconsistencies

Goals:

- keep behavior coverage
- reduce source navigation burden
- remove low-value drift

Suggested order:

1. move large inline tests
2. align HDF5 version declarations
3. schedule ecosystem upgrades only after structural changes settle

---

## What Not To Do

The following would create a lot of churn for little value right now:

- do not rewrite the capability-trait model
- do not merge crates just because files are large
- do not begin with a dependency upgrade campaign
- do not redesign the whole UI visually before simplifying the shell
- do not rewrite PVCAM logic by hand when macros and extraction solve the real problem
- do not schedule already-fixed cleanup items from the April memo

---

## Recommended Beads Structure

The beads plan created from this review should have:

- one top-level epic for the program
- one gate epic or gate set for phase approvals
- one sub-epic each for:
  - architecture boundary completion
  - PVCAM decomposition
  - execution/data-path cleanup
  - UI shell refactor
  - test and hygiene follow-through

Each child task should be independently pick-up-able by another agent. That means every task should contain:

- exact files or modules in scope
- what to avoid changing
- acceptance criteria
- dependencies
- suggested verification commands
- design notes where the migration path is not obvious

---

## Final Priority Order

If engineering capacity is limited, the order should be:

1. complete `hardware` / `driver-registry` ownership
2. decompose PVCAM through macro extraction
3. unify parameter setting onto `Parameterized`
4. remove per-frame `spawn_blocking` from storage writes
5. refactor the UI shell around a panel-controller model
6. move large inline tests and align low-level dependency drift

That ordering produces the highest maintainability gain while also improving reliability and performance in the places that matter most.

---

## Appendix: Key Source Anchors

- `crates/driver-pvcam/src/components/features/mod.rs`
- `crates/driver-pvcam/src/lib.rs`
- `crates/hardware/src/registry.rs`
- `crates/driver-registry/src/lib.rs`
- `crates/server/src/grpc/server.rs`
- `crates/server/src/alerting.rs`
- `crates/common-traits/src/capabilities.rs`
- `crates/experiment/src/run_engine/command_dispatch.rs`
- `crates/experiment/src/run_engine/mod.rs`
- `crates/ui/src/app/mod.rs`
- `crates/ui/src/app/tabs.rs`
- `crates/ui/src/panels/mod.rs`
- `crates/storage/src/ring_buffer.rs`
