# PR Prioritization Strategy - November 12, 2025

## Executive Summary

Following comprehensive analysis by **Gemini 2.5 Pro** and **Codex**, we've implemented a strategic PR prioritization system aligned with V3 migration goals documented in `GEMINI_ARCHITECTURAL_ANALYSIS_2025-11-11.md`.

**Key Outcomes:**
- **Reduced PR backlog**: 38 → 25 → 15 PRs (61% reduction)
- **10 PRs closed**: 6 obsolete V1-extending + 4 duplicates
- **15 PRs prioritized** with P0/P1/P2 labels
- **2 automation scripts** created for monitoring and pattern detection

## Priority Label System

### P0-critical (3 PRs)
**Core V3 architectural changes that unblock other work**

- **#67**: Implement MaiTai V3 driver
  - Files: `src/instruments_v2/maitai_v3.rs`, `src/core.rs`, `src/instrument/v3_adapter.rs`
  - Implements InstrumentV3 traits for laser control

- **#65**: Implement PVCAM V3 driver
  - Files: `src/instruments_v2/pvcam_v3.rs`, `src/instrument_manager_v3.rs`, `docs/V3_CAMERA_ARCHITECTURE.md`
  - Camera-side counterpart for V3 pipeline

- **#58**: Implement ESP300 V3 driver
  - Files: `src/core_v3.rs`, `src/instrument/esp300_v3.rs`, `src/instrument/mod.rs`
  - Follows MotionController trait pattern

### P1-high (6 PRs)
**V3 instrument migrations and high-priority features**

- **#82**: Add instrument capability discovery
  - Extends `src/core_v3.rs`, `src/core/capabilities.rs`, `src/instrument_manager_v3.rs`
  - V3-aware metadata for discovery layer

- **#101**: Add reusable InstrumentStateMachine
  - Refactors `crates/daq-core/src/state_machine.rs`, `src/instrument_manager_v3.rs`
  - Shared async state machine for V3 drivers

- **#93**: Implement measurement synchronization framework
  - Adds `src/sync/measurement_sync.rs` with `InstrumentManagerV3` hooks
  - Multi-instrument V3 measurement alignment

- **#81**: Refactor Instrument State Machine
  - Continues V3 core modernization from #101
  - Should land in same train to avoid diverging patterns

- **#91**: Implement GUI plugin system
  - Threads plugin metadata through `src/instrument_manager_v3.rs`, `src/parameter.rs`
  - Enables V3 instruments to expose controls without actor glue

- **#95**: Add storage integration tests
  - Extends `src/data/storage.rs`, adds `tests/storage_integration_test.rs`
  - Ensures V3 pipeline persistence doesn't regress

### P2-medium (3 PRs)
**V3-compatible features and bug fixes - quick wins**

- **#89**: Add Elliptec graceful shutdown test
  - Focused addition to `tests/elliptec_integration_tests.rs`
  - Preferred over closed #100 (broader config churn)

- **#61**: Add broadcast channel stress tests
  - Pure test suite in `tests/`
  - Easy merge after rebase, no actor API dependencies

- **#99**: Add Python unicode/quotes metadata test
  - Self-contained in `python/tests/*`
  - Quick merge after trimming accidental `.gitignore` changes

### P3-low (0 PRs currently)
**Non-essential or legacy-compatible changes**

### Unclassified (3 PRs)
**New PRs requiring evaluation**

- **#102**: refactor(daq-29): Extract SCPI/VISA communication patterns
  - Recently appeared, needs Gemini/Codex analysis
  - Likely P1 or P2 depending on V3 alignment

- **#88**: Add Elliptec position validation test
  - Recently appeared, needs classification

- **#79**: Add GUI keyboard shortcuts
  - Touches `src/gui/mod.rs` with actor-driven hotkeys
  - May need reclassification if GUI still V1-coupled

## Closed PRs (10 total)

### V1/Actor Model Extensions (6 PRs)

1. **#84**: Add measurement timestamping improvements
   - **Reason**: Extends actor model timestamp propagation (`src/app_actor.rs`, `src/measurement/instrument_measurement.rs`)
   - **Conflict**: V3 moves timestamps into capability types
   - **Action**: Reimplement for V3 after P0 drivers complete

