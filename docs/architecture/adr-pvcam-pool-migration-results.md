# ADR: PVCAM Memory Pool Migration Results

**Status:** Migration Complete
**Date:** 2026-01-17
**Author:** Architecture Review
**Related Issues:** bd-0dax (epic), bd-0dax.7.4
**Last Updated:** 2026-01-26

---

## Context

The PVCAM driver was allocating approximately 16MB per frame at 100 FPS, causing ~1.6GB/sec of allocation churn. This created several problems:

| Problem | Impact |
|---------|--------|
| Allocation latency | `malloc()` introduces jitter in frame acquisition hot path |
| Memory fragmentation | Rapid large allocations fragment the heap |
| Cache inefficiency | Non-contiguous allocations cause cache misses |
| GC pressure | High allocation rate triggers frequent garbage collection |

### Original Allocation Hot Spots

| Allocation | Size | Location |
|------------|------|----------|
| `sdk_bytes.to_vec()` | ~8 MB | acquisition.rs |
| `Arc::new(frame)` | 64 bytes | acquisition.rs |
| Arrow tap data clone | ~8 MB | acquisition.rs |
| **Total** | **~16 MB/frame** | |

At 100 FPS with 8 MB frames, this resulted in **~200 allocations/sec** and **~1.6 GB/sec allocation rate**.

---

## Decision

Implemented a zero-allocation pool architecture with:

1. **Pre-allocated BufferPool** for frame data buffers
2. **Single-owner mpsc + tap observer pattern** for frame distribution
3. **FrameView** for zero-copy borrowed access by observers
4. **FrameData** as the poolable frame structure

This design eliminates heap allocations from the hot path while maintaining flexibility for multiple consumers.

---

## Changes Made

### 1. daq-pool Crate

Created new `daq-pool` crate (`crates/daq-pool/`) with:

- **`Pool<T>`**: Generic object pool with lock-free access after acquire
  - Semaphore + lock-free queue pattern for thread safety
  - RwLock only taken during acquire/release/grow, NOT during access
  - Cached slot pointer in `Loaned<T>` eliminates per-access locking (bd-0dax.1.6)

- **`BufferPool`**: Specialized byte buffer pool with `bytes::Bytes` integration
  - Pre-allocated byte vectors
  - `freeze()` converts to zero-copy `Bytes`

- **`Loaned<T>`**: RAII guard for pooled items
  - Automatic return to pool on drop
  - `Deref`/`DerefMut` for transparent access
  - Clone support (acquires new pool slot)

Key files:
- `crates/daq-pool/src/lib.rs` - Pool<T> and Loaned<T>
- `crates/daq-pool/src/buffer_pool.rs` - BufferPool implementation
- `crates/daq-pool/src/frame_data.rs` - FrameData type

### 2. FrameData (Poolable Frame Structure)

Created `FrameData` in `daq-pool` for zero-allocation reuse:

```rust
pub struct FrameData {
    // Pre-allocated pixel buffer (never shrinks)
    pub pixels: Vec<u8>,
    pub actual_len: usize,

    // Frame identity
    pub frame_number: u64,
    pub hw_frame_nr: i32,

    // Dimensions and timing
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub timestamp_ns: u64,
    pub exposure_ms: f64,

    // ROI and metadata (inline, not boxed)
    pub roi_x: u32,
    pub roi_y: u32,
    pub temperature_c: Option<f64>,
    pub binning: Option<(u16, u16)>,
}
```

Design choices:
- **Fixed-capacity pixel buffer**: Pre-allocated, never shrinks
- **Inline metadata**: No Box allocation for metadata fields (~100 bytes)
- **O(1) reset**: Clears metadata, preserves buffer capacity
- **No pixel zeroing**: Previous frame data overwritten by next memcpy

### 3. FrameView (Zero-Copy Borrowed Access)

Added `FrameView<'a>` to `common::data` for observer pattern:

```rust
pub struct FrameView<'a> {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pixels: &'a [u8],  // Borrowed, not owned
    pub frame_number: u64,
    pub timestamp_ns: u64,
    // ... additional metadata
}
```

Benefits:
- **Zero allocation** when adapting internal frame types for observers
- **Copy-on-write** semantics: observers must copy if they need to persist

### 4. FrameObserver Trait

Added observer pattern to `common::capabilities`:

```rust
pub trait FrameObserver: Send + Sync {
    /// Called synchronously for each frame. MUST NOT block.
    fn on_frame(&self, frame: &FrameView<'_>);

    fn name(&self) -> &str { "unnamed_observer" }
}

pub struct ObserverHandle(pub u64);
```

Contract:
- `on_frame()` MUST complete quickly (< 1ms recommended)
- Implementations MUST copy data if persistence needed
- Implementations MUST handle backpressure internally

### 5. TapRegistry

Added `TapRegistry` to `daq-storage` for managing frame observers:

- Registration/unregistration of tap consumers
- Decimation support (deliver every Nth frame)
- Non-blocking delivery with backpressure handling
- Per-tap dropped frame counters

### 6. FrameProducer Trait Updates

Updated `FrameProducer` trait in `common::capabilities`:

