---
status: reviewed
last_reviewed: 2026-03-10
reviewed_by: Hermes Agent
source_issues:
  - bd-ucyu
  - bd-vhr3
supersedes:
  - GEMINI_SUGGESTIONS/NEW/STATEFUL_ARCHITECTURE_PROPOSAL.md
  - GEMINI_SUGGESTIONS/NEW/REFACTORING_RECS.md
  - GEMINI_SUGGESTIONS/NEW/COMEDI_KERNELCRASH.md
historical_material:
  - GEMINI_SUGGESTIONS/OLD/ANALYSIS.md
  - GEMINI_SUGGESTIONS/OLD/GAP_ANALYSIS_REPORT.md
  - GEMINI_SUGGESTIONS/OLD/Architecture Proposal Hybrid Control
  - GEMINI_SUGGESTIONS/OLD/Hardening Plan Execution and Lifecycle via
  - GEMINI_SUGGESTIONS/OLD/High-Throughput Memory Management in a H
  - GEMINI_SUGGESTIONS/OLD/Refactoring Rhai Scripting boundaries us
---

Reviewed Conclusions: Gemini Suggestions on Comedi and Architecture

Purpose
- This is the canonical, maintained summary of what remains useful from the Gemini suggestion set.
- The files under GEMINI_SUGGESTIONS/OLD/ preserve rejected or superseded proposals for historical traceability only.
- The files under GEMINI_SUGGESTIONS/NEW/ are reviewed summaries, not raw implementation blueprints.

Confirmed Findings
1. Open-per-RPC access is the root operational problem
   - ni_daq_service.rs performs repeated Comedi open/use/close work in RPC handlers.
   - This pattern correlated with Maitai freezes during concurrent "Read All" traffic.
   - Keeping a persistent registry-owned handle is the correct direction.

2. Semaphore(1) is a stopgap, not a final architecture
   - Commit 5baec5e46 serializes Comedi access and prevents the immediate crash.
   - It also suppresses any opportunity for safe concurrency and does not remove the underlying architectural mismatch.

3. The current codebase already has the right ownership model
   - The project already uses DriverFactory plus DeviceRegistry to create and retain hardware handles.
   - Any fix should consolidate Comedi access into that existing path rather than introducing a second manager abstraction.

4. There is a real wrapper-safety bug separate from the kernel-freeze issue
   - ComediDevice has an ffi_lock mutex.
   - Some subsystem implementations bypass it by using device.handle() directly instead of with_handle().
   - This needs to be fixed before making stronger concurrency claims.

5. Validation should stay empirical
   - GUI stress testing, concurrent AI/AO exercise, and dmesg monitoring are still the right validation tools.
   - Kernel hardening on Maitai is worthwhile, but it is a separate workstream from the driver-architecture cleanup.

Rejected Recommendations
1. Do not build a HardwareManager singleton
   - It duplicates existing DriverFactory and DeviceRegistry responsibilities.
   - It would create a third Comedi access path beside the registry path and the direct-open RPC path.

2. Do not treat the actor model as the approved target architecture
   - Per-subdevice worker threads may be a future experiment, but not the accepted design.
   - The ni_pcimio driver appears to use device-wide kernel locking; parallel AI/AO/DIO is not yet proven safe.

3. Do not rely on the code snippets from the raw proposal docs
   - Rejected snippets referenced APIs that do not exist in driver-comedi, including lock_subdevice, data_read_delayed, data_write, cancel, and unlock_subdevice.
   - Those snippets are historical examples of a rejected approach, not a basis for implementation.

Open Experiments
1. Can safe concurrency be recovered after registry consolidation?
   - First remove direct-open access and ffi_lock bypasses.
   - Then test whether the global semaphore can be relaxed safely.

2. Is per-subdevice parallelism actually supported by the kernel driver?
   - Treat this as a hardware experiment, not a design premise.
   - Require stress-test evidence before changing the concurrency model.

Recommended Phases
Phase 1: Safety and correctness
- Fix ffi_lock bypasses in analog_input.rs, analog_output.rs, and digital_io.rs.
- Add concurrency regression tests covering concurrent reads/writes through the existing wrappers.

Phase 2: Registry completeness
- Add DIO, Counter, and status-oriented factories to driver-comedi so the registry path can fully replace direct opens.

Phase 3: Service migration
- Migrate NiDaqService RPC handlers to use registry-owned handles exclusively.
- Remove repeated ComediDevice::open calls from RPC scope.

Phase 4: Hardening and validation
- Re-run Maitai stress tests.
- Decide whether Semaphore(1) remains necessary.
- Keep kernel hardening work tracked separately under bd-vhr3.

Validation Checklist
- WASM GUI "Read All" stress test remains stable.
- dmesg shows no Comedi warnings, resets, trace dumps, or IRQ anomalies.
- Concurrent AI/AO exercise passes without hangs or unexpected EBUSY failures.
- No direct handle() calls remain in subsystem implementations where with_handle() is required.

Document Map
- REVIEWED_CONCLUSIONS.md: canonical reviewed guidance.
- COMEDI_KERNELCRASH.md: reviewed kernel-level incident analysis with confirmed findings vs hypotheses.
- STATEFUL_ARCHITECTURE_PROPOSAL.md: reviewed architecture summary showing what was rejected and what replaced it.
- REFACTORING_RECS.md: reviewed execution plan aligned with the current beads phases.