2. **#54**: Implement transaction system for atomic configuration updates
   - **Reason**: Adds transaction system extending actor protocol (`src/app_actor.rs`, `src/network/server_actor.rs`)
   - **Conflict**: Should use InstrumentV3 capability negotiations instead
   - **Action**: Reimplement using V3 patterns

3. **#64**: Improve error handling in instrument drivers
   - **Reason**: Reinforces V1 error handling across all V1 drivers and V2 adapters
   - **Conflict**: Extends legacy registry instead of migrating to V3 capability traits
   - **Impact**: Would force painful rebases for P0 driver PRs that delete/replace these files

4. **#97**: Fix build errors and tests on main
   - **Reason**: Patches V1 files (`src/core.rs`, `src/instruments_v2/newport_1830c.rs`)
   - **Conflict**: Directly conflicts with V3 Newport work documented in ByteRover
   - **Action**: Fix build errors in V3 context after P0 merges

5. **#68**: feat(python): improve test coverage for python bindings
   - **Reason**: Introduces vendored `pyo3-log` artifacts conflicting with cleaned-up `python/` layout
   - **Issue**: Ignores new InstrumentV3 trait surface
   - **Action**: Recreate Python test coverage after V3 stabilizes

6. **#100**: feat(test): add graceful shutdown test for elliptec
   - **Reason**: Duplicate of #89 with broader config churn (`src/config.rs`, `tests/distributor_metrics_test.rs`)
   - **Preferred**: #89 (focused approach)

### Duplicate/Legacy Tests (4 PRs)

7. **#63**: feat: Add polling rate integration test for Elliptec
   - **Reason**: Shares same branch with #52 creating duplicate CI runs
   - **Issue**: Both extend V1 Elliptec driver tests bound to legacy actor events
   - **Action**: Rewrite tests for V3 after Elliptec V3 port

8. **#52**: feat: Add polling rate integration test for Elliptec
   - **Reason**: Duplicate branch with #63
   - **Action**: Consolidate test work for V3

9. **#57**: Add Elliptec dual-device broadcast test
   - **Reason**: Extends V1 Elliptec driver tests bound to legacy actor events
   - **Action**: Wait for V3 port or convert to InstrumentV3 mocks

10. **#56**: Add integration test for Elliptec GUI
    - **Reason**: V1 storage writer tests, narrow and V1-focused
    - **Action**: Convert to target InstrumentV3 data paths

## Strategic Rationale

### Why This Prioritization?

1. **V3 Migration is Critical Priority**
   - Per Gemini analysis: Actor model (`DaqManagerActor`) is primary performance bottleneck
   - V1/V2/V3 hybrid architecture creates maintenance burden
   - Completing V3 driver migration unlocks broader refactoring

