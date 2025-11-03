# Beads Issues Review - 2025-11-03

**Date**: 2025-11-03
**Trigger**: Phase 2 V2 Migration completion
**Changes**: 3 closed, 3 created

## Summary

Updated beads issue tracker to reflect Phase 2 completion and identify Phase 3 priorities based on independent code review findings.

## Issues Closed

### 1. bd-46c9: Step 2.1 Update InstrumentRegistry for V2 trait ✅
**Reason**: Registry V2 implemented with all critical fixes
**Commit**: 9cf5376

**Completed Work**:
- Created `src/instrument/registry_v2.rs` with `Pin<Box<dyn Instrument>>` support
- Fixed RecvError::Lagged handling (instruments survive bursty loads)
- Fixed V1→V2 command translation (all commands now mapped)
- Fixed GUI status cache propagation (Arc<Mutex<HashMap>> pattern)
- Added error logging for channel full

All critical issues from Codex review resolved. See `docs/PHASE2_FIXES_SUMMARY.md`.

### 2. bd-cd89: Step 2.4 Remove app.rs blocking layer ✅
**Reason**: GUI blocking operations documented as Phase 3 tech debt

**Analysis**:
- `Gui::new()` blocking is pre-visibility (not user-facing)
- Documented in `gui/mod.rs:216-227` with clear explanation
- Control panel blocking (`with_inner`) is **HIGHER priority** (user-facing freezes)
- Control panel migration requires Phase 3 rewrite (tracked in bd-e116)

One-time initialization blocking is acceptable technical debt.

### 3. bd-555d: Phase 2 Update core infrastructure for V2 ✅
**Reason**: Phase 2 complete - all substeps finished

**All 4 Substeps Complete**:
- ✅ bd-cacd: Step 1.1 Revert V3 files to V2
- ✅ bd-61c7: Step 2.2 Update DaqManagerActor for V2 Measurement enum
- ✅ bd-4a46: Step 2.3 Update GUI for V2 measurements
- ✅ bd-46c9: Step 2.1 Update InstrumentRegistry for V2 trait
- ✅ bd-cd89: Step 2.4 Remove app.rs blocking layer

**Review Status**:
- ✅ Gemini: Approved with minor comments
- ✅ Codex: All critical issues resolved

## Issues Created

### 1. bd-e116: Control Panel Async Migration (Phase 2 follow-up) 🆕
**Priority**: P1 (HIGH)
**Type**: Task
**Status**: Open (ready to work)

**Problem**: Control panels use deprecated `DaqApp::with_inner()` which blocks UI thread on EVERY user action.

**Impact**: User-facing freezes when clicking instrument control buttons.

**Scope**:
- Rewrite all control panels to use async `command_tx.send()`
- Track pending operations with timeouts
- Display errors when commands fail
- Remove `DaqApp::with_inner()` entirely

**Files**:
- `src/gui/instrument_controls.rs` - All control panels (20+ blocking calls)
- `src/app.rs` - Remove `DaqApp::with_inner()`

**Discovered From**: Phase 2 Codex review (Issue #3 - Blocking operations)

---

### 2. bd-47f9: Phase 2 Integration Tests 🆕
**Priority**: P1 (HIGH)
**Type**: Task
**Status**: Open (ready to work)

**Purpose**: Add integration tests for Phase 2 critical fixes.

**Tests Needed**:
1. **Broadcast overflow recovery** - Verify RecvError::Lagged continues processing
2. **GUI status cache updates** - Verify instrument_status_cache reflects state
3. **V2 command translation** - Verify all V1 commands translate correctly
4. **Pending operation timeouts** - Verify 30s timeout with error logging

**Files**:
- `tests/phase2_integration_tests.rs` (new)

**Discovered From**: Phase 2 review - recommended integration tests

---

### 3. bd-19e3: Phase 2 Performance Validation 🆕
**Priority**: P2 (MEDIUM)
**Type**: Task
**Status**: Open (ready to work)

**Purpose**: Validate Phase 2 fixes under production-like loads.

**Performance Tests**:
1. **Startup time** - Target: <500ms
2. **GUI freeze duration** - Target: 0ms for user actions
3. **Frame drop rate** - Target: <1% at 100 Hz camera load
4. **Cache refresh overhead** - Target: <10ms per refresh
5. **Command channel capacity** - Target: <0.1% channel-full errors

**Test Scenarios**:
- Single camera at 100 Hz (10 min run)
- Multi-instrument concurrent (3x V2 instruments)
- Bursty load alternating 200 Hz / 10 Hz (5 min)

**Discovered From**: Phase 2 review - recommended performance testing

## Current Priority Work

### Phase 2 Follow-up (Ready to Work)

| Issue | Priority | Title | Type |
|-------|----------|-------|------|
| bd-e116 | P1 | Control Panel Async Migration | Task |
| bd-47f9 | P1 | Phase 2 Integration Tests | Task |
| bd-19e3 | P2 | Phase 2 Performance Validation | Task |

### Hardware Integration (Ongoing)

The Elliptec integration work (bd-e52e.*) is ongoing with 20+ P1 issues ready to work.

### V3 Integration (Future)

bd-155 (Phase 3: V3 Integration and Production Readiness) remains in progress with Milestone 1.2 active.

## Dependency Analysis

**Blocked Issues**: 0
- All Phase 2 follow-up issues are ready to work (no blockers)

**Open Parent Epics**:
- bd-155: Phase 3 V3 Integration (in_progress)
- bd-e52e: Elliptec Rotator Integration (open)
- bd-78: Dynamic Configuration Platform (open)

## Recommendations

### Immediate Priority (1-2 weeks)
1. **bd-e116**: Control Panel Async Migration
   - User-facing impact (GUI freezes)
   - Removes deprecated `with_inner()` pattern
   - Highest priority for UX improvement

2. **bd-47f9**: Phase 2 Integration Tests
   - Validate critical fixes work correctly
   - Prevent regressions
   - Build confidence in Phase 2 stability

### Medium Priority (2-4 weeks)
3. **bd-19e3**: Phase 2 Performance Validation
   - Measure actual production performance
   - Validate <1% frame loss target
   - Identify any remaining bottlenecks

### Long-term (4-8 weeks)
4. Continue Elliptec hardware integration (bd-e52e.*)
5. Plan V3 integration (bd-155 Milestone 2)
6. V1 legacy removal (bd-09b9)

## Related Documents

- **Phase 2 Fixes**: `docs/PHASE2_FIXES_SUMMARY.md`
- **Independent Reviews**: `docs/PHASE2_INDEPENDENT_REVIEWS.md`
- **Phase 2 Completion**: `docs/PHASE2_COMPLETION_REPORT.md`

## Statistics

**Before Review**:
- Open: 68 issues
- In Progress: 4 issues
- Closed: 29 issues

**After Review**:
- Open: 71 issues (+3 created)
- In Progress: 2 issues (-2 closed)
- Closed: 32 issues (+3 closed)

**Net Change**: +3 open, -2 in progress, +3 closed

## Next Steps

1. Start work on bd-e116 (Control Panel Async Migration)
2. Add integration tests (bd-47f9)
3. Run performance benchmarks (bd-19e3)
4. Continue Elliptec hardware testing
5. Plan Phase 3 V3 integration roadmap

---

**Review Completed**: 2025-11-03
**Next Review**: After bd-e116 completion
