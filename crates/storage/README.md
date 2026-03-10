# storage

High-throughput buffering and persistence infrastructure for rust-daq.

## Overview

The `storage` crate provides:

- ring-buffer infrastructure for live data flow
- persistence backends for HDF5, Arrow, Parquet, TIFF, and Zarr
- helper readers/config for downstream consumers

## Feature Flags

| Feature | Purpose |
|---------|---------|
| `storage_hdf5` | HDF5 persistence |
| `storage_arrow` | Arrow IPC support |
| `storage_parquet` | Parquet support (depends on Arrow) |
| `storage_tiff` | TIFF export |
| `storage_zarr` | Zarr V3 support |
| `networking` | networking-related support used by some integrations |
| `metrics` | storage metrics/observability hooks |

## Writer Types

Current writer types in `src/` include:

- `Hdf5Writer`
- `ArrowDocumentWriter`
- `ParquetDocumentWriter`
- `ParquetWriter`
- `ZarrWriter`
- `TiffWriter`

## Ring Buffer

The ring-buffer layer supports mmap-backed producer/consumer workflows and tap-style readers for downstream consumers.

## Examples and Benches

Examples and benches live in this crate, not the workspace root:

- `crates/storage/examples/`
- `crates/storage/benches/`

For example, `ring_arrow_bench` is feature-gated on `storage_arrow`.

## Related Crates

- `common` — shared frame/data types
- `pool` — allocation-conscious buffer management
- `experiment` — orchestration and document flow
- `server` — runtime streaming and persistence integration
