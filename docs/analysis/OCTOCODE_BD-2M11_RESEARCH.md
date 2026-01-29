# Octocode Research: bd-2m11 Epic (Hardware Test Suite Failures & Technical Debt)

**Date:** 2025-01-29  
**Method:** Octocode MCP (`localSearchCode`, `localGetFileContent`, `localViewStructure`) — research-first, evidence-based.  
**Purpose:** Thoroughly review the codebase and inform refactoring of beads issues/epics. No implementation until findings are synthesized.

---

## 1. Epic Context

- **Epic:** bd-2m11 — Hardware Test Suite Failures & Technical Debt (2026-01-28)
- **Children:** bd-2m11.1 (Comedi), .2 (Colorbar), .3 (E2E scripts), .4 (Frame observer), .5 (ELL14 retry), .6 (gRPC metrics), .7–.8 (warnings), .9 (std::thread::sleep audit)
- **Scope:** 11 unique test failures (138 total with retries) on maitai-eos; related warnings and technical debt.

---

## 2. Findings by Task

### 2.1 bd-2m11.3 — E2E Script Execution (P0)

**Tests:** `test_e2e_script_count_plan`, `test_e2e_script_linescan_plan`, `test_e2e_script_multiple_plans`  
**Files:** `crates/rust-daq/tests/e2e_script_execution.rs`, `crates/scripting/src/script_runner.rs`, `rhai_engine.rs`, `yield_bindings.rs`, `plan_bindings.rs`

**Evidence:**

- E2E tests use `ScriptPlanRunner::new(run_engine)`, `runner.run(script).await.expect(...)`. Scripts call `yield_plan(__yield_handle, count(...))` or `line_scan(...)`.
- `ScriptPlanRunner::run` (script_runner.rs ~147–197):
  - Builds `YieldChannelBuilder` → `(yield_handle, yield_rx, result_tx)`.
  - **Spawns `std::thread::spawn`** and calls `run_script_blocking(script, handle)`.
  - `run_script_blocking` uses `RhaiEngine::with_yield_support()`, which registers `plan_bindings` (e.g. `count`, `line_scan`) and `yield_bindings` (`yield_plan`), and injects `__yield_handle` into scope (rhai_engine.rs, yield_bindings).
- Tests register mock `stage_x` and `power_meter` via `DeviceRegistry` + `create_test_registry()`.

**Root-cause hypotheses (to validate):**

1. **Engine/scope mismatch:** `run_script_blocking` builds a fresh `RhaiEngine::with_yield_support()` and sets `__yield_handle` in scope. E2E scripts assume `__yield_handle` and `count`/`line_scan`/`yield_plan` exist. Verify that the same engine/scope is used for both registration and E2E script execution.
2. **Blocking thread vs. tokio:** Script runs in `std::thread::spawn`; plan execution is async (RunEngine). Cross-thread communication via `yield_rx`/channels — possible race or lifecycle issue (e.g. `script_done_rx` / timeout) affecting robustness.
3. **Error visibility:** `.expect("Script execution failed")` hides Rhai `EvalAltResult`. Bead suggests `unwrap_or_else(|e| panic!(...))` to surface script errors (missing fn, type mismatch, etc.).

**Refactor suggestions:**

- Keep **diagnosis-first** change: replace `.expect` with `unwrap_or_else` in E2E tests so failures show real Rhai errors.
- Add an **investigation** bead (or sub-task under bd-2m11.3): trace `run_script_blocking` → `with_yield_support` → `register_yield_bindings` / `register_plans` and confirm E2E uses the same code path. Document where `__yield_handle` is set and consumed.
- Consider **integration** bead: E2E environment (mock registry, RunEngine, feature flags) vs. maitai-eos (real hardware, different features). Clarify if failures are env-specific.

---

### 2.2 bd-2m11.2 — Colorbar Midpoint Gamma (P1)

**Tests:** `test_midpoint_adjustment_darkens`, `test_midpoint_adjustment_brightens` (colorbar.rs ~487–512)  
**File:** `crates/ui/src/widgets/colorbar.rs`

**Evidence:**

- `apply_adjustment` (lines 95–110):
  - Linear fast path: `midpoint == 0.5` → identity.
  - Otherwise: `gamma = -LN_2 / midpoint.ln()`, then `value.powf(gamma).clamp(0, 1)`.
