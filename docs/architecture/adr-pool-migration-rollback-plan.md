# ADR: BufferPool Migration Rollback Plan

**Status:** Accepted
**Date:** 2026-01-17
**Author:** Architecture Review
**Related Issues:** bd-0dax (epic), bd-0dax.9

---

## Context

The `daq-pool` crate introduces a zero-allocation frame handling architecture to eliminate per-frame heap allocations (~8MB per frame at 100 FPS). This is a significant change to the critical PVCAM frame acquisition path.

If the new pool-based implementation exhibits unforeseen issues (performance regressions, subtle bugs, timing problems, memory leaks), we need a clear, tested rollback path back to the working state.

This document defines the rollback strategy, validation checkpoints, and execution procedures.

---

## Decision

Implement a **multi-layered rollback strategy** with:
1. Environment variable feature toggle for runtime switching
2. Preserved original code path in fallback branch
3. Clear validation checkpoints with pass/fail criteria
4. Documented rollback execution procedures

---

## Architecture Overview

### Current Implementation (Post-Migration)

```
┌─────────────────────────────────────────────────────────────────┐
│                    BufferPool Path (NEW)                        │
│  BufferPool::try_acquire() → PooledBuffer → freeze() → Bytes   │
│  - Zero allocation after warmup                                  │
│  - Buffer auto-returns to pool when Bytes dropped                │
│  - Falls back to heap allocation on pool exhaustion              │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Frame::from_bytes(pixel_data)                 │
│                              │                                   │
│                              ▼                                   │
│                    Broadcast to consumers                        │
└─────────────────────────────────────────────────────────────────┘
```

### Previous Implementation (Pre-Migration)

```
┌─────────────────────────────────────────────────────────────────┐
│                 Direct Allocation Path (OLD)                     │
│  std::slice::from_raw_parts(sdk_ptr) → Bytes::copy_from_slice() │
│  - Per-frame heap allocation (~8MB each)                         │
│  - Simple, proven, but causes GC pressure at high FPS           │
└─────────────────────────────────────────────────────────────────┘
```

---

## Rollback Strategy

### 1. Runtime Feature Toggle (Immediate Rollback)

The pool implementation already includes a fallback path when the pool is exhausted. This path can be forced via environment variable:

```rust
// In acquisition.rs frame loop
let use_pool = std::env::var("PVCAM_USE_POOL")
    .map(|v| v != "0")
    .unwrap_or(true);  // Default: pool enabled

let (pixel_data, used_pool): (Bytes, bool) = if use_pool {
    match buffer_pool.try_acquire() {
        Some(mut buffer) => {
            unsafe { buffer.copy_from_ptr(frame_ptr as *const u8, copy_bytes); }
            (buffer.freeze(), true)
        }
        None => {
            // Pool exhausted - fall back to heap
            let data = unsafe {
                Bytes::copy_from_slice(
                    std::slice::from_raw_parts(frame_ptr as *const u8, copy_bytes)
                )
            };
            (data, false)
        }
    }
} else {
    // Pool disabled - always use heap allocation
    let data = unsafe {
        Bytes::copy_from_slice(
            std::slice::from_raw_parts(frame_ptr as *const u8, copy_bytes)
        )
    };
    (data, false)
};
```

**Rollback Execution:**
```bash
# On maitai or any PVCAM machine
export PVCAM_USE_POOL=0
./target/release/rust-daq-daemon daemon --port 50051 --hardware-config config/maitai_hardware.toml
```

**Verification:**
- Log should show `pool_hit_rate_pct = 0` in `pvcam_alloc_trace` target
- All frames should report `used_pool = false`

---

### 2. Git Revert Strategy (Code Rollback)

If the environment variable toggle is insufficient (e.g., pool initialization itself causes issues):

**Branch Structure:**
```
main
  └── feat/pvcam-pool-migration    ← Pool implementation
        └── feat/pool-rollback     ← Rollback branch (if needed)
```

**Revert Procedure:**

```bash
# Option A: Revert specific commits
git revert <pool-migration-commit-hash>

# Option B: Create rollback branch from pre-migration state
git checkout -b feat/pool-rollback <pre-migration-commit>
# Cherry-pick any bug fixes from migration branch that don't involve pool
```

**Key Files to Revert:**
- `crates/daq-driver-pvcam/src/components/acquisition.rs` (frame loop changes)
- `crates/daq-driver-pvcam/src/components/frame_pool.rs` (can remove entirely)
- `crates/daq-driver-pvcam/Cargo.toml` (remove `daq-pool` dependency)

