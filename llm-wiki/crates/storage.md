# crate: `storage`

<!--
last-ingested: 2026-04-19
sources:
  - crates/storage/
  - docs/explanation/architecture.md §Data Pipeline
  - docs/adr/015-hybrid-persistence-architecture.md
see-also:
  - ../concepts/ring-buffer.md
  - ./pool.md
-->

**Role:** High-throughput storage + buffering. Implements the "Mullet
Strategy": Arrow IPC ring buffer in front (low-latency), HDF5 in back
(reliable).

**Supported formats:** HDF5, Arrow IPC, Parquet, TIFF, Zarr (V3).

**Key types:**

- `RingBuffer` (`ring_buffer.rs`) — mmap + seqlock + Arrow IPC. See [`../concepts/ring-buffer.md`](../concepts/ring-buffer.md).
- `DocumentSink` trait — consumer API that decouples RunEngine document production from storage format.
- `HdfDocumentSink`, `ArrowDocumentSink`, `ZarrSink` (feature `storage_zarr`) — built-in impls.
- `TiffWriter::write_frame_data` — current TIFF API. `TiffWriter::write_frame` carries `#[deprecated(since = "0.3.0")]` (verified `crates/storage/src/tiff_writer.rs:91`).

**Feature flags:** `storage_arrow` (Arrow IPC + tensor formatting);
`storage_zarr` (Zarr V3).

**Dependents:** `server`, `experiment`, `bin`.
