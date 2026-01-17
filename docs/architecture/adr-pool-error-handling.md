# ADR: Buffer Pool Error Handling Strategy

**Status:** Accepted
**Date:** 2026-01-17
**Author:** Architecture Review
**Related Issues:** bd-0dax.8, bd-0dax

---

## Context

The `daq-pool` crate provides zero-allocation object pools (`Pool<T>` and `BufferPool`) for high-performance frame handling in the PVCAM driver. At 100 FPS with 8MB frames, per-frame heap allocations cause GC pressure and latency spikes that exceed the SDK's buffer window.

This document defines the error handling strategy for pool operations, covering failure modes, recovery strategies, and metrics for monitoring pool health.

---

## Decision

**Error handling follows a "fail gracefully, degrade performance" philosophy.** Pool exhaustion is a recoverable condition that should not crash the system or lose data silently.

---

## Failure Modes & Responses

### 1. Pool Creation Failure

**Cause:** Out-of-memory during pre-allocation at startup

**Code Location:** `Pool::new()`, `BufferPool::new()`

```rust
let pool = BufferPool::new(30, 8 * 1024 * 1024);  // ~240MB pre-allocation
```

**Symptoms:**
- `panic!` from `vec![0u8; buffer_capacity]` failing
- System OOM killer intervention
- Process abort

**Response:**
- Return error from `start_stream()` or driver initialization
- Do NOT start acquisition if pool cannot be created
- Log error with requested size and available memory

**Implementation:**
```rust
pub fn try_new(pool_size: usize, buffer_capacity: usize) -> Result<Self, PoolError> {
    let total_bytes = pool_size.checked_mul(buffer_capacity)
        .ok_or(PoolError::AllocationOverflow)?;

    // Pre-flight check (optional, OS-specific)
    if !can_allocate(total_bytes) {
        return Err(PoolError::InsufficientMemory {
            requested: total_bytes,
            available: available_memory(),
        });
    }

    // Proceed with allocation...
}
```

**Recovery:** Reduce pool size or buffer capacity. On embedded/constrained systems, use smaller frame ROIs.

---

### 2. Pool Exhaustion (try_acquire returns None)

**Cause:** All slots in use by consumers (backpressure)

**Code Location:** `Pool::try_acquire()`, `BufferPool::try_acquire()`

**Symptoms:**
- `try_acquire()` returns `None`
- All semaphore permits exhausted
- Free index queue empty

**Response:**

The response depends on the acquisition context:

**Option A: Graceful Fallback (Current PVCAM Implementation)**
```rust
let (pixel_data, used_pool): (Bytes, bool) = match buffer_pool.try_acquire() {
    Some(mut buffer) => {
        unsafe { buffer.copy_from_ptr(frame_ptr, copy_bytes); }
        (buffer.freeze(), true)
    }
    None => {
        // Fallback: Heap allocation (slower, but data preserved)
        POOL_MISSES.fetch_add(1, Ordering::Relaxed);
        if should_log_warning() {
            tracing::warn!(
                pool_misses = misses,
                pool_available = buffer_pool.available(),
                "Buffer pool exhausted - falling back to heap allocation"
            );
        }
        let data = Bytes::copy_from_slice(unsafe {
            std::slice::from_raw_parts(frame_ptr, copy_bytes)
        });
        (data, false)
    }
};
```

**Option B: Drop Frame (Acceptable for Preview/Monitoring)**
```rust
match buffer_pool.try_acquire() {
    Some(slot) => { /* copy frame data */ }
    None => {
        frames_dropped_pool_exhaustion.fetch_add(1, Ordering::Relaxed);
        if should_log_warning() {
            tracing::warn!(
                dropped = frames_dropped_pool_exhaustion.load(Ordering::Relaxed),
                "Pool exhausted, dropping frame"
            );
        }
        continue;  // Skip to next frame
    }
}
```

**Choosing Between Options:**
| Context | Recommended | Rationale |
|---------|-------------|-----------|
| Scientific acquisition | Option A | Data integrity critical |
| Live preview/monitoring | Option B | Latency more important than every frame |
| Storage pipeline | Option A | All frames must be preserved |
| gRPC streaming (tap) | Option B | Taps are best-effort |

---

### 3. Timeout on Slot Acquisition

**Cause:** Consumers holding slots longer than expected

**Code Location:** `Pool::try_acquire_timeout()`, `BufferPool::try_acquire_timeout()`

