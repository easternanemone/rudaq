# ADR-014: Frame Streaming Buffer Reuse and Compression Threading

**Status:** Implemented
**Date:** 2026-03-07
**Author:** briansquires
**Related Issues:** PR #417

---

## Context

The daemon-to-GUI frame streaming pipeline performs **6 heap allocations per frame**:

| # | Location | Allocation | Size (4MP 16-bit) |
|---|----------|------------|-------------------|
| 1 | `GrpcStreamObserver::on_frame()` | `to_vec()` or downsample | ~8 MB |
| 2 | `FrameData` protobuf construction | data field moved (no copy) | 0 |
| 3 | `lz4_flex::compress_prepend_size()` | new `Vec<u8>` | ~2-4 MB |
| 4 | `lz4_flex::decompress_size_prepended()` | new `Vec<u8>` | ~8 MB |
| 5 | `FrameUpdate::from()` | `Vec<u8>` → `Arc<[u8]>` memcpy | ~8 MB |
| 6 | RGBA conversion buffer | **already recycled** | ~32 MB |

For a 4MP 16-bit camera at 30fps, allocations #1 + #3-5 create ~480 MB/s of
allocation churn. While modern allocators (mimalloc) handle this, the churn
causes unnecessary memory pressure, TLB misses, and GC pauses.

Additionally, each frame's LZ4 compression was dispatched through
`tokio::task::spawn_blocking()`, adding ~50-200μs of scheduling overhead per
frame (thread pool wake + task queue contention).

## Decision

### 1. Buffer-reuse compression/decompression API

Add `compress_frame_into()` and `decompress_frame_into()` to `protocol::compression`.
These accept a `&mut Vec<u8>` buffer and use `std::mem::swap` to exchange the
frame's data with the buffer contents:

```rust
pub fn compress_frame_into(frame: &mut FrameData, buffer: &mut Vec<u8>) { ... }
pub fn decompress_frame_into(frame: &mut FrameData, buffer: &mut Vec<u8>) -> Result<(), String> { ... }
```

After compression, the buffer holds stale uncompressed data (correct capacity,
wrong contents) — perfect for reuse on the next frame. The `Vec` never shrinks,
so after the first frame the capacity is stable.

The wire format is identical to the allocating API (4-byte LE size prefix +
LZ4 block), ensuring backward compatibility.

### 2. Dedicated compression thread

Replace per-frame `spawn_blocking` with a long-lived `std::thread` named
`lz4-compress-{device_id}`. The thread:
- Receives `(ObserverFramePacket, StreamingMetrics)` from a `tokio::mpsc` channel
- Owns a persistent compression buffer (pairs with optimization 1)
- Sends compressed `FrameData` back to the async forwarding task

The async forwarding task uses `tokio::select!` to multiplex three sources:
observer input, compressed output, and gRPC client disconnect.

### 3. `Arc<Vec<u8>>` for `FrameUpdate.data`

Change `FrameUpdate.data` from `Arc<[u8]>` to `Arc<Vec<u8>>`.

`Arc<[u8]>` is a dynamically-sized type (DST). Constructing it from `Vec<u8>`
requires allocating a new `Arc` with the data stored inline after the header,
which means copying all the bytes. For an 8 MB frame, that's an 8 MB memcpy.

`Arc<Vec<u8>>` wraps the existing `Vec` heap allocation. The `Arc` header
is pointer-sized and the data stays in place — no copy needed. Downstream
code accesses it via `Deref` chains: `Arc<Vec<u8>>` → `&Vec<u8>` → `&[u8]`.

## Consequences

### Positive

- Eliminates allocations #3, #4, and #5 (compression buffer, decompression
  buffer, Arc layout conversion) — 3 of the 6 per-frame allocations.
- Removes ~50-200μs of Tokio blocking-pool scheduling overhead per frame.
- The compression thread naturally owns its buffer, making the buffer-reuse
  pattern zero-cost to integrate.
- All APIs are wire-compatible: old clients can connect to new servers and
  vice versa.

### Negative

- One additional OS thread per active frame stream (lightweight — it blocks
  on channel recv when idle, zero CPU when no frames are flowing).
- `Arc<Vec<u8>>` adds one pointer indirection vs `Arc<[u8]>` for data access.
  This is negligible compared to the 8 MB memcpy it eliminates.
- The `tokio::select!` in the forwarding task is now three-armed instead of
  two-armed, slightly increasing code complexity.

### Not implemented (future work)

- **Fuse copy + compress in observer callback** (optimization 4 from plan):
  Compress `FrameView`'s borrowed `&[u8]` directly, eliminating allocation #1.
  Requires benchmarking to ensure it doesn't cause frame drops on the driver
  thread at high frame rates.
- **Server-side RGBA display mode**: Send pre-colormapped 8-bit RGBA frames,
  halving bandwidth and eliminating client-side RGBA conversion entirely.
  Larger feature with UX implications (loses client-side contrast adjustment).

## Verification

- `cargo nextest run -p protocol` — 56 tests including roundtrip and
  cross-API compatibility tests for `compress_frame_into`/`decompress_frame_into`
- `cargo nextest run -p server -p ui -p integration-tests --profile ci` —
  1157 tests including `test_stream_frames_rate_limiting_and_metrics`
- `cargo check -p ui --target wasm32-unknown-unknown --no-default-features --features web` —
  WASM compilation smoke test
- Hardware-in-the-loop validation pending (deploy to maitai, verify frame
  streaming at full resolution with FPS counter)
