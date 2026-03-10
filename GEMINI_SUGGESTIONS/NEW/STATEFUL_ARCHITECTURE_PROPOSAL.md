---
status: reviewed
last_reviewed: 2026-03-10
reviewed_by: Hermes Agent
source_issue: bd-ucyu
superseded_by: GEMINI_SUGGESTIONS/NEW/REVIEWED_CONCLUSIONS.md
historical_proposal: GEMINI_SUGGESTIONS/OLD/Architecture Proposal Hybrid Control
---

Reviewed Architecture Summary: Comedi Access Must Become Stateful, but Within the Existing Registry

Status
- The original actor-model proposal diagnosed the direction of failure correctly: open-per-RPC access is unsafe.
- The proposed solution was rejected.
- This file now summarizes the accepted architectural direction.

Confirmed Diagnosis
- Repeated open/use/close of /dev/comedi0 inside gRPC handlers is the wrong ownership boundary.
- A persistent device handle owned for the process lifetime is the right direction.
- Semaphore(1) prevents the immediate crash but is a stopgap.

Why the Original Proposal Was Rejected
1. It introduced a new HardwareManager singleton even though the codebase already has DriverFactory and DeviceRegistry.
2. It assumed per-subdevice thread-level parallelism is safe before that has been demonstrated on ni_pcimio.
3. It relied on non-existent driver-comedi APIs.
4. It would have created another access path instead of consolidating the existing ones.

Accepted Architectural Direction
1. Preserve one ownership model
   - DriverFactory constructs the device.
   - DeviceRegistry owns the long-lived handle.
   - Services obtain and use registry-owned handles.

2. Eliminate direct-open RPC behavior
   - NiDaqService should stop opening /dev/comedi0 in request handlers.
   - RPCs should route through registry-owned Comedi devices and their typed capabilities.

3. Fix wrapper correctness before concurrency optimization
   - Comedi subsystem implementations must use the ffi_lock-protected access path.
   - Direct device.handle() use in subsystem implementations should be removed where with_handle() is required.

4. Treat concurrency as a validation problem, not an assumption
   - Keep Semaphore(1) until registry migration and wrapper fixes are complete.
   - Only then test whether safe concurrency can be reintroduced.

Non-Goals
- No HardwareManager singleton.
- No actor-thread rewrite at this stage.
- No assumption that AI/AO/DIO can run in parallel just because the subdevices are distinct.

Implementation Implications
- The right long-term architecture is stateful, but stateful through the existing driver/registry stack.
- The work is a consolidation effort, not a greenfield concurrency framework.

See Also
- GEMINI_SUGGESTIONS/NEW/REVIEWED_CONCLUSIONS.md
- GEMINI_SUGGESTIONS/NEW/COMEDI_KERNELCRASH.md
- GEMINI_SUGGESTIONS/NEW/REFACTORING_RECS.md
