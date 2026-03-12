> [!WARNING] **ARCHIVAL / HISTORICAL**
> This document is a historical snapshot of an investigation and is preserved for context. It does not represent current operational guidance or source-of-truth architecture.

# PVCAM Performance Gap Analysis

**Date:** 2026-01-11
**Related Issue:** bd-u1kx
**Status:** Analysis Complete

## Executive Summary

The Rust PVCAM driver achieves ~4.4 FPS compared to C++'s ~50 FPS (a 10x performance gap). After thorough code analysis, the **primary bottleneck** is per-frame heap allocation in the frame processing loop. The code is memory-safe and thread-safe, but violates the SDK's zero-copy design pattern.

## Test Results

| Implementation | Frames | FPS | Result |
|----------------|--------|-----|--------|
| C++ stall_test | 200 | ~50 | No stall |
| Rust frame loop | 98 iterations, 44 frames | 4.4 | No stall (but low FPS) |

## Side-by-Side Pattern Comparison

### C++ SDK Pattern (stall_test.cpp + Common.cpp)

**Callback (minimal operations):**
```cpp
void PV_DECL StallTestEofHandler(FRAME_INFO* pFrameInfo, void* pContext) {
    ctx->eofCounter++;
    ctx->eofFrameInfo = *pFrameInfo;  // Copy struct (40 bytes)
    pl_exp_get_latest_frame(ctx->hcam, &ctx->eofFrame);  // Clear buffer
    std::lock_guard<std::mutex> lock(ctx->eofEvent.mutex);
    ctx->eofEvent.flag = true;
    ctx->eofEvent.cond.notify_all();
}
```

**Main Loop (minimal processing):**
```cpp
while (framesAcquired < 200) {
    WaitForEofEvent(ctx, 2000, errorOccurred);  // Wait on condvar
    std::cout << "Frame #" << ctx->eofFrameInfo.FrameNr << std::endl;
    framesAcquired++;
}
```

**Key characteristics:**
- No memory allocation in hot path
- No frame data copy (uses SDK buffer directly)
- Simple condvar wait: `wait_for(lock, timeout, predicate)`
- Predicate checks single flag: `ctx->eofEvent.flag`

### Rust Implementation (acquisition.rs)

**Callback (lines 382-431):**
```rust
pub unsafe extern "system" fn pvcam_eof_callback(...) {
    let _ = std::panic::catch_unwind(|| {
        // Store frame info via atomic operations
        ctx.store_frame_info(info);

        // Conditional SDK call based on mode
        if ctx.circ_overwrite.load(Ordering::Acquire) {
            pl_exp_get_latest_frame(hcam, &mut frame_ptr);
            ctx.store_frame_ptr(frame_ptr);
        }

        // Signal: atomic increment + mutex + condvar
        ctx.signal_frame_ready(frame_nr);
    });
}
```

**Frame Loop (lines 2516-2798) - Per Frame:**
1. Check shutdown/streaming (2 atomic loads)
2. `pl_exp_get_oldest_frame_ex` FFI call
3. Frame loss detection (atomic load + compare + potential log)
4. Duplicate detection (compare + potential break + log)
5. Update last_hw_frame_nr (atomic store)
6. **Metadata decoding** (`pl_md_frame_decode` FFI call if enabled)
7. **CRITICAL: Copy frame data** `sdk_bytes.to_vec()` - HEAP ALLOCATION
8. Zero-frame detection (4 memory reads from copied buffer)
9. Release frame to SDK (`pl_exp_unlock_oldest_frame`)
10. Decrement pending counter (`consume_one()`)
11. Increment frame_count (atomic)
12. Create `Frame` struct with metadata
13. **Arc allocation:** `Arc::new(frame)` - HEAP ALLOCATION
14. Check receiver count
15. Send to broadcast channel
16. try_send to reliable channel
17. Optional: Arrow tap send
18. Optional: Metadata channel send

## Performance Bottlenecks (Ranked by Impact)

### 1. Per-Frame Heap Allocation (CRITICAL)

**Location:** `acquisition.rs:2649`
```rust
let pixel_data = sdk_bytes.to_vec();
```

**Impact:** For a 2048x2048 @ 16-bit camera:
- Frame size: 8,388,608 bytes (~8MB)
- At 50 FPS: 419 MB/second of allocations
- Each allocation involves: malloc, memcpy, possible memory fragmentation

**SDK design:** PVCAM uses a circular buffer in user-allocated memory. The SDK returns pointers INTO this buffer. The C++ code reads directly from the buffer without copying.