- Docstring: `midpoint < 0.5` → darken; `midpoint > 0.5` → brighten.
- Bead analysis: **gamma formula inverted**. Current form gives opposite effect (e.g. midpoint 0.3 brightens, 0.7 darkens). Correct form: `gamma = midpoint.ln() / 0.5_f32.ln()`.

**Refactor suggestions:**

- Fix gamma per bead: use `self.midpoint.ln() / 0.5_f32.ln()` (or equivalent).
- **Regression risk:** User-facing; midpoint may be persisted (egui storage, config). Bead suggests: grep for midpoint persistence, consider release note / migration.
- Add a **small** bead or checklist item: “Verify no persistence of midpoint” before release.

---

### 2.3 bd-2m11.1 — Comedi Feature Flag / SIGABRT (P1)

**Test:** `test_comedi_discover_returns_vec`  
**Files:** `crates/daq-driver-comedi/src/device.rs` (lines 741–748), `comedi-sys` (panic message, `#[cfg(feature = "comedi-sdk")]`)

**Evidence:**

- `comedi_discover()` (device.rs) invokes comedi APIs. Without `comedi-sdk`, `comedi-sdk`-guarded code panics with “comedi function called but comedi-sdk feature is not enabled.”
- Test (741–748): calls `comedi_discover()` with **no** `#[cfg(feature = "comedi-sdk")]`, so it runs even when the feature is off → SIGABRT.

**Refactor suggestions:**

- **Quick fix (bead Option A):** `#[cfg(feature = "comedi-sdk")]` on `test_comedi_discover_returns_vec`.
- **Better design (Option C):** Make `comedi_discover` non-panicking when `comedi-sdk` is disabled (e.g. return `Ok(Vec::new())`), and add a **separate** test for the “no feature” path.
- Keep bd-2m11.1 as-is; optionally add a follow-up task for Option C.

---

### 2.4 bd-2m11.6 — gRPC Frame Rate Metrics (P1)

**Test:** `test_stream_frames_rate_limiting_and_metrics`  
**File:** `crates/rust-daq/tests/grpc_integration_test.rs` (lines 291–344)

**Evidence:**

- Test starts stream, reads frames for up to 3 s (`start.elapsed() < Duration::from_secs(3)`), collects `frames`, then:
  - `elapsed = start.elapsed().as_secs_f64().max(0.1)`
  - `fps = frames.len() as f64 / elapsed`
  - Asserts `6.0 <= fps <= 14.0`; failure message: `"expected some frames, got {}"` (the `fps` value).
- Failure mode: very low fps (e.g. 0.18) → few or no frames in 3 s. Bead: “FPS=0.185 means ~1 frame in 5+ seconds” — **timing-sensitive** / environment-dependent.

**Refactor suggestions:**

- Bead fix: **assert on frame count**, not FPS. For example: fixed 2 s run, then `frames.len() >= floor(6.0 * 2)` (or similar), with explicit rate bounds.
- Consider **controlled time** (`tokio::time::Instant` + `advance`) if available in test harness, to reduce flakiness.
- Link to **bd-2m11.9:** `std::thread::sleep` in async paths can stall the executor and distort timing; audit before blaming only the test.

---

### 2.5 bd-2m11.4 — Frame Observer Timing (P2)

**Test:** `test_channel_based_observer_pattern`  
**File:** `crates/common/tests/frame_observer_timing.rs`

**Evidence:**

- `SlowObserver` **intentionally** uses `std::thread::sleep(Duration::from_millis(1))` in `on_frame` to “simulate blocking work” and violate the <100µs requirement (lines 15–17).
- `test_slow_observer_exceeds_threshold` asserts that `SlowObserver` takes >100µs.
- `test_channel_based_observer_pattern` demonstrates a channel-based offload pattern (lines 88+). Structure implies timing-dependent assertions (exact line range not fully captured in search).

**Refactor suggestions:**

- **bd-2m11.9 interaction:** `std::thread::sleep` in `SlowObserver` is **by design** for that test. Do **not** replace it in `SlowObserver` itself; that would break “slow observer” semantics.
- **Audit** other uses of `std::thread::sleep` in the same file and in frame-delivery paths. Ensure no **unintended** blocking in async/tokio context.
- Bead suggests `#[tokio::test(start_paused = true)]` and explicit time advancement for **timing-sensitive** tests. Evaluate for `test_channel_based_observer_pattern` if it’s flaky.

