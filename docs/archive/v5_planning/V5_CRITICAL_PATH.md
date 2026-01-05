# V5 Reactive Parameter System - Critical Path

> **⚠️ HISTORICAL REFERENCE - ALL ISSUES RESOLVED (2025-12-07)**
>
> This critical path document tracked the V5 "Split Brain" architecture resolution.
> All P0 and P1 issues are now complete. The reactive Parameter<T> system is fully operational.
> This document is retained for historical reference and architectural decisions.

**Date**: 2025-12-02 (Original), 2025-12-07 (Final Update)
**Analysis**: clink/gemini confirmation of architectural fragmentation
**Status**: ✅ RESOLVED - Split Brain Architecture Successfully Eliminated

> **UPDATE 2025-12-06**: All P0 and P1 issues have been completed. The "Split Brain" architecture
> has been eliminated through successful driver migration to Parameter<T>. This document is
> retained for historical reference and architectural decisions.

## Executive Summary

**RESOLVED**: rust-daq V5 previously had a "Split Brain" architecture where three independent state systems existed without integration. This has been fixed:
- ~~**Brain A (Hardware)**: `Arc<RwLock<T>>` - opaque, silent~~ → **Migrated to Parameter<T>**
- **Brain B (High-Level)**: `Parameter<T>` - rich, reactive, **NOW USED BY ALL DRIVERS**
- **Brain C (Modules)**: `Observable<T>` - lightweight notifications (composed into Parameter<T>)

**Result**: gRPC clients now see hardware changes in real-time, presets can snapshot state, experiments have complete metadata.

## Root Cause (Historical)

Drivers were **never migrated** to use the reactive parameter system. The V5 architecture was built but V4 patterns (raw locks) were never removed.

**Resolution**: All major drivers (ELL14, ESP300, PVCAM, MaiTai, Newport1830C) now use Parameter<T> with async hardware callbacks.

## Critical Path - COMPLETED

### Phase 1: Foundation (P0 - ✅ COMPLETE)

**Epic**: bd-gcjl - V5 Reactive Parameter System Integration

#### 1.1 Unify Reactive Primitives
**Issue**: bd-si19 (P0) - ✅ CLOSED
**Title**: CRITICAL: Unify Parameter and Observable into single reactive primitive

**Resolution**: Parameter<T> now composes Observable<T> internally:
```rust
pub struct Parameter<T> {
    inner: Observable<T>,  // Base primitive (watch, subscriptions, validation)
    hw_writer: Option<Box<dyn Fn(T) -> BoxFuture<'static, Result<()>>>>,
}
```

---

#### 1.2 Add Central Parameter Registry
**Issue**: bd-9clg (P0) - ✅ CLOSED
**Title**: Add Parameterized trait for central parameter registry

**Resolution**: Parameterized trait implemented in 9 files:
```rust
pub trait Parameterized {
    fn parameters(&self) -> &ParameterSet;
}
```

---

### Phase 2: Driver Migration (P0 - ✅ COMPLETE)

#### 2.1 Migrate All Drivers
**Issue**: bd-dili (P0) - ✅ CLOSED
**Title**: Migrate all hardware drivers to unified Parameter system

**Migrated Drivers**:
1. ✅ MockCamera
2. ✅ MockStage
3. ✅ PVCAM
4. ✅ ELL14
5. ✅ ESP300
6. ✅ MaiTai
7. ✅ Newport1830C

---

### Phase 3: Integration (P1 - ✅ COMPLETE)

#### 3.1 gRPC Module Restoration
**Issue**: bd-gmwv (P1) - ✅ CLOSED
**Resolution**: gRPC module compiles with feature-gating. Health service provides real metrics.

#### 3.2 System Metrics Collection
**Issue**: bd-3ti1 (P1) - ✅ CLOSED
**Resolution**: SystemMetricsCollector implemented with sysinfo crate.

#### 3.3 Async I/O
**Issue**: bd-081j (P1) - ✅ CLOSED
**Resolution**: tokio::fs used throughout for async file operations.

---

## Historical Follow-ups (tracked in bd)

All follow-up work was moved to bd; this document is purely historical and carries no open TODOs. Relevant backlog items live in bd (e.g., bd-dqic ring buffer taps, bd-ej44 experiment manifests, bd-pauy health alerts, documentation/tests tasks).

## Dependency Graph (Historical)

```
Epic: bd-gcjl (CLOSED)
├─ bd-si19 (P0) ✅ Unify Parameter/Observable
├─ bd-9clg (P0) ✅ Parameterized trait
├─ bd-dili (P0) ✅ Migrate drivers
├─ bd-gmwv (P1) ✅ gRPC compilation
├─ bd-3ti1 (P1) ✅ System metrics
├─ bd-081j (P1) ✅ Async I/O
└─ [P2 issues: bd-dqic, bd-ej44, bd-pauy - pending]
```

## Success Criteria - ACHIEVED

The V5 architecture is now complete:
- ✅ All driver state is `Parameter<T>`-based (no raw `Arc<RwLock>` in drivers)
- ✅ gRPC services compile and provide health metrics
- ✅ Modules can observe driver parameter changes via `ParameterSet`
- ✅ Parameterized trait implemented across all major drivers
- ✅ Observable<T> composed into Parameter<T>
- ✅ Async callbacks use BoxFuture<'static, Result<()>>

## Remaining Arc<RwLock Usage (Acceptable)

Some internal state management still uses Arc<RwLock>:
- `crates/rust-daq/src/hardware/mock.rs` - Internal frame buffer state (not exposed to Parameter system)
- `crates/rust-daq/src/hardware/registry.rs` - Device registry collection
- `crates/rust-daq/src/scripting/` - Script engine internal state

These are intentional architectural choices, not Split Brain violations.

## Key Lessons Learned

1. **Composition over Inheritance**: Parameter<T> composing Observable<T> worked well
2. **BoxFuture for Callbacks**: Required for async hardware operations
3. **Feature Gating**: Essential for conditional compilation (networking, hardware)
4. **Incremental Migration**: Driver-by-driver migration was manageable

## References

- **V5_ARCHITECTURE.md**: Complete architectural documentation
- **V5_IMPLEMENTATION_STATUS.md**: Current implementation status
- **ADR_005_REACTIVE_PARAMETERS.md**: Design decisions for Parameter<T>