```rust
pub type LoanedFrame = daq_pool::Loaned<daq_pool::FrameData>;

#[async_trait]
pub trait FrameProducer: Send + Sync {
    // ... existing methods ...

    /// Register primary frame consumer (zero-allocation)
    async fn register_primary_output(
        &self,
        tx: tokio::sync::mpsc::Sender<LoanedFrame>,
    ) -> Result<()>;

    /// Register frame observer for secondary access
    async fn register_observer(
        &self,
        observer: Box<dyn FrameObserver>,
    ) -> Result<ObserverHandle>;

    /// Unregister a frame observer
    async fn unregister_observer(&self, handle: ObserverHandle) -> Result<()>;
}
```

Deprecations:
- `subscribe_frames()` - replaced by `register_primary_output()`
- `take_frame_receiver()` - replaced by `register_primary_output()`

---

## API Migration

| Old API | New API |
|---------|---------|
| `subscribe_frames()` -> `broadcast::Receiver<Arc<Frame>>` | `register_primary_output()` -> `mpsc::Receiver<LoanedFrame>` |
| N/A (no observer pattern) | `register_observer()` -> `ObserverHandle` |
| Multiple broadcast subscribers | Single primary + multiple observers |

### Migration Example

**Before (heap allocation per frame):**
```rust
let rx = camera.subscribe_frames().await?;
camera.start_stream().await?;
while let Ok(frame) = rx.recv().await {
    // Arc<Frame> - heap allocated per frame
    process(frame);
}
```

**After (zero allocation):**
```rust
let (tx, mut rx) = tokio::sync::mpsc::channel(32);
camera.register_primary_output(tx).await?;
camera.start_stream().await?;
while let Some(frame) = rx.recv().await {
    // LoanedFrame - from pre-allocated pool
    process(&frame);
    // frame dropped here, returns to pool automatically
}
```

---

## Performance

### Expected Metrics

| Metric | Before | After |
|--------|--------|-------|
| Allocations/sec | ~200 | ~0 (after warmup) |
| Allocation rate | ~1.6 GB/sec | ~0 MB/sec in hot path |
| Pool acquire/release | N/A | Sub-microsecond |
| RwLock contention | N/A | None (cached pointer) |

### Key Optimizations

1. **Cached slot pointer (bd-0dax.1.6)**: `Loaned<T>` caches the slot pointer at acquire time, eliminating RwLock access on every `Deref` call.

2. **No pixel zeroing**: `FrameData::reset()` only clears metadata (~100 bytes), not the 8MB pixel buffer. Previous frame data is overwritten by the next `copy_from_sdk()` call.

3. **Semaphore-based slot management**: Lock-free availability tracking via `tokio::sync::Semaphore`.

4. **Graceful fallback**: Pool exhaustion falls back to heap allocation rather than blocking, preserving data integrity.

---

## Error Handling

Pool operations follow a "fail gracefully, degrade performance" philosophy:

| Failure Mode | Response |
|--------------|----------|
| Pool exhaustion | Fall back to heap allocation, log warning |
| Acquire timeout | Fall back to heap, log backpressure warning |
| Primary channel full | Drop frame, increment counter |
| Tap channel full | Silent drop (best-effort semantics) |
| Semaphore closed | Panic (programming error) |

See [ADR: Buffer Pool Error Handling Strategy](adr-pool-error-handling.md) for detailed error handling documentation.

---

## Status Summary

The migration from deprecated `subscribe_frames()` to the new pooled frame APIs is complete across the codebase:

1. **Primary frame delivery (bd-0dax.5)**: `register_primary_output()` is fully integrated into PVCAM and generic driver frame loops, delivering `LoanedFrame` ownership to primary consumers.

2. **Frame observers/taps (bd-0dax.4)**: `register_observer()` and `unregister_observer()` provide non-blocking secondary access via `FrameView` references for monitoring, experiment capture, and UI preview.

3. **Deprecation warnings**: The old `subscribe_frames()` and `take_frame_receiver()` methods are marked deprecated and documented to use the new APIs instead.

4. **Documentation updated**: All docstrings, examples, and guides reference the new zero-allocation APIs.

---

## Lessons Learned

1. **Pointer caching is critical**: Initial implementation locked RwLock on every `Deref`, causing measurable overhead. Caching the slot pointer at acquire time (bd-0dax.1.6) eliminated this.

2. **Graceful degradation > hard failure**: Scientific data is precious. Falling back to heap allocation on pool exhaustion preserves data at the cost of performance, which is the right tradeoff.

3. **Observer pattern enables flexibility**: The primary/observer split allows zero-copy delivery to the primary consumer while still supporting secondary consumers (GUI preview, gRPC streaming) without blocking.

4. **Inline metadata avoids allocation chaining**: Boxing metadata would add another allocation per frame. Inline fields (~100 bytes) are negligible compared to the 8MB pixel buffer.

---

## References

- [bd-0dax: PVCAM Memory Pool Architecture Migration (Epic)](../../.beads/issues/bd-0dax.md)
- [ADR: Buffer Pool Error Handling Strategy](adr-pool-error-handling.md)
- [ADR: BufferPool Migration Rollback Plan](adr-pool-migration-rollback-plan.md)
- [ADR: PVCAM Driver Architecture](adr-pvcam-driver-architecture.md)
- [daq-pool crate](../../crates/daq-pool/src/lib.rs)

---

## Revision History

| Date | Author | Description |
|------|--------|-------------|
| 2026-01-17 | bd-0dax.7.4 | Initial migration results documentation |