**Files to KEEP (do not revert):**
- `crates/daq-pool/` (keep crate for future use, just don't depend on it)
- Test files (can be marked `#[ignore]` if pool not available)
- Documentation and ADRs

---

### 3. Compile-Time Feature Flag (Future Option)

If rollback situations become frequent, consider adding a compile-time feature:

```toml
# In crates/daq-driver-pvcam/Cargo.toml
[features]
buffer_pool = ["dep:daq-pool"]  # Default off initially
```

```rust
// In acquisition.rs
#[cfg(feature = "buffer_pool")]
let pixel_data = acquire_with_pool(&buffer_pool, frame_ptr, copy_bytes);

#[cfg(not(feature = "buffer_pool"))]
let pixel_data = unsafe {
    Bytes::copy_from_slice(std::slice::from_raw_parts(frame_ptr as *const u8, copy_bytes))
};
```

**Note:** This is a future enhancement if runtime toggle proves insufficient.

---

## Validation Checkpoints

### Checkpoint 1: Unit Tests (Local)

**Pass Criteria:** 100% unit tests pass
**Rollback Trigger:** Any test failure

```bash
cargo nextest run -p daq-pool
cargo nextest run -p daq-driver-pvcam --features mock
```

### Checkpoint 2: Mock Integration Tests (Local)

**Pass Criteria:** 100% integration tests pass with mock camera
**Rollback Trigger:** Any test failure

```bash
cargo nextest run --workspace --features mock
```

### Checkpoint 3: Hardware Smoke Test (maitai)

**Pass Criteria:**
- 200 frames acquired successfully at 100 FPS
- Pool hit rate > 95%
- No frame drops detected

**Rollback Trigger:**
- Pool hit rate < 90%
- Frame drop rate > 1%
- Any panic or crash

```bash
# On maitai
source scripts/env-check.sh
cargo nextest run --profile hardware --features hardware_tests -p daq-driver-pvcam \
  -- --test-threads=1 --nocapture
```

### Checkpoint 4: Extended Soak Test (maitai)

**Pass Criteria:**
- 24-hour continuous acquisition
- Stable memory usage (no growth > 10%)
- Pool hit rate stable > 99%
- No frame discontinuities

**Rollback Trigger:**
- Memory growth > 20% over baseline
- Pool hit rate degradation
- Any frame loss after warmup

**Monitoring Commands:**
```bash
# Memory monitoring
watch -n 60 'ps -o pid,rss,vsz,comm -p $(pgrep rust-daq-daemon)'

# Log monitoring
tail -f /var/log/rust-daq/daemon.log | grep -E "(pool_hit_rate|frame_loss|WARN|ERROR)"
```

### Checkpoint 5: Production Pilot

**Pass Criteria:**
- No regression in experiment workflows
- User-reported frame rate matches pre-migration
- No unexpected GUI lag or disconnects

**Rollback Trigger:**
- Any user-reported regression
- Frame rate degradation > 5%
- GUI responsiveness issues

---

## Symptoms Requiring Rollback

### Immediate Rollback (PVCAM_USE_POOL=0)

| Symptom | Detection Method | Likely Cause |
|---------|------------------|--------------|
| Pool exhaustion warnings | Log: `Buffer pool exhausted` | Consumer too slow |
| High allocation latency | Log: `avg_alloc_us > 1000` | Pool contention |
| Frame drops resuming | Log: `frame_loss > 0` after warmup | Timing issue |

### Code Rollback Required

| Symptom | Detection Method | Likely Cause |
|---------|------------------|--------------|
| Pool initialization crash | Daemon fails to start | Memory allocation failure |
| Semaphore deadlock | Daemon hangs at start | Pool implementation bug |
| Memory leak | RSS grows unbounded | Buffer not returning to pool |

---

## Rollback Execution Checklist

### Immediate Rollback (Runtime)

- [ ] Set `PVCAM_USE_POOL=0` in environment
- [ ] Restart daemon
- [ ] Verify logs show `used_pool = false`
- [ ] Confirm frame acquisition resumes normally
- [ ] File issue with symptoms and logs

### Code Rollback

- [ ] Stop daemon
- [ ] `git checkout feat/pool-rollback` (or revert commits)
- [ ] `source scripts/build-maitai.sh`
- [ ] Restart daemon
- [ ] Verify frame acquisition works
- [ ] Run smoke test to confirm stability
- [ ] Document rollback reason in beads issue

---

## What NOT to Delete During Migration

Until validation complete at all checkpoints, preserve:

- [ ] Original allocation path code (in fallback branch)
- [ ] Pre-migration test baselines
- [ ] Performance benchmarks from pre-migration state
- [ ] Documentation of pre-migration behavior

---

## Post-Rollback Analysis

If rollback is executed:

1. **Capture Diagnostics:**
   - Full daemon logs from issue occurrence
   - Memory profiles (`heaptrack` or similar)
   - Frame timing histograms

2. **Root Cause Analysis:**
   - Compare pool metrics to expected values
   - Review SDK buffer timing assumptions
   - Check for edge cases in frame size/ROI

3. **Document in ADR:**
   - What failed
   - Why it failed
   - What changes are needed before retry

---

## References

- [ADR: PVCAM Driver Architecture](adr-pvcam-driver-architecture.md)
- [ADR: PVCAM Continuous Acquisition](adr-pvcam-continuous-acquisition.md)
- [daq-pool crate documentation](../../crates/daq-pool/src/lib.rs)
- [bd-0dax epic issue](../.beads/issues/bd-0dax.md)

---

## Revision History

| Date | Author | Description |
|------|--------|-------------|
| 2026-01-17 | bd-0dax.9 | Initial rollback plan |
