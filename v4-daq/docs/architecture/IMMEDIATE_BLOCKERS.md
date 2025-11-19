# Immediate Blockers & Open Questions

## Status: Design Complete, Ready for Implementation

This document captures blockers and open questions identified during the V2/V4 coexistence design phase.

---

## Critical Blockers (Must Resolve Before Phase 1E)

### BLOCKER-1: Kameo Actor Lifecycle Integration

**Issue**: How does Kameo's supervised actor system integrate with DualRuntimeManager's manual coordination?

**Current Understanding:**
- Kameo provides automatic supervision and restart policies
- DualRuntimeManager needs to coordinate shutdown
- Unclear: How to gracefully shutdown Kameo actors from external supervisor?

**Action Items:**
1. [ ] Review Kameo documentation on `ActorRef::shutdown()` or equivalent
2. [ ] Create test: Spawn Kameo actor, shutdown externally with timeout
3. [ ] Verify actors cleanly stop without panics
4. [ ] Document shutdown protocol in design

**Assigned To:** Architecture Review
**Timeline:** This week (before Phase 1E begins)

**Code Reference:**
- Kameo actor examples in `v4-daq/src/actors/`
- Expected integration point: `DualRuntimeManager::shutdown()`

---

### BLOCKER-2: Tokio Runtime & Kameo Runtime Coexistence

**Issue**: Both tokio and Kameo need async runtime. How do they interact?

**Current Understanding:**
- V2 uses `tokio::spawn()` for instruments and storage
- V4 will use Kameo (need to verify if it needs separate runtime or uses tokio)
- Both need event loop, potential conflicts?

**Action Items:**
1. [ ] Investigate Kameo's runtime requirements
2. [ ] Does Kameo use tokio internally or separate runtime?
3. [ ] If separate: How to coordinate with tokio tasks?
4. [ ] Create test with both tokio tasks and Kameo actors
5. [ ] Verify no runtime conflicts

**Assigned To:** Architecture Review
**Timeline:** This week

**Code Reference:**
- Check `v4-daq/src/actors/instrument_manager.rs` for runtime usage
- V2 runtime in `src/app_actor.rs`

---

### BLOCKER-3: Shared Resource Contention Testing

**Issue**: Design assumes Arc<Mutex<>> prevents deadlocks, but needs verification.

**Current Understanding:**
- Theory: Timeout-protected acquisitions can't deadlock
- Practice: Need to test under actual contention

**Action Items:**
1. [ ] Create contention test harness
2. [ ] V2 actor acquiring serial port, then VISA
3. [ ] V4 actor acquiring VISA, then serial port
4. [ ] Run for 1000 iterations, check for deadlocks
5. [ ] Use tools like `loom` for exhaustive testing

**Assigned To:** Phase 1E.1 (DualRuntimeManager task)
**Timeline:** During implementation

**Code Location:** Should be in `tests/contention_tests.rs`

---

### BLOCKER-4: VISA SDK Licensing & Installation

**Issue**: VISA (National Instruments) has licensing complexities. Both V2 and V4 need VISA.

**Current Understanding:**
- NI-VISA has free runtime but requires installation
- Multiple VISA implementations exist (Keysight, Rohde & Schwarz, etc.)
- May need to handle cases where VISA not installed

**Action Items:**
1. [ ] Document VISA installation requirements
2. [ ] Determine: Can both V2 and V4 use same VISA installation?
3. [ ] Create fallback for missing VISA
4. [ ] Feature flag for VISA support?
5. [ ] Document in migration guide

**Assigned To:** Phase 1F (instrument migration)
**Timeline:** Before VISA instruments migrated

---

## Major Open Questions (Design Decisions Needed)

### QUESTION-1: Should V2/V4 Share HDF5 Files?

**Options:**
A) Separate HDF5 files per subsystem (current design)
B) Single merged HDF5 file with V2/V4 groups
C) V4 writes Arrow, V2 continues with its format

**Pros/Cons:**
- Option A: Simple, no conflicts, but duplicates data
- Option B: Cleaner files, but requires shared schema
- Option C: Minimal change, but keeps dual formats

**Recommendation:** Option A (separate files) for Phase 1E-2, revisit in Phase 3

**Decision Needed By:** Architecture Review approval
**Impact:** Affects storage design in Phase 1E.2

---

### QUESTION-2: Should GUI Display Merged V2/V4 Measurements?

**Options:**
A) Unified view - Single GUI shows merged data from both
B) Separate tabs - V2 in one tab, V4 in another
C) Configurable - User chooses at startup

**Pros/Cons:**
- Option A: Best UX, requires bridge, complex
- Option B: Simple, but fragmented view
- Option C: Most flexible, most complex code

**Recommendation:** Option C (configurable) allows staged implementation

**Decision Needed By:** GUI architecture review
**Impact:** Affects MeasurementBridge design (optional)

---

### QUESTION-3: How to Handle Instrument ID Conflicts?

**Scenario:** User configures both V2 and V4 with instrument ID "scpi_meter"

**Options:**
A) Configuration validation prevents duplicates (hard error)
B) Auto-rename: V4 becomes "scpi_meter_v4" (soft error)
C) Last-one-wins: Later config overwrites earlier (silent, bad)

**Recommendation:** Option A (hard error with helpful message)

**Decision Needed By:** Phase 1E.5 (configuration system)
**Impact:** Configuration validation logic

---

### QUESTION-4: Backward Compatibility Requirements?

**Question:** Do we need V2 instruments to work with V4 GUI?

