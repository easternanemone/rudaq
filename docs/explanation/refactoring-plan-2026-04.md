# rust-daq Comprehensive Refactoring Plan

**Date:** 2026-04-16
**Tracking issue:** bd-ckbf1
**Scope:** 29 workspace crates, ~305 KLOC
**Methodology:** Six parallel deep-dive audits (architecture, error handling, async/perf, trait design, code health, modernization) cross-referenced for convergence.

---

## 1. Executive Summary

The codebase is **architecturally sound** — strict layering is intact, the capability-trait abstraction scales, the error-contract design is documented and mostly implemented correctly, and the async runtime patterns are reasonable. There are **no foundational rewrites required**.

What it needs is **targeted decomposition and contract-completion**. Six independent audits converged on the same hot spots:

1. **God modules** (PVCAM features 4482 LOC, hardware/registry 3685, server/grpc/server 3069, etc.) — these are now blocking new feature work, not just aesthetics.
2. **Incomplete in-flight refactors** — `SdkStreamingState` was supposed to consolidate 7 mutex fields into one enum, but 3 channel fields remain outside it. This is a latent deadlock surface.
3. **Boilerplate that should be a macro** — ~30 PVCAM parameter accessors are 90% identical (~2000 LOC reducible).
4. **Two API duplications** — `Settable` vs `Parameterized`, `subscribe_frames` vs `register_primary_output` — both have a deprecated path that hasn't been removed.
5. **Per-frame `spawn_blocking` on ring-buffer write** — 50–200 µs of unnecessary task overhead per frame at 1000 fps.
6. **Drop-path safety gap in `PvcamDriver`** — panic during acquisition does not synchronously drive the camera safe.

