# Ring Buffer (mmap + seqlock + Arrow IPC)

<!--
last-ingested: 2026-04-19
sources:
  - crates/storage/src/ring_buffer.rs
  - crates/pool/
  - docs/explanation/architecture.md §Data Pipeline
  - docs/adr/014-frame-streaming-buffer-reuse.md
see-also:
  - ../architecture.md §Data flow
  - ../crates/pool.md
  - ../crates/storage.md
-->

The low-latency leg of the Mullet Strategy. Zero-copy frame streaming from
driver to viewers.

## Primitives

| Concept | Location | Role |
|---------|----------|------|
| `Pool<T>` / `BufferPool` | `crates/pool` | Pre-allocated object pool. `Loaned<T>` returns to pool on last drop. |
| `RingBuffer` | `crates/storage/src/ring_buffer.rs` | mmap-backed, seqlock synchronized, Arrow IPC layout. |
| `FrameView<'a>` | `common` | Zero-copy borrow of frame bytes from driver. |
| `FrameObserver` trait | `common-traits` | Consumer that taps the stream without owning it. |

## Ring buffer on disk

Path: **configurable** — the caller passes the buffer path to the
`RingBuffer` constructor (`crates/storage/src/ring_buffer.rs:189`). Deploy
to a tmpfs location (e.g. `/dev/shm/…`) for RAM-backed POSIX mmap.
Tests use ephemeral `tempfile::TempDir` paths. There is no hardcoded
`/dev/shm/ring.buf` in the code; older docs that stated one were
inaccurate.

Layout: Arrow IPC record batches, fixed-size slots. Seqlock version
counters let readers detect torn reads without locking writers.

Cross-process access: Python or Julia can open the buffer file and read
live frames without entering the Rust acquisition loop, as long as the
daemon config and the reader agree on the path.

## Pipeline (camera → GUI)

```
Camera driver
  → FrameView<'a> (zero-copy borrow)
     → GrpcStreamObserver::on_frame()
        • Full:    frame.pixels().to_vec()         (ALLOC #1 — copy from driver)
        • Preview: downsample_2x2 → Vec<u8>
        • Fast:    downsample_4x4 → Vec<u8>
     → tokio::mpsc
     → Dedicated LZ4 compression thread (std::thread, not spawn_blocking)
        • compress_frame_into(&mut frame, &mut reusable_buf)   (buffer reused)
     → tokio::mpsc
     → Async forwarder (rate limit + backpressure + metrics)
     → gRPC stream
  ~~~ network ~~~
Client ImageViewerPanel
  → decompress_frame_into(&mut frame, &mut reusable_buf)       (buffer reused)
  → FrameUpdate { data: Arc<Vec<u8>>, … }
  → RGBA converter thread
     • convert_frame_to_rgba_into(req, &mut reusable_rgba_buf) (buffer reused)
  → egui TextureHandle::set
```

Buffer-reuse APIs (`protocol::compression`):

- `compress_frame_into(frame, buf)` / `decompress_frame_into(frame, buf)` — write into pre-allocated `Vec<u8>` via `std::mem::swap`. Wire-compatible with allocating variants.

Threading:

- Compression thread is a long-lived `std::thread` to avoid Tokio blocking-pool scheduling overhead (~50–200 µs per frame).
- RGBA converter thread, same pattern, client side.

## Why seqlock over RwLock

Seqlocks let writers never block, and let readers detect inconsistency
without acquiring a lock. For a single-writer, many-readers frame stream
at >100 fps with multi-MB frames, RwLock contention would cost multiple
frames per second.

## Consumers

- **HDF5 writer** (`storage/src/hdf5.rs`) — reliable back-of-the-mullet path.
- **gRPC stream observer** — live path to GUI.
- **Python / Julia analysis** — `/dev/shm/ring.buf` open + read.

All impl `DocumentSink` (or a lower-level frame-observer equivalent).

## Relevant ADRs

- ADR-014: Frame streaming buffer reuse.
- ADR-015: Hybrid persistence architecture.
