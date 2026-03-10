---
status: reviewed
last_reviewed: 2026-03-10
reviewed_by: Hermes Agent
source_issues:
  - bd-ucyu
  - bd-vhr3
superseded_by: GEMINI_SUGGESTIONS/NEW/REVIEWED_CONCLUSIONS.md
---

Reviewed Incident Analysis: Comedi Kernel Freeze on Maitai

Status
- This document is mostly accurate as incident analysis.
- It is not an approved implementation plan.
- Confirmed findings and hypotheses are separated below.

Confirmed Findings
1. Repeated open/use/close access from RPC handlers is the operational trigger pattern
   - Concurrent Comedi access initiated by "Read All"-style traffic correlated with catastrophic Maitai freezes.
   - Treating the PCI device like a stateless web endpoint is the wrong ownership model.

2. Semaphore(1) already mitigates the immediate crash
   - Commit 5baec5e46 serializes Comedi access.
   - This reduces risk now, but it is a stopgap rather than a principled endpoint.

3. There is an independent safety bug in the wrappers
   - Some subsystem implementations bypass ffi_lock by calling device.handle() directly instead of the protected wrapper path.
   - This must be fixed regardless of the final concurrency decision.

4. Kernel hardening is useful but separate
   - nmi_watchdog, soft-lockup panic, hung-task panic, and reboot-on-panic settings are defensible operational safeguards.
   - They belong to the Maitai hardening track, not the core driver-architecture fix.

Working Hypotheses
1. IRQ storm or low-level driver deadlock remains plausible
   - The observed full-system unresponsiveness is consistent with a kernel-space failure mode.
   - However, this remains a hypothesis until captured with post-crash evidence.

2. Per-subdevice concurrency may or may not be safe
   - The NI PCI-MIO-16XE-10 and ni_pcimio stack appear to use device-wide spinlocks.
   - Do not assume AI/AO/DIO parallelism is safe merely because the hardware exposes multiple subdevices.

3. Single-open lifetime ownership is still the likely end state
   - A persistent handle is the right direction.
   - In this codebase that should mean registry-owned lifetime management, not a new singleton manager.

Operational Guidance
- Use this document to guide investigation and test design.
- Do not read this document as approval for actor-thread or per-subdevice parallel execution designs.

Recommended Validation
- Reproduce with controlled GUI and scripted load.
- Monitor dmesg and prior-boot kernel logs.
- Verify that wrapper-level ffi_lock bypasses are removed.
- Re-test after registry migration before making concurrency claims.

Kernel Hardening Checklist
- kernel.nmi_watchdog = 1
- kernel.softlockup_panic = 1
- kernel.hung_task_panic = 1
- kernel.hung_task_timeout_secs = 30
- kernel.panic = 10

Forensic Commands
- journalctl -k -b -1 --no-pager | tail -n 200
- watch -n 1 'cat /proc/interrupts | grep -i comedi'
- ls -lah /var/crash/

Historical Note
- Older drafts mixed sound incident analysis with overconfident architectural prescriptions.
- Those rejected prescriptions are preserved in GEMINI_SUGGESTIONS/OLD/ for traceability only.
