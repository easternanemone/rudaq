# Phase 1 Review Findings

**Date:** 2025-12-21
**Reviewers:** Codex (code quality), Gemini (architecture)

## Summary

Phase 1 implementation is **functionally complete** but requires fixes for 4 issues before proceeding to Phase 2.

## High Priority (P1) - Must Fix

### bd-jin1: Fix signal_plotter_stream async integration pattern
**Source:** Gemini
**File:** `crates/daq-egui/src/panels/signal_plotter_stream.rs`

The proposed async integration violates Rust ownership rules - cannot mutably borrow `self` in background Tokio task.

**Fix:** Use message-passing pattern:
1. Create `mpsc::channel()` pair
2. Store `Receiver` in `SignalPlotterPanel`
3. Pass `Sender` to async Tokio task
4. In panel's `update()`, drain: `while let Ok(msg) = self.rx.try_recv() { ... }`

### bd-ijre: Separate StreamObservables from StreamParameterChanges
**Source:** Gemini
**File:** `crates/daq-server/src/grpc/hardware_service.rs`

`HardwareServiceImpl::new` monitors `Observable<T>` in `StreamParameterChanges` path, causing:
- Double traffic for rapidly changing observables
- Inefficient string serialization for plotting data

**Fix:**
- Remove `Observable<T>` monitoring from `StreamParameterChanges`
- Implement `StreamObservables` as dedicated high-throughput numeric stream
- Use `sample_rate_hz` for server-side downsampling

## Medium Priority (P2) - Should Fix

### bd-ju6n: Add timeout and concurrency bounds to async device refresh
**Source:** Codex
**File:** `crates/daq-egui/src/panels/instrument_manager.rs:232-259`

`refresh_device_states` spawns unbounded async tasks with no timeout. A hung device can stall auto-refresh.

**Fix:**
- Add `Semaphore` + `FuturesUnordered` for concurrency bounds
- Add per-call timeouts
- Ensure `action_in_flight` decrements on timeout/cancel

### bd-le6k: Move DeviceCategory to driver/registry layer
**Source:** Gemini
**File:** `crates/daq-server/src/grpc/hardware_service.rs`

`infer_device_category` uses brittle string matching, violates Open-Closed Principle.

**Fix:**
- Add `DeviceCategory` field to `DeviceMetadata` or `Driver` trait
- Hardware service reads field, falls back to inference only if missing

## Low Priority - Nice to Have

| Issue | Source | Description |
|-------|--------|-------------|
| Per-frame device list cloning | Codex | `main_rerun.rs:267` - iterate by index instead |
| Device state cache never prunes | Codex | `instrument_manager.rs` - retain only current IDs |
| Stream stop error details dropped | Codex | `main_rerun.rs:435-438` - use `error_message` |
| Plot points re-allocated each frame | Codex | `signal_plotter.rs:183-186` - reuse buffer |
| DeviceCategory enum forces single choice | Gemini | Consider `repeated` field or capabilities |

## Positives Noted

- **Codex:** Good async/result-channel pattern, bounded signal history, clean dock-state persistence
- **Gemini:** Proto schema cleanly separates concerns, `StreamParameterChanges` handles multi-client sync correctly

## Recommendation

Address P1 issues (bd-jin1, bd-ijre) before starting Phase 2 implementation, as they affect the core streaming architecture.