**Context:** PVCAM uses `CIRC_NO_OVERWRITE` mode with a 20-slot circular buffer. At 100 FPS, this gives ~200ms before the SDK overwrites the oldest frame. The timeout should be well under this threshold.

```rust
// Recommended timeout: 50-100ms (25-50% of SDK buffer window)
let slot = pool.try_acquire_timeout(Duration::from_millis(75)).await;
```

**Symptoms:**
- `try_acquire_timeout()` returns `None` after timeout expires
- Warning logged with timeout duration and pool state

**Response:**
```rust
let slot = match pool.try_acquire_timeout(Duration::from_millis(75)).await {
    Some(s) => s,
    None => {
        acquire_timeout_count.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            timeout_ms = 75,
            available = pool.available(),
            size = pool.size(),
            "Pool acquire timeout - severe backpressure detected"
        );

        // Decision point: fallback allocation or drop frame
        // For scientific data, prefer fallback
        return fallback_heap_allocation(frame_ptr, frame_bytes);
    }
};
```

**Recovery:** Investigate consumer bottleneck. Common causes:
- Storage I/O too slow
- Network congestion on gRPC stream
- GUI rendering blocking

---

### 4. Primary Channel Full (Consumer Backed Up)

**Cause:** Frame consumer (storage writer, gRPC sender) processing slower than frame rate

**Response:**
```rust
if primary_tx.try_send(frame).is_err() {
    channel_full_counter.fetch_add(1, Ordering::Relaxed);
    // Frame (Loaned<FrameData>) dropped here
    // Buffer automatically returns to pool via Drop impl

    if should_log_warning() {
        tracing::warn!(
            dropped = channel_full_counter.load(Ordering::Relaxed),
            "Primary channel full, frame dropped"
        );
    }
}
```

**Key Insight:** When a `Loaned<T>` is dropped (explicitly or by channel rejection), the buffer automatically returns to the pool via the `Drop` implementation. No explicit cleanup is required.

---

### 5. Primary Channel Closed (Consumer Crashed)

**Cause:** Receiver task panicked, was cancelled, or dropped

**Symptoms:**
- `try_send()` returns `Err(SendError)`
- Channel receiver is `None`

**Response:**
```rust
match primary_tx.try_send(frame) {
    Ok(()) => { /* success */ }
    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
        tracing::error!("Primary consumer channel closed - stopping acquisition");

        // Set shutdown flag to stop acquisition gracefully
        shutdown_flag.store(true, Ordering::Release);
        break;
    }
    Err(tokio::sync::mpsc::error::TrySendError::Full(frame)) => {
        // Channel full - drop frame (handled above)
        drop(frame);
    }
}
```

**Recovery:** Stop acquisition gracefully. Do not attempt to reconnect the channel - let the higher-level orchestration handle restart.

---

### 6. Tap Channel Full (Best-Effort Consumers)

**Cause:** GUI or gRPC stream consumer too slow

**Response:**
```rust
// Taps are explicitly best-effort - no logging on drop
if let Err(frame) = tap_tx.try_send(frame.clone()) {
    tap_drops.fetch_add(1, Ordering::Relaxed);
    // No logging - expected behavior under load
    // frame returned to pool via clone's Drop
}
```

**Distinction from Primary Channel:**
| Attribute | Primary Channel | Tap Channel |
|-----------|-----------------|-------------|
| Semantics | Guaranteed delivery | Best-effort |
| On full | Log warning | Silent drop |
| On closed | Stop acquisition | Remove tap |

---

### 7. Semaphore Closed Unexpectedly

**Cause:** Pool dropped while items still loaned (programming error)

**Code Location:** `Pool::acquire()`, `BufferPool::acquire()`

```rust
let permit = self.semaphore.acquire().await
    .expect("semaphore closed unexpectedly");
```

**Response:** This is a logic error indicating:
- Pool was dropped while `Loaned` items still exist
- Reference counting bug in pool lifecycle management

**Action:** Panic is appropriate. This should never happen in correct code.

**Prevention:** The `Arc<Pool<T>>` design ensures the pool outlives all `Loaned<T>` instances because each `Loaned` holds an `Arc` clone.

---

### 8. Free Queue Desync (Internal Invariant Violation)

**Cause:** Bug in semaphore/queue coordination

**Code Location:** `Pool::try_acquire()`, `BufferPool::try_acquire()`

```rust
let idx = self.free_indices.pop()
    .expect("free list empty after permit - internal invariant violated");
```

**Response:** Panic. This indicates a fundamental bug in pool implementation:
- Semaphore permit acquired but no index in free queue
- Possible race condition or double-return

