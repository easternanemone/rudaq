---
status: reviewed
last_reviewed: 2026-03-10
reviewed_by: Hermes Agent
source_issue: bd-ucyu
superseded_by: GEMINI_SUGGESTIONS/NEW/REVIEWED_CONCLUSIONS.md
historical_proposal: GEMINI_SUGGESTIONS/OLD/Hardening Plan Execution and Lifecycle via
references:
  - GEMINI_SUGGESTIONS/NEW/COMEDI_KERNELCRASH.md
  - GEMINI_SUGGESTIONS/NEW/STATEFUL_ARCHITECTURE_PROPOSAL.md
---

Reviewed Refactoring Plan: Registry Consolidation Instead of Actor-System Rewrite

Status
- The original phased plan had a useful execution shape.
- Its target architecture was wrong.
- This document keeps the phase structure and aligns it with the accepted design.

Phase 1: Safety and Wrapper Correctness
Goal
- Remove unsafe wrapper bypasses before changing ownership or concurrency.

Actions
- Audit driver-comedi subsystem implementations.
- Replace direct device.handle() access with ffi_lock-safe paths such as with_handle() where required.
- Add concurrency regression tests that exercise concurrent AI/AO/DIO calls through the current wrappers.

Why first
- If wrappers bypass ffi_lock, later registry migration will still rest on unsafe internals.

Phase 2: Registry Coverage
Goal
- Make the existing DriverFactory and DeviceRegistry path feature-complete for NiDaqService.

Actions
- Add DIO factories.
- Add Counter and status-oriented factories if RPC coverage requires them.
- Confirm capability exposure matches what the service layer needs.

Why second
- Service migration should not invent a parallel abstraction just to fill capability gaps.

Phase 3: NiDaqService Migration
Goal
- Remove open-per-RPC behavior from the service layer.

Actions
- Update NiDaqService handlers to use registry-owned handles exclusively.
- Remove ComediDevice::open calls from RPC scope.
- Keep error mapping aligned with existing gRPC conventions.

Expected result
- One persistent ownership path for Comedi access.
- No repeated VFS churn from RPC handlers.

Phase 4: Stress Testing and Semaphore Decision
Goal
- Determine whether Semaphore(1) can be removed, narrowed, or must remain.

Actions
- Run the WASM GUI "Read All" stress test.
- Run concurrent AI/AO exercise where supported.
- Monitor dmesg during validation.
- If failures persist, keep the semaphore and continue investigating kernel-level limits.

Validation Signals
- No hangs or hard freezes on Maitai.
- No Comedi warnings, tracebacks, or IRQ anomalies in dmesg.
- No direct-open path remains in NiDaqService.
- No known ffi_lock bypass remains in driver-comedi subsystem implementations.

Out of Scope for This Plan
- No HardwareManager file.
- No worker-thread actor system.
- No assumption of safe per-subdevice parallelism without evidence.

Historical Note
- The rejected actor-system implementation details were intentionally not copied forward.
- Historical raw material remains under GEMINI_SUGGESTIONS/OLD/ for traceability only.