**Options:**
A) Yes - V4 GUI must show V2 measurements
B) No - V2 and V4 use separate GUIs during migration
C) Gradual - V4 GUI gains V2 support in Phase 2

**Recommendation:** Option C (gradual)

**Decision Needed By:** Before Phase 1F GUI work
**Impact:** GUI and bridge architecture

---

## Medium-Priority Unknowns

### UNKNOWN-1: Serial Port Driver Behavior

**Question:** How do serial port drivers handle multiple simultaneous open attempts?

**Impact:** Affects SharedSerialPort design

**Action:**
- [ ] Test with actual hardware (USB serial adapters)
- [ ] Verify: Can one actor hold open while another waits?
- [ ] Document behavior in design

---

### UNKNOWN-2: Performance Overhead of Arc<Mutex<>>

**Question:** What's the actual overhead of serialized resource access?

**Estimates:** Probably < 5% for typical measurement rates

**Action:**
- [ ] Baseline performance test (V2 alone, V4 alone)
- [ ] Dual system performance test
- [ ] Measure lock contention with tools
- [ ] Document in Phase 2 performance report

---

### UNKNOWN-3: Kameo Error Propagation

**Question:** When Kameo actor panics, how to handle gracefully?

**Impact:** Affects supervisor design

**Action:**
- [ ] Review Kameo supervision patterns
- [ ] Test actor panic scenarios
- [ ] Document recovery procedure

---

## Phase 1E Pre-Requisites

Before starting Phase 1E, resolve:
- [ ] BLOCKER-1: Kameo lifecycle integration
- [ ] BLOCKER-2: Runtime coexistence
- [ ] BLOCKER-3: Contention testing strategy
- [ ] BLOCKER-4: VISA SDK requirements

Determine:
- [ ] QUESTION-1: File sharing approach
- [ ] QUESTION-2: GUI architecture
- [ ] QUESTION-3: ID conflict handling
- [ ] QUESTION-4: Backward compatibility

---

## Recommended Approach for Unknowns

### Phase 1E: Make Conservative Choices

1. **Separate everything initially**
   - Separate HDF5 files (V2 and V4)
   - Separate GUIs (V2 and V4)
   - Hard validation errors on conflicts

2. **Add bridges incrementally**
   - MeasurementBridge as "nice to have"
   - Unified GUI as Phase 2 enhancement
   - Shared files as Phase 3 optimization

3. **Test thoroughly**
   - Each choice validated with tests
   - No assumptions, only measurements

### This Approach Avoids:
- Tight coupling during migration
- Complex shared state issues
- Overdesigning for unknowns
- Blocking Phase 1E on decisions

### Phase 2: Optimize Based on Data
- Measure actual overhead
- Get user feedback on separate GUIs
- Implement optimizations where needed
- Keep architecture flexible

---

## Decision Authority & Timeline

| Decision | Authority | By When | Impact |
|----------|-----------|---------|--------|
| Kameo integration | Arch Review | This week | Blocks 1E.1 |
| Runtime coexistence | Arch Review | This week | Blocks 1E.1 |
| HDF5 sharing | PM | Next sprint | 1E.5 |
| GUI architecture | UI Lead | Next sprint | 1F.6 |
| ID conflict handling | PM | Next sprint | 1E.5 |
| Backward compat | PM | Next sprint | 1F |

---

## Risk from Unknowns

**If Kameo integration fails:**
- Cannot build DualRuntimeManager
- Blocks entire Phase 1E
- Contingency: Redesign to use tokio for both

**If runtime conflicts occur:**
- Deadlocks or panics during testing
- Blocks Phase 1E integration tests
- Contingency: Separate processes

**If performance unacceptable:**
- Cannot meet <20% overhead target
- Blocks Phase 2 production readiness
- Contingency: Optimize internals

---

## Mitigation: Early Testing

Recommend **Spike Task (1-2 days):**

"Test Kameo + Tokio coexistence in isolation"

```rust
#[tokio::main]
async fn main() {
    // Create both actor systems
    let v2_task = tokio::spawn(async { /* V2 task */ });
    let kameo_actor = kameo::spawn(/* V4 actor */);

    // Both run in parallel
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Verify no panics, deadlocks, conflicts
    v2_task.abort();
    kameo_actor.cast(Shutdown);

    tokio::time::sleep(Duration::from_millis(100)).await;
}
```

This spike should answer:
- Can both run in same tokio runtime?
- Do they interfere with each other?
- What's the shutdown protocol?

---

## Conclusion

**Current Status:** Design is solid despite unknowns. Conservative choices in Phase 1E allow unknowns to be resolved without redesign.

**Path Forward:**
1. Resolve blockers before Phase 1E start
2. Make conservative architectural choices
3. Test continuously to validate assumptions
4. Phase 2: Optimize based on real data

**Key Principle:** Better to over-design for safety and optimize later, than under-design and hit major issues mid-phase.

---

## Next Steps

**This Week:**
- [ ] Resolve Kameo integration blocker
- [ ] Resolve runtime coexistence blocker
- [ ] Architecture review approval

**Next Week:**
- [ ] Spike task: Kameo + Tokio test
- [ ] Phase 1E task planning and estimation
- [ ] Team capacity planning

**Phase 1E Start:**
- [ ] All blockers resolved
- [ ] All decisions made
- [ ] Ready for implementation

---

**Document Status:** Complete - Awaiting Architecture Review
**Last Updated:** 2025-11-17
**Owner:** System Architecture Designer