**Prevention:** The semaphore and free queue must always be in sync:
- Acquire: Semaphore permit first, then pop index
- Release: Push index first, then add permit

---

## Metrics to Track

```rust
/// Metrics for pool health monitoring
pub struct PoolMetrics {
    /// Frames dropped due to pool exhaustion (try_acquire returned None)
    pub frames_dropped_pool_exhaustion: AtomicU64,

    /// Frames dropped due to acquire timeout
    pub frames_dropped_timeout: AtomicU64,

    /// Frames dropped due to primary channel full
    pub frames_dropped_channel_full: AtomicU64,

    /// Frames dropped at tap channels (per tap, best-effort)
    pub frames_dropped_tap: AtomicU64,

    /// Fallback heap allocations (when pool exhausted but data preserved)
    pub heap_fallback_count: AtomicU64,

    /// High water mark of pool utilization
    pub pool_high_water_mark: AtomicUsize,

    /// Current pool utilization (for real-time monitoring)
    pub pool_current_usage: AtomicUsize,

    /// Total successful pool acquisitions
    pub pool_hits: AtomicU64,

    /// Total pool misses (exhaustion events)
    pub pool_misses: AtomicU64,
}

impl PoolMetrics {
    /// Calculate pool hit rate as percentage
    pub fn hit_rate_percent(&self) -> f64 {
        let hits = self.pool_hits.load(Ordering::Relaxed);
        let misses = self.pool_misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            100.0
        } else {
            (hits as f64 / total as f64) * 100.0
        }
    }
}
```

---

## Rate-Limited Logging

All warning logs should be rate-limited to prevent log spam under sustained backpressure:

```rust
/// Log at most once per second, then every Nth occurrence
fn should_log_warning(count: u64) -> bool {
    count <= 10 || count % 100 == 0
}

// Usage
if should_log_warning(pool_misses) {
    tracing::warn!(
        pool_misses = pool_misses,
        pool_available = pool.available(),
        "Buffer pool exhausted"
    );
}
```

---

## Recovery Strategies

### Transient Backpressure

**Symptoms:** Occasional pool exhaustion, recovers automatically

**Strategy:**
1. Use heap fallback to preserve data
2. Monitor hit rate percentage
3. Alert if hit rate < 95% sustained over 10 seconds

### Sustained Backpressure

**Symptoms:** Hit rate < 80%, continuous fallback allocations

**Strategy:**
1. Increase pool size (if memory permits)
2. Reduce frame rate or ROI
3. Investigate consumer bottleneck
4. Consider adding frame decimation (drop every Nth frame for preview)

### Consumer Failure

**Symptoms:** Channel closed, consumer task dead

**Strategy:**
1. Stop acquisition gracefully
2. Log error with consumer identity
3. Allow higher-level orchestration to restart
4. Do NOT auto-restart within driver (could cause infinite loops)

---

## Design Rationale

### Why Panic on Invariant Violations?

Semaphore/queue desync and semaphore closed are programming errors, not runtime conditions. Panicking:
- Makes bugs immediately visible
- Prevents silent data corruption
- Is the Rust-idiomatic response to logic errors

### Why Fallback Instead of Drop?

For scientific acquisitions, data integrity trumps performance. A heap allocation is slow (~100μs vs ~1μs) but preserves the frame. For scientific use cases, missing data is unacceptable.

### Why Best-Effort for Taps?

Taps (GUI preview, gRPC streaming) are secondary consumers. The primary data pipeline must not be blocked by slow secondary consumers. Dropping frames at taps is acceptable and expected.

### Why Rate-Limited Logging?

At 100 FPS with sustained backpressure, logging every event would:
- Flood log files (100 warnings/second)
- Consume I/O bandwidth needed for data
- Make logs unreadable

---

## Acceptance Criteria (from bd-0dax.8)

- [x] All failure modes have documented responses
- [x] Pool exhaustion doesn't panic or corrupt state
- [x] Channel closure is handled gracefully
- [x] Metrics track all drop types
- [x] Warnings are rate-limited to prevent log spam

---

## References

- [ADR: PVCAM Driver Architecture](adr-pvcam-driver-architecture.md)
- [ADR: PVCAM Continuous Acquisition](adr-pvcam-continuous-acquisition.md)
- [daq-pool crate](../../crates/daq-pool/src/lib.rs)

---

## Revision History

| Date | Author | Description |
|------|--------|-------------|
| 2026-01-17 | bd-0dax.8 | Initial error handling strategy documentation |
