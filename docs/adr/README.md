# Architecture Decision Records (ADRs)

This directory contains Architecture Decision Records for the rust-daq project. ADRs document significant architectural decisions along with their context, rationale, and consequences. They serve as a historical log so that future contributors can understand *why* the system is built the way it is.

For background on ADRs, see [Michael Nygard's original article](https://cognitopia.com/blog/architecture/architecture-decision-records).

## Index

| Number | Title | Status | Date | Summary |
|--------|-------|--------|------|---------|
| 001 | [Capability System Consolidation](001-capability-consolidation.md) | Proposed | 2026-01-27 | Consolidate capability definitions spread across 6 files and 4 layers into a single source of truth. |
| 002 | [GUI Connection Reliability](002-connection-reliability.md) | Accepted | 2024-12-24 | Multi-layered connection reliability system to handle drops, daemon restarts, and zombie connections. |
| 003 | [gRPC Validation Layer](003-grpc-validation-layer.md) | Proposed | 2025-12-22 | Defense-in-depth input validation at the gRPC edge to catch malformed or dangerous data early. |
| 004 | [Panic Safety Architecture](004-panic-safety.md) | Accepted | 2026-02-02 | Guarantee safe hardware shutdown on process panics, even without the async runtime. |
| 005 | [Buffer Pool Error Handling](005-pool-error-handling.md) | Accepted | 2026-01-17 | Graceful degradation strategy for zero-allocation pool exhaustion in the PVCAM frame path. |
| 006 | [Buffer Pool Migration Rollback](006-pool-migration-rollback.md) | Accepted | 2026-01-17 | Rollback plan and validation checkpoints for the zero-allocation frame pool migration. |
| 007 | [PVCAM 85-Frame Stall Fix](007-pvcam-85-frame-stall-fix.md) | Implemented | 2026-01-11 | Root-cause fix for continuous streaming stalls after ~85 frames by matching SDK callback patterns. |
| 008 | [PVCAM Continuous Acquisition](008-pvcam-continuous-acquisition.md) | Accepted | 2025-01-09 | Use CIRC_NO_OVERWRITE with `pl_exp_get_latest_frame_ex()` for Prime BSI continuous acquisition. |
| 009 | [PVCAM Driver Architecture](009-pvcam-driver-architecture.md) | Accepted | 2025-01-09 | Justification for the multi-layer PVCAM driver architecture (~9K LOC) for production scientific use. |
| 010 | [PVCAM Pool Migration Results](010-pvcam-pool-migration-results.md) | Migration Complete | 2026-01-17 | Results from eliminating ~1.6 GB/s allocation churn via zero-allocation frame pools. |
| 011 | [PVCAM SDK Pattern Compliance](011-pvcam-sdk-pattern-compliance.md) | In Progress | 2025-01-10 | Add SDK-mandated `ATTR_AVAIL` parameter discovery checks throughout the driver. |
| — | [PVCAM Performance Gap Analysis](analysis-pvcam-performance-gap.md) | Analysis Complete | 2026-01-11 | Root-cause analysis of the 10x FPS gap (4.4 vs 50) traced to per-frame heap allocation. |

## Creating a New ADR

Use the next available number and follow this template:

```markdown
# ADR: <Title>

**Status:** Proposed | Accepted | Implemented | Deprecated | Superseded
**Date:** YYYY-MM-DD
**Author:** <name or team>
**Related Issues:** <bead IDs, GitHub issues, etc.>

---

## Context

What is the issue that we are seeing that motivates this decision or change?

## Decision

What is the change that we are proposing and/or doing?

## Consequences

What becomes easier or harder as a result of this change?

### Positive

- ...

### Negative

- ...

### Risks

- ...
```

### Status Lifecycle

```
Proposed --> Accepted --> Implemented
                \--> Deprecated
                \--> Superseded by ADR-NNN
```

- **Proposed** — Under discussion, not yet agreed upon.
- **Accepted** — Agreed upon, implementation may or may not have started.
- **Implemented** — Fully implemented and verified.
- **Deprecated** — No longer relevant or actively discouraged.
- **Superseded** — Replaced by a newer ADR (link to it).