2. **Dependency-Driven Ordering**
   - P0 drivers (#67, #65, #58) all touch `src/instrument_manager_v3.rs`, `src/config.rs`
   - P1 infrastructure (#82, #101, #93, #81, #91, #95) depend on V3 drivers being merged
   - Coordinated rebase prevents repeated conflicts

3. **Avoid Technical Debt**
   - Closing V1-extending PRs prevents reinforcing legacy patterns
   - Reduces merge conflicts for V3 work
   - Forces explicit V3 reimplementation of needed features

4. **Quick Wins for Morale**
   - P2 test PRs can merge quickly after rebases
   - Visible progress while P0/P1 work continues
   - Validates V3 pipeline functionality

## Conflict Patterns & Shared Files

### V3 Core PRs (Rebase Together)
**PRs**: #101, #81, #93, #82, #91, #95
**Shared Files**: `src/instrument_manager_v3.rs`, `src/core_v3.rs`, `src/core/capabilities.rs`
**Strategy**: Treat as one release train to avoid re-implementing capability negotiation differently

### V3 Driver PRs (Merge Order Matters)
**PRs**: #67, #65, #58
**Shared Files**: `src/instrument_manager_v3.rs`, `src/config.rs`
**Strategy**: Schedule coordinated rebase/merge, likely order: ESP300 → PVCAM → MaiTai

## Automation Scripts

### 1. PR Rebase Monitor (`scripts/monitor_pr_rebases.sh`)
**Purpose**: Track Jules agent rebase progress without checking each PR individually

**Usage**:
```bash
./scripts/monitor_pr_rebases.sh
```

**Output**:
- Total open PRs
- PRs ready for review (MERGEABLE)
- PRs still conflicting (awaiting rebase)
- Progress percentage

**Run Frequency**: Daily during rebase phase

### 2. Deprecated Pattern Scanner (`scripts/scan_deprecated_patterns.sh`)
**Purpose**: Identify PRs using legacy actor model patterns

**Patterns Detected**:
- `DaqManagerActor`, `app_actor`, `server_actor`
- `actix`, `Actor`, `Handler`, `Context`, `Addr`
- `InstrumentMeasurement` (V1/V2 measurement type)
- `V2InstrumentAdapter` (temporary adapter pattern)

**Usage**:
```bash
./scripts/scan_deprecated_patterns.sh
```

**Output**:
- List of PRs with deprecated patterns
- Specific patterns found in each PR
- Recommendations for closure or V3 reimplementation

**Run Frequency**: Weekly or when new PRs appear

## Next Steps

### Immediate (Days 1-3)
1. **Monitor rebase progress** using `monitor_pr_rebases.sh`
2. **Apply labels to new PRs** (#102, #88, #79) after analysis
3. **Coordinate P0 driver PRs** (#67, #65, #58) for sequential merge

### Short-term (Week 1)
1. **Merge P0 drivers** in order: ESP300 → PVCAM → MaiTai
2. **Coordinate P1 infrastructure PRs** (#82, #101, #93, #81, #91, #95) for rebase on merged drivers
3. **Quick-win P2 test PRs** (#89, #61, #99) as rebases complete

### Medium-term (Weeks 2-4)
1. **Complete V3 infrastructure merges** (all P1 PRs)
2. **Evaluate and close/reimplement** remaining unclassified PRs
3. **Document V3 migration completion**

### Long-term (Post-V3)
1. **Revisit closed PR requirements**
   - Reimplement #84 (timestamps) for V3 capability types
   - Reimplement #54 (transactions) using V3 negotiations
   - Recreate #68 (Python tests) for InstrumentV3 surface
2. **Port remaining V1 instruments** to V3 (Elliptec, etc.)
3. **Remove actor model** entirely per Gemini recommendation
4. **Complete experiment configuration** system

## Key Metrics

### Before Cleanup
- **Total Open PRs**: 38
- **Merged**: 7
- **No prioritization system**

### After First Pass (Yesterday)
- **Total Open PRs**: 23 (after merging 7 + closing 9)
- **All marked CONFLICTING**
- **Rebase feedback added to all PRs**

### After Strategic Cleanup (Today)
- **Total Open PRs**: 15 (60% reduction from peak)
- **PRs Closed**: 10 (6 obsolete + 4 duplicates)
- **P0-critical**: 3 PRs (V3 drivers)
- **P1-high**: 6 PRs (V3 infrastructure)
- **P2-medium**: 3 PRs (quick wins)
- **Unclassified**: 3 PRs (need analysis)
- **Progress**: 0% ready for review (all awaiting rebase)

### Target State
- **P0 drivers merged**: Week 1
- **P1 infrastructure merged**: Week 2-3
- **P2 quick wins merged**: Week 1-2
- **All 15 PRs resolved**: Week 3-4

## References

- **Gemini Analysis**: `GEMINI_ARCHITECTURAL_ANALYSIS_2025-11-11.md`
- **Codex Analysis**: Conversation continuation ID `70d5c625-078b-4d80-a6da-9c095aa23dcc`
- **Previous Cleanup**: `PR_CLEANUP_REPORT_2025-11-11.md`
- **ByteRover Memory**: V3 Newport implementation, architectural patterns

## Change Log

### 2025-11-12
- Created priority label system (P0/P1/P2/P3)
- Closed 10 obsolete/duplicate PRs
- Applied labels to 12 classified PRs
- Created `monitor_pr_rebases.sh` automation
- Created `scan_deprecated_patterns.sh` automation
- Documented complete strategy and rationale

---

**Generated by**: Claude Code (orchestrator)
**Input from**: Gemini 2.5 Pro (strategic guidance) + Codex (deep code analysis)
**Date**: November 12, 2025