**Why Rust copies:** The broadcast channel requires `Arc<Frame>` which owns its data. Sending a reference into SDK memory would be unsafe once the frame is released.

### 2. Arc Allocation Per Frame

**Location:** `acquisition.rs:2721`
```rust
let frame_arc = Arc::new(frame);
```

**Impact:** Additional heap allocation per frame (small but adds up).

### 3. Complex Validation Logic

**Locations:**
- Duplicate detection: lines 2556-2592
- Frame loss detection: lines 2542-2555
- Zero-frame detection: lines 2651-2681

**Impact:** Adds branches and potential logging in hot path. C++ test has none of this.

### 4. Broadcast Channel Overhead

**Location:** `acquisition.rs:2764`
```rust
let _ = frame_tx.send(frame_arc.clone());
```

**Impact:** `tokio::sync::broadcast` has more overhead than simple condvar signaling. Also clones Arc on send.

### 5. Complex Wait Predicate

**Location:** `acquisition.rs:232-237`
```rust
.wait_timeout_while(guard, timeout_duration, |notified| {
    !*notified
        && self.pending_frames.load(Ordering::Acquire) == 0
        && !self.shutdown.load(Ordering::Acquire)
})
```

**Impact:** Checks 3 conditions vs C++'s single flag. May cause spurious wakeups.

### 6. Logging in Hot Path

Multiple `tracing::` calls that may have overhead even when level is disabled.

## Memory Safety Analysis

The FFI code is **memory-safe**. Key protections:

| Check | Location | Status |
|-------|----------|--------|
| Null pointer validation | Line 388-390 | OK |
| panic::catch_unwind | Line 387 | OK - prevents UB on panic |
| FRAME_INFO copy | Line 397 | OK - copies struct, not pointer |
| Atomic ordering | Throughout | OK - Acquire/Release pairs |
| Mutex in callback | Line 146 | OK - uses std::sync::Mutex, not tokio |

**No memory safety issues found.**

## Thread Safety Analysis

The code is **thread-safe**. Key synchronization:

| Mechanism | Purpose | Status |
|-----------|---------|--------|
| AtomicU32 pending_frames | Frame counter | OK - fetch_add/fetch_update |
| AtomicPtr frame_ptr | Lock-free pointer exchange | OK |
| AtomicBool circ_overwrite | Mode flag | OK - single writer |
| Condvar + Mutex | Wait notification | OK - proper pairing |
| AtomicBool shutdown | Graceful shutdown | OK - Release/Acquire |

**Potential concern:** The callback locks a mutex (`frame_info: std::sync::Mutex`). While brief, mutex locks in callbacks can introduce jitter. However, this is a copy of the struct, not a blocking wait.

**No thread safety issues found.**

## Root Cause Summary

The 10x performance gap is primarily caused by:

1. **Per-frame memory allocation** (`to_vec()`) - dominates at ~8MB per frame
2. **SDK pattern mismatch** - Rust copies data, C++ uses buffer directly
3. **Validation overhead** - Rust has extensive checks C++ test lacks

The code is safe but not optimized for the SDK's zero-copy design pattern.

## Comparison Table

| Aspect | C++ stall_test | Rust acquisition.rs |
|--------|---------------|---------------------|
| Memory allocation per frame | 0 bytes | ~8MB + Arc |
| Frame data copy | No | Yes (to_vec) |
| SDK calls per frame | 1 (in callback) | 2-3 (get + unlock + optional metadata) |
| Validation checks | None | 4+ (duplicate, loss, zero-frame, receiver) |
| Synchronization | Simple condvar | Condvar + atomics + broadcast channel |
| Logging | cout | tracing with potential overhead |

## Recommendations

Before implementing changes, consider these architectural questions:

1. **Zero-copy frame delivery:** Can we use a ring buffer or memory pool to avoid per-frame allocation?

2. **Broadcast channel replacement:** Could a simpler signaling mechanism work for frame delivery?

3. **Validation placement:** Can validation be moved to a separate async task instead of the hot path?

4. **Profiling first:** Before optimizing, profile with `perf` or `samply` to confirm allocation is the bottleneck.

## References

- **SDK Examples:** `/opt/pvcam/sdk/examples/LiveImage.cpp`, `FastStreamingToDisk.cpp`
- **C++ Test:** `temp_repro/stall_test.cpp`
- **Rust Implementation:** `crates/driver-pvcam/src/components/acquisition.rs`
- **Performance Issue:** beads issue `bd-u1kx`
- **Stall Fix ADR:** `007-pvcam-85-frame-stall-fix.md`