---

### 2.6 bd-2m11.5 — ELL14 Retry Logic (P2)

**Test:** `test_get_device_info_retries_on_truncated_response`  
**File:** `crates/hardware/src/drivers/ell14.rs` (lines 3306–3341)

**Evidence:**

- Test uses `tokio::io::duplex(256)`, `Ell14Driver::with_test_port(port, "2", 398.2222)`.
- Mock task: on first request, sends **truncated** response `b"2IN0E14002842202115\n"` (16 chars). Test expects `get_device_info` to **retry** and eventually succeed.
- Implementation has a retry loop for “truncated device info response” (ell14.rs ~1722, 1740).
- Possible mismatch: mock sends **only one** truncated response, then exits. If the driver retries by **re-issuing the request**, the mock must respond again. Bead notes: “First attempt: send truncated… This should retry once and succeed on **second** attempt” — mock may need to send a **full** response on the second read.

**Refactor suggestions:**

- Add an **investigation** sub-task: trace retry loop (request/response flow, how “truncated” is detected) and mock behavior (number of reads, what is sent on each). Confirm mock sends truncated then **full** (or valid) response on retry.
- Optionally add `RUST_LOG=debug` (or tracing) to the test for debugging.
- Ensure `test_get_device_info_fails_after_max_retries` (all truncated) still passes; keep both behaviors covered.

---

### 2.7 bd-2m11.9 — `std::thread::sleep` Audit (P1)

**Scope:** `crates/` (Octocode search: **82 matches in 33 files**)

**Evidence (sample):**

- **frame_observer_timing.rs:** `SlowObserver` uses `std::thread::sleep` by design (see 2.5).
- **daq-driver-comedi:** `multi_channel.rs` (lines 340, 371, 380), `streaming.rs` (647), `continuous_streaming` tests, examples.
- **daq-driver-pvcam:** `acquisition.rs` (2682, 3008); comment notes “frame_loop_sequence uses std::thread::sleep + blocking PVCAM FFI.”
- **daq-driver-mock** lib.rs: states mocks use `tokio::time::sleep`, not `std::thread::sleep`.

**Refactor suggestions:**

- **Bead:** Replace `std::thread::sleep` with `tokio::time::sleep(...).await` **only** where code runs in async/tokio context. Exclude:
  - Dedicated “slow” test doubles (e.g. `SlowObserver`).
  - Explicitly blocking/sync code (e.g. PVCAM FFI loop) where migration is a larger change — treat as separate work.
- Add a **bd-2m11.9** sub-task: **catalog** all 82 usages (file:line, sync vs async context, test vs prod). Prioritize:
  1. Async paths used by production (e.g. gRPC, stream handlers).
  2. Tests that affect bd-2m11.4 / bd-2m11.6 (frame observer, gRPC metrics).
- Consider a **dedicated epic** or “tech-debt” epic for “Eradicate blocking sleep from async paths” if the audit reveals broad impact.

---

## 3. Cross-Cutting Themes

| Theme | Affected tasks | Action |
|-------|----------------|--------|
| **Timing / flakiness** | bd-2m11.4, bd-2m11.6 | Prefer count-based or virtual-time tests; document use of `std::thread::sleep` in tests. |
| **`std::thread::sleep` in async** | bd-2m11.9, bd-2m11.4, bd-2m11.6 | Audit → replace in async paths; avoid changing intentional “slow” test observers. |
| **Error visibility** | bd-2m11.3 | Surface Rhai errors in E2E (e.g. `unwrap_or_else`) before deeper fixes. |
| **Feature gating** | bd-2m11.1 | Gate Comedi tests or make `comedi_discover` safe when feature off. |
| **User-facing behavior** | bd-2m11.2 | Gamma fix + persistence check; release note if needed. |

---

## 4. bd-izdj Overlap / Proceed vs Hold

**bd-2m11 is not independent of bd-izdj** (Production Hardening: Reliability & Resilience). Overlap analysis:

