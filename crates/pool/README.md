# daq-pool

Zero-allocation object pool for high-performance frame handling in the rust-daq system.

## Overview

This crate provides two complementary pool implementations optimized for the PVCAM frame processing pipeline where per-frame heap allocations are prohibitively expensive:

- **`Pool<T>`**: Generic object pool with lock-free access after acquire
- **`BufferPool`**: Specialized byte buffer pool with `bytes::Bytes` integration

## Key Design: Lock-Free Access

Unlike naive pool implementations that take a lock on every `get()` call, this pool caches the slot pointer at `Loaned` creation time. This eliminates per-access locking overhead, critical for high-throughput frame processing where frames may be accessed multiple times.

```rust
use daq_pool::Pool;

// Create pool with 30 frame buffers (~240MB for 8MB frames)
let pool = Pool::new_with_reset(
    30,
    || vec![0u8; 8 * 1024 * 1024],  // 8MB frame buffer
    |buf| buf.fill(0),               // Reset on return
);

// Acquire a buffer (no allocation!)
let mut frame = pool.acquire().await;
frame[0] = 42;  // Direct access via Deref - NO LOCK TAKEN

// Return to pool automatically when dropped
drop(frame);
```

## BufferPool for Zero-Copy Bytes

The `BufferPool` integrates with `bytes::Bytes` for true zero-copy frame handling:

```rust
use daq_pool::{BufferPool, PooledBuffer};
use bytes::Bytes;

// Create pool with 30 8MB buffers (~240MB total)
let pool = BufferPool::new(30, 8 * 1024 * 1024);

// Acquire and fill buffer
let mut buffer = pool.try_acquire().expect("pool exhausted");
unsafe {
    buffer.copy_from_ptr(sdk_ptr, frame_bytes);
}

// Convert to Bytes (zero-copy!)
let bytes: Bytes = buffer.freeze();

// bytes can be cloned, sent to consumers, etc.
// When all clones dropped, buffer returns to pool
```

## Memory Flow

```
1. BufferPool pre-allocates Vec<u8> buffers at startup
2. acquire() returns PooledBuffer (wraps buffer + pool reference)
3. Copy SDK data into buffer (mutable access via get_mut())
4. freeze() converts to Bytes (zero-copy, just Arc increment)
5. Bytes passed to Frame, broadcast to consumers
6. When all Bytes clones dropped, PooledBuffer::drop() runs
7. Buffer returned to pool for reuse
```

## Safety Model

The pool uses a semaphore + lock-free queue pattern:

1. **Semaphore** tracks available slots (permits = available items)
2. **`SegQueue`** holds indices of free slots (lock-free)
3. **`RwLock<Vec<UnsafeCell<T>>>`** only locked during:
   - `acquire()`: to get slot pointer (once per loan)
   - `release()`: to apply reset function
   - `grow()`: to add new slots (rare)
4. **`Loaned`** caches raw pointer for lock-free access thereafter

## PVCAM Integration

For PVCAM frame processing, use `try_acquire_timeout()` to avoid blocking longer than the SDK's buffer window:

```rust
// PVCAM uses CIRC_NO_OVERWRITE with 20-slot buffer
// At 100 FPS = ~200ms before data overwritten
// Use timeout well under this (50-100ms)
let buffer = pool.try_acquire_timeout(Duration::from_millis(50)).await;
```

## API Reference

### Pool<T>

| Method | Description |
|--------|-------------|
| `new(size, factory, reset)` | Create pool with factory and optional reset |
| `new_simple(size, factory)` | Create pool without reset function |
| `new_with_reset(size, factory, reset)` | Create pool with reset function |
| `acquire()` | Acquire item, blocking if none available |
| `try_acquire()` | Try to acquire without blocking |
| `try_acquire_timeout(duration)` | Try to acquire with timeout |
| `size()` | Get total pool size |
| `available()` | Get currently available count |

### Loaned<T>

| Method | Description |
|--------|-------------|
| `get()` | Lock-free immutable access |
| `get_mut()` | Lock-free mutable access |
| `clone_item()` | Clone contents and return slot to pool |
| `try_clone()` | Clone into new pool slot if available |
| `slot_index()` | Get slot index (for debugging) |

### BufferPool

| Method | Description |
|--------|-------------|
| `new(pool_size, buffer_capacity)` | Create buffer pool |
| `try_acquire()` | Try to acquire without blocking |
| `try_acquire_timeout(duration)` | Try to acquire with timeout |
| `available()` | Get currently available count |
| `stats()` | Get pool statistics |

### PooledBuffer

| Method | Description |
|--------|-------------|
| `get_mut()` | Get mutable slice for writing |
| `copy_from_ptr(ptr, len)` | Unsafe copy from raw pointer |
| `freeze()` | Convert to `Bytes` (zero-copy) |
| `capacity()` | Get buffer capacity |

## Performance Characteristics

- **Acquire**: O(1) amortized (semaphore + lock-free queue pop + brief read lock)
- **Access**: O(1) lock-free (cached pointer)
- **Release**: O(1) (optional reset + lock-free queue push + semaphore release)
- **Memory**: Pre-allocated at startup, no per-frame allocation

## Feature Flags

None. This crate has no optional features.

## License

MIT