The plan below is sequenced so each phase unblocks the next: PVCAM cleanup first (it's both the largest god module and the location of two safety gaps), then registry extraction (clarifies hardware crate), then the contract-completion sweep.

---

## 2. Convergent Findings (Cross-Audit Signal)

These items appeared in **three or more** of the six audits. They are the highest-confidence priorities.

> **Verification note (2026-04-16):** Before any edits, every "convergent" finding was opened in source. Four of the originally listed sub-findings turned out to be **already-fixed or based on a misread of the code** and have been struck below. This is recorded as a warning to future readers: cross-audit "convergence" is not a substitute for reading the file.

| # | Finding | Audits agreeing | Effort | Priority |
|---|---------|-----------------|--------|----------|
| C1 | `driver-pvcam/src/components/features/mod.rs` (4482 LOC) needs decomposition + macro extraction | Architecture, Code health, Reliability (mock locks) | M (3 d) → L (1 wk with macro) | **P0** |
| C2 | `hardware/src/registry.rs` (3685 LOC) should be split (or extracted into its own crate) | Architecture, Code health | L (3–4 d) | **P0** |
| ~~C3~~ | ~~Complete the `SdkStreamingState` refactor — move `reliable_tx`, `primary_tx`, `metadata_tx` into the enum variant~~ | ~~Reliability, Async/perf~~ | — | **FALSIFIED — see §2.1** |
| C4 | `Settable` ↔ `Parameterized` dual dispatch path in `command_dispatch.rs` | Trait design, Architecture | M (2 d) | P1 |
| C5 | `server/src/grpc/server.rs` (3069 LOC) should be split into 5 modules | Architecture, Code health | S (1–2 d) | P1 |
| C6 | Per-frame `spawn_blocking(rb.write())` adds 50–200 µs/frame | Async/perf, Reliability (RingBuffer warning) | M (2 d) | P1 |
| C7 | `tracing::instrument` not used; structured logging is ad-hoc | Modernization | M (2–3 h, opportunistic) | P2 |
| C8 | `tokio 1.36 → 1.40+` and `tonic 0.10 → 0.12` upgrade | Modernization | H (4 h, breaking) | P2 |

### 2.1 Falsified findings (verified against source 2026-04-16)

| ID | Original claim | What the source actually shows | Disposition |
|----|----------------|--------------------------------|-------------|
| A4 / C3 | `reliable_tx`, `primary_tx`, `metadata_tx` should be moved into `SdkStreamingState::Streaming` | These channels are **registration sinks** with a different lifecycle than the streaming state — they are set by external callers and persist across start/stop. The block comment at `acquisition/mod.rs` explicitly documents this. Moving them would break re-registration after a restart. | **Drop.** No-op. |
| A7 | `std::thread::sleep(Duration::from_millis(1))` polling sites in `frame_loop.rs` need `yield_now()` | The bd-3gnv refactor is **already done** (`frame_loop.rs:1568` uses `yield_now()` with the comment marker). The remaining sleeps at lines 296/642/864/879 are **intentional** backpressure throttles, not busy-wait substitutes. | **Drop.** Already fixed. |
| A8 | TOCTOU race in `attach_choice_listeners` between `is_none()` and `as_ref().unwrap().clone()` on `speed_table` | `speed_table` is a plain `Option<Arc<SpeedTable>>` field with **no interior mutability** and `attach_choice_listeners` borrows `&self`. Concurrent mutation is impossible by Rust's borrow rules. | **Drop.** No race. |
| C2 (server) | `module_service.rs` maps anyhow → `Status::internal(err.to_string())` without a downcast chain | `module_service.rs:50–59` already calls `anyhow_to_status(err)`, the canonical downcast. Nothing to fix. | **Drop.** Already correct. |
| C3 (server) | `StreamLimiter::active_streams` grows unbounded; needs LRU | `streaming.rs:268–270` removes the entry when count reaches 0. Map is bounded by currently-streaming clients × MAX_STREAMS_PER_CLIENT. | **Drop.** Self-bounding by design. |
| B3 (hardware) | `lib_reload.rs:372–407` has `TempDir::new().unwrap()` panics that crash the registry on full `/tmp` | All matches are inside `#[cfg(test)] mod tests` — production `StatePreserver` never uses `TempDir`. | **Drop.** No production panic site exists. |

---

## 3. The Plan, by Theme

### Theme A — PVCAM driver decomposition (P0, ~2 weeks)

The PVCAM driver is the single largest source of LOC concentration in the workspace and the source of three independent reliability/correctness findings. It is **the natural starting point** because the work is well-isolated, well-tested (extensive `tests/buffer_overflow.rs`, `acquisition_timing.rs`, `frame_integrity.rs`), and unblocks other work.

**A1. Extract PVCAM parameter accessor macro** — `crates/driver-pvcam/src/components/features/mod.rs:1–4482`
- ~30 `get_*`/`set_*` pairs follow an identical SDK-pattern: availability check → FFI call → mock fallback. Each is 15–30 LOC.
- Build a `declare_param_accessor!` declarative macro that takes (PVCAM param const, Rust type, transform, mock-state field) and emits the pair.
- **Expected reduction:** ~2000 LOC; single source of truth for the SDK interaction pattern.
- **Risk:** Medium. Mitigated by 1700-LOC `frame_integrity.rs` test suite and existing `mock_state` coverage.

**A2. Decompose remaining `features/mod.rs` into 7 domain modules** — same file
- After A1, split residual logic by domain: `temperature.rs`, `cooling.rs`, `readout.rs`, `gain.rs`, `prime_features.rs`, `diagnostic.rs`, `availability.rs`.
- Each module ~200–400 LOC, independently testable.
- **Expected:** `features/mod.rs` shrinks from 4482 → ~500 LOC of glue.

**A3. Decompose `driver-pvcam/src/lib.rs` capability impls** — `crates/driver-pvcam/src/lib.rs:1–3599`
- Move each capability impl into its own file under `capabilities/`: `exposure.rs` (400 LOC), `trigger.rs` (300), `frame_producer.rs` (800), `parameterized.rs` (700), `commandable.rs` (200).
- `lib.rs` shrinks to ~200 LOC for factory + struct definition.

**A4. ~~Complete the `SdkStreamingState` refactor~~** — **FALSIFIED.** See §2.1. The `*_tx` channels are intentionally outside the enum (registration-lifecycle, not streaming-lifecycle). No change.

**A5. Replace mock-state `std::sync::Mutex` with `parking_lot::Mutex`** — `crates/driver-pvcam/src/components/connection.rs:158` (declaration) + 28 `.lock().unwrap()` sites in `crates/driver-pvcam/src/components/features/mod.rs`
- Eliminates lock-poisoning panic risk in mock paths. Drop-in change after Mutex import swap.

**A6. Defensive shutdown in `PvcamDriver::Drop`** — `crates/driver-pvcam/src/lib.rs:3530–3554`
- Today, `Drop` only sets a flag and warns; PvcamAcquisition's own Drop performs the actual abort. **Verified safe** but the warn message is silent on whether the abort succeeded. Improvement: forward the abort result to the log so post-mortem analysis can distinguish "driver dropped cleanly" from "abort raced with frame poll".
- The originally-proposed `block_in_place` shutdown is **rejected** because it can deadlock the runtime if Drop runs from inside a Tokio task. Document the existing layered defense (Drop → PvcamAcquisition::Drop → HardwareWatchdog) in `docs/adr/004-panic-safety.md` instead.

**A7. ~~`yield_now()` polling refactor~~** — **FALSIFIED.** See §2.1. bd-3gnv is already implemented at `frame_loop.rs:1568`; the remaining `sleep(1ms)` sites are intentional throttles.

**A8. ~~Defer race in `attach_choice_listeners`~~** — **FALSIFIED.** See §2.1. No interior mutability on `speed_table`; no race possible.

---

### Theme B — Hardware crate boundary clarification (P0, ~1 week)

**B1. Decompose `hardware/src/registry.rs`** — `crates/hardware/src/registry.rs:1–3685`
- Two viable approaches; pick one based on team appetite:
  - **B1a (recommended, lower-risk):** Split in place into `registry/{core, capabilities, health, validation, pooling}.rs`.
  - **B1b (higher-leverage):** Extract a new `device-registry` crate. Used by `hardware`, `server`, `experiment`, `daq-modules`, `ui`. Reduces `hardware` from ~6 KLOC to ~2.5 KLOC and clarifies that the registry is a foundational service.
- B1b is architecturally cleaner but requires updating ~10 dependent crates' imports. Recommend B1a as a stepping stone if appetite is constrained.

**B2. Decompose `hardware/src/manifest_driver/driver.rs`** — `crates/hardware/src/manifest_driver/driver.rs:1–2106`
- Manifest-driven device construction has accreted. Split by concern: parsing, transport binding, capability assembly, lifecycle.

**B3. ~~Add structured-error wrapping to dynamic library reload~~** — **FALSIFIED.** Every `.unwrap()` and `TempDir` reference in `lib_reload.rs` is inside the `#[cfg(test)] mod tests` block (lines 372, 388, 397, plus the `.unwrap()`s on `save`/`load`/`clear`). The production `StatePreserver` has no panic-on-disk-full risk because it never touches `TempDir`. No change needed.

---

### Theme C — Server / gRPC layer cleanup (P1, ~1 week)

**C1. Decompose `server/src/grpc/server.rs`** — `crates/server/src/grpc/server.rs:1–3069`
- Split into `server/{builder, service_registry, health, shutdown, reflection}.rs`. Existing tests cover all paths. Mostly mechanical extraction.

**C2. ~~Add downcast chain to `module_service.rs`~~** — **FALSIFIED.** See §2.1. `module_service.rs:50–59` already calls `anyhow_to_status(err)`. No change needed.

**C3. ~~Bound the `active_streams` map~~** — **FALSIFIED.** `streaming.rs:268–270` already calls `entry.remove()` when the per-IP count drops to 0. The map only contains currently-streaming clients; the gRPC server's own connection cap bounds total active-client count. No leak.

**C4. Bound the webhook rate-limit map** — `crates/server/src/alerting.rs:37`
- `DashMap<String, Instant>` grows unbounded — `check_rate_limit` only `.get()`s and `.insert()`s, never `.remove()`s. Keys include per-run-UID abort keys (`abort:{run_uid}`), so every aborted plan permanently adds one entry. Fix with opportunistic pruning: when `len() > 1024`, retain entries newer than `2 * rate_limit` (anything older is already past its rate-limit window so the entry is dead weight).

**C5. Add panic hook for safety-interlock forensics** — `crates/bin/src/main.rs`
- Register `std::panic::set_hook` that logs `"FATAL: daemon panic detected; safety interlock should activate"` with timestamp before the runtime tears down. Trivial; high forensic value.

---

### Theme D — Engine and trait-system contract completion (P1, ~1 week)

**D1. Unify `Settable` onto `Parameterized`** — `crates/experiment/src/run_engine/command_dispatch.rs:74–87`
- Today: RunEngine dispatches `PlanCommand::Set` through both `Settable::set_value` and `Parameterized::set_json`. Drivers must choose; some implement both.
- Make `Parameterized` canonical; provide a `SettableAdapter<T: Parameterized>` blanket-implementor for the remaining 19 `Settable` impls. Plan a deprecation cycle of `Settable` over 2 releases.

**D2. Fully deprecate `FrameProducer::subscribe_frames`** — `crates/common-traits/src/capabilities.rs:483`
- Add `#[deprecated(since = "x.y", note = "use register_primary_output for pooled frames")]`.
- Remove in v2.0 release.

**D3. Add marker traits for composed capabilities** — `crates/common-traits/src/capabilities.rs`
- Document and enable runtime introspection:
  ```
  pub trait Camera: FrameProducer + ExposureControl + Triggerable {}
  impl<T: FrameProducer + ExposureControl + Triggerable> Camera for T {}
  ```
- Same for `Spectrometer`, `MotionStage`. One-liner each, zero impact on existing impls.

**D4. Decompose `common-traits/src/capabilities.rs`** — same file (1975 LOC)
- Split by domain: `capabilities/{readable, movable, controllable, frame, spectrum, safety}.rs`. Mostly file moves; preserve `pub use` re-exports for backward compatibility.

**D5. Plan `PlanCommand` extensibility (deferred to v2.0)**
- Today, adding a command requires editing the `PlanCommand` enum, `command_dispatch.rs` match, and serializers. Plugins cannot extend.
- For v2.0: introduce `PlanCommand::Custom(Box<dyn PlanCommandExt>)` variant with a `CommandRegistry`. **Do not implement now**; just document the change as the path forward in `docs/adr/`.

---

### Theme E — Performance hot paths (P1, ~3 days)

**E1. AsyncRingBuffer fast path** — `crates/server/src/grpc/server.rs:317, 1779`
- Today: every frame write goes through `tokio::task::spawn_blocking(move || rb.write(&frame))`. At 1000 fps, that's 50–200 ms/sec of pure scheduling overhead.
- Build an `AsyncRingBuffer` wrapper that does the write inline (the ring buffer is mmap-backed and pre-allocated, so it almost never blocks); only spawn-blocking on real backpressure.
- Estimated CPU savings: 5–15% throughput at high frame rates.

**E2. Switch RunEngine and supervisor state streams to `watch::Receiver`** — `crates/experiment/src/run_engine/state_machine.rs`, `crates/hardware/src/supervisor.rs`
- Today: `broadcast::channel(256)` for state updates. State has only one "current" value — `watch` is the natural fit. Reduces buffer memory ~4 KB/stream and gives subscribers the latest state on first read.

**E3. Move `block_on_parameter_set` runtime detection to once-per-connection** — `crates/common-traits/src/parameter.rs:495–521`
- Each parameter `set_json` call runs `Handle::try_current().runtime_flavor()`. Cache the flavor at gRPC service init. Saves 5–20 µs × 1000 params on bulk parameter scans.

---

### Theme F — Modernization (P2, opportunistic)

**F1. Coordinated upgrade: tokio 1.36 → 1.40+, tonic 0.10 → 0.12, prost 0.12 → 0.13**
- These three must move together. Allow ~4 hours including integration test sweep. Not urgent but the gap will keep widening.

**F2. Harmonize `hdf5-metno` version pinning**
- Inconsistent across `common-traits` (0.12.3) vs `common`/`storage` (0.12.4). Move to `[workspace.dependencies]`. ~20 min.

**F3. Upgrade `bindgen` 0.69 → 0.70 in all `*-sys` crates**
- Faster FFI build times; verify against PVCAM/Andor/Comedi headers.

**F4. Add `#[tracing::instrument]` opportunistically**
- Zero usages today. Don't sweep — instrument as files are touched. Capability-trait method impls are the highest-value targets.

**F5. Rename `Switchable` → `OnOffControllable`**
- Resolves naming overlap with `ShutterControl`. Single-rename refactor.

**F6. Feature-flag rename: `pvcam_sdk` → `pvcam_hardware`** (with deprecation alias)
- Brings PVCAM into uniform naming with `comedi_hardware`, `andor_hardware`.

---

### Theme G — Test hygiene (P2, ongoing)

**G1. Move large inline test modules to `crates/integration-tests/`**
- `crates/storage/src/ring_buffer.rs` has ~800 LOC of inline `#[cfg(test)]` tests for mmap behavior. Move to `crates/integration-tests/tests/ring_buffer_*.rs`.
- Same treatment for the ~400 LOC inline tests in `crates/driver-pvcam/src/lib.rs`.

**G2. Document `#[ignore]` reasons** — 22 ignored tests across the workspace
- Add a single-line `// ignored: requires PVCAM SDK` comment to each. Aids the hardware-test workflow.

**G3. Magic-number sleep audit** — e.g. `bin/tests/golden_lifecycle.rs` has `sleep(8s)` with no rationale
- Either name the constant with a comment, or replace with a deterministic readiness check.

---

## 4. Anti-Patterns: What NOT to Do

The audits explicitly recommend **against** the following, despite surface plausibility:

- **Do not migrate `async_trait` → native AFIT.** All 168 usages serve `Arc<dyn Trait>` dynamic dispatch (DeviceComponents, CapabilityProvider). Native AFIT does not yet support this with `Send + Sync` bounds. The current design is correct; the audits unanimously agreed.
- **Do not split `*-sys` crates back into their parents.** Conventional and useful for build-script isolation; merging gains nothing.
- **Do not consolidate the `Mock*` profile types into `driver-universal`.** They serve a distinct testing purpose (deterministic scenarios, RNG seed) that the manifest driver does not.
- **Do not remove `mimalloc`.** Profiling supports the choice for camera streaming workloads.
- **Do not redesign `DeviceComponents` (yet).** At 30 fields it's annoying but manageable; revisit at 50+ traits with a `TypeMap`-backed facade.
- **Do not unify `ui-graph` and `experiment::Plan` model now.** The Rhai code-generation path works and is documented; unification needs design consensus first.
- **Do not change the four-layer error contract.** It's well-designed (DaqError → anyhow → gRPC Status → ClientError). The fixes (Theme C2) close gaps in *application*, not in *design*.

---

## 5. Sequenced Execution Plan

**Phase 1 (weeks 1–2): PVCAM cleanup + safety**
- A1, A2, A3 (decomposition + macro)
- A5 (parking_lot for mocks) — **first commit, lowest risk**
- A6 (Drop-path log forwarding only — `block_in_place` rejected)
- C5 (panic hook in `main.rs`)
- C4 (bounded `last_alert` map in `alerting.rs`; ~~C3~~ already self-bounding)
- ~~A4, A7, A8~~ falsified during verification
- **Outcome:** PVCAM shrinks from 7.1 KLOC to ~4 KLOC; mock-state lock-poisoning eliminated; safety forensics improved; two unbounded-growth maps bounded.

**Phase 2 (week 3): Hardware boundary**
- ✅ B1a (registry decomposition, in-place split): `registry.rs` (3685 LOC) → `registry/mod.rs` (1911) + `tests.rs` (1232) + `types.rs` (293) + `loading.rs` (287). Three pure mechanical commits, no behavior change.
- ~~B3~~ falsified (production code has no panic risk)
- ~~C2~~ already correct (falsified)
- **Outcome:** `hardware` crate is navigable. Production code per file is at least 60% smaller; tests + helpers are siblings rather than buried inside a 3.7 KLOC monolith.

**Phase 3 (week 4): Server & engine**
- C1 (server.rs decomposition)
- D1 (Settable/Parameterized unification — start of deprecation cycle)
- D3, D4 (capability marker traits + capabilities.rs decomposition)
- E1 (AsyncRingBuffer fast path)
- E2 (watch channels for state)
- **Outcome:** Server startup is readable; engine dispatch is single-path; per-frame overhead drops measurably.

**Phase 4 (ongoing): Modernization & polish**
- F1 (tokio/tonic upgrade — schedule a coordinated half-day)
- F2, F3, F5, F6
- G1, G2, G3
- D2 (deprecate `subscribe_frames`)
- **Outcome:** No longer drifting from ecosystem; deprecation cycle begun for v2.0.

---

## 6. Total Effort & Outcome

| Phase | Calendar | LOC reduced | Reliability fixes | Performance |
|-------|----------|-------------|-------------------|-------------|
| 1 | 2 weeks | ~3 600 | 4 (drop, mock locks, race, deadlock) | 1 (jitter) |
| 2 | 1 week | ~1 200 | 3 (gRPC mapping, 2× memory leaks) | — |
| 3 | 1 week | ~1 500 | 1 (engine dispatch dual-path) | 3 (rb write, watch, param) |
| 4 | ongoing | ~200 | — | — |
| **Total** | **~4 weeks focused** | **~6 500** | **8 closed** | **4 wins** |

LOC reduction comes from macro extraction (A1, ~2000), test relocation (G1, ~800), and decomposition compression (overlap removal). Net file count grows (~30 new modules) but per-file complexity drops sharply.

---

## 7. Open Questions for the Team

1. **B1a vs B1b** — local registry split, or extract a new `device-registry` crate? B1b is architecturally cleaner; B1a is lower-risk.
2. **`ui-slint` commitment** — is it a real alternative to `ui` or an experiment? Needs a `[features] experimental` gate either way.
3. **`PlanCommand` v2.0 visitor pattern** — worth ADR-ing now even though implementation is deferred?
4. **Macro vs proc-macro for A1** — `macro_rules!` is sufficient and lighter; proc-macro buys better diagnostics. Recommend `macro_rules!` first.