| bd-2m11 task | bd-izdj overlap | Verdict |
|--------------|-----------------|---------|
| **2m11.3** E2E script execution | **izdj.7** Script Crash Recovery with Checkpoints — same surface: `scripting`, `ScriptPlanRunner`, RunEngine | **HOLD** |
| **2m11.6** gRPC frame rate metrics test | **izdj.3** Parameter Bounds · **izdj.4** Circuit Breaker · **izdj.17** Rate Limiting · **izdj.21** Stream Resume — same surface: `server` gRPC, `stream_frames` | **HOLD** |
| **2m11.4** Frame observer timing | **izdj.1** Buffer Pool Backpressure · **izdj.2** Tap Channel Overflow — frame pipeline | **HOLD** |
| **2m11.1** Comedi feature flag | None | **PROCEED** |
| **2m11.2** Colorbar gamma | izdj.11 (GUI persistence) only if colorbar persisted; fix is local | **PROCEED** |
| **2m11.5** ELL14 retry test | izdj.6 (graceful shutdown) touches hardware cleanup, not ELL14 retry logic | **PROCEED** |
| **2m11.9** `std::thread::sleep` audit | izdj.9 (tracing) broad; touches many of same crates | **DEFER** |

**Rule:** Proceed only with **bd-2m11.1**, **bd-2m11.2**, and **bd-2m11.5** while izdj is in progress. Defer 2m11.3, 2m11.4, 2m11.6, 2m11.9 until relevant izdj work has landed.

---

## 5. Recommended Bead / Epic Refactors

1. **bd-2m11.3 (E2E):**
   - Add **investigation** sub-tasks: (a) trace engine/bindings/yield flow; (b) compare E2E env vs maitai-eos.
   - Implement diagnosis change (unwrap_or_else) as first step; keep existing bead.

2. **bd-2m11.9 (`std::thread::sleep`):**
   - Add **catalog** sub-task: list all 82 usages with context (sync/async, test/prod).
   - Add **remediation** sub-tasks per area (e.g. comedi, pvcam, tests) with clear “do not touch” list (e.g. `SlowObserver`).

3. **bd-2m11.4 / bd-2m11.6:**
   - In bead descriptions, **explicitly** reference bd-2m11.9 and timing best practices (docs/guides/testing.md).
   - Consider **joint** “timing robustness” sub-epic if multiple tests remain flaky after fixes.

4. **bd-2m11.5 (ELL14):**
   - Add **investigation** sub-task: mock vs retry logic; document expected request/response sequence.

5. **Epic bd-2m11:**
   - Add a **“Research complete”** or **“Octocode findings”** comment pointing to this document.
   - Optionally add a **“Definition of done”** for each child (e.g. “E2E: Rhai errors visible; investigation sub-tasks created”).

---

## 6. References (file:line)

| Item | Location |
|------|----------|
| E2E tests | `crates/rust-daq/tests/e2e_script_execution.rs` |
| ScriptPlanRunner::run | `crates/scripting/src/script_runner.rs` ~147–197 |
| run_script_blocking, with_yield_support | `crates/scripting/src/script_runner.rs` ~361–366; `rhai_engine.rs` |
| Plan / yield bindings | `crates/scripting/src/plan_bindings.rs`, `yield_bindings.rs` |
| Colorbar apply_adjustment | `crates/ui/src/widgets/colorbar.rs` 95–110 |
| Comedi test | `crates/daq-driver-comedi/src/device.rs` 741–748 |
| gRPC rate-limit test | `crates/rust-daq/tests/grpc_integration_test.rs` 291–344 |
| Frame observer timing | `crates/common/tests/frame_observer_timing.rs` (e.g. 15–17, 88+) |
| ELL14 retry test | `crates/hardware/src/drivers/ell14.rs` 3306–3341 |
| std::thread::sleep | 82 matches, 33 files under `crates/` (see bd-2m11.9 audit). |

---

## 7. Next Steps

1. **Review** this document; adjust beads/epic descriptions and add sub-tasks as above.
2. **Prioritize:** P0 (bd-2m11.3 diagnosis) → P1 fixes (Comedi, Colorbar, gRPC, sleep audit) → P2 (observer, ELL14) + warning cleanup.
3. **Re-run** Octocode (or local search) after changes to confirm no new usages of blocking sleep in async paths and that E2E/Comedi/Colorbar/gRPC/ELL14 fixes align with this research.
