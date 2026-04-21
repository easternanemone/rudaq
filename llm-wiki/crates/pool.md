# crate: `pool`

<!--
last-ingested: 2026-04-19
sources:
  - crates/pool/Cargo.toml
  - crates/pool/src/
see-also:
  - ../concepts/ring-buffer.md
  - ./storage.md
-->

**Role:** Zero-allocation object pool for frame handling. Critical for
high-FPS camera streaming where per-frame allocations cause latency.

**Key exports:**

- `Pool<T>` — generic lock-free object pool.
- `BufferPool` — byte-buffer pool with `bytes::Bytes` integration.
- `Loaned<T>` — RAII handle; returns to pool on drop.
- `ForeignView` trait — zero-copy access from Python / C++ / GPU consumers.
- `BorrowGuard` / `BorrowCount` — prevent slot reclamation while foreign code holds references.
- `DlPackDescriptor` (feature `dlpack`) — tensor metadata for NumPy / PyTorch interop.

**Dependents:** `storage`, `driver-pvcam`, `driver-andor-sdk3`,
`driver-mock` (frame-producing drivers), `server`.

**Rules:**

- Never `clone()` a `Loaned<T>`. The single-owner drop is what triggers
  return to the pool.
- Frame-producing drivers must `acquire()` from the pool, not allocate.
- Size pool at startup for peak FPS + maximum in-flight frames across consumers.
