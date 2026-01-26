---
phase: 08-advanced-scans
plan: 01
subsystem: storage
tags: [zarr, storage, xarray, n-dimensional]

dependency-graph:
  requires: []
  provides: [zarr-v3-writer, xarray-compatibility, n-dimensional-storage]
  affects: [08-02, 08-03, 08-04]

tech-stack:
  added: [zarrs-0.22, object_store-0.11]
  patterns: [spawn_blocking-for-io, fluent-builder-api]

key-files:
  created:
    - crates/daq-storage/src/zarr_writer.rs
  modified:
    - crates/daq-storage/Cargo.toml
    - crates/daq-storage/src/lib.rs

decisions:
  - id: zarrs-crate
    choice: "zarrs 0.22 with filesystem feature"
    rationale: "Official Rust Zarr V3 implementation with object_store integration"
  - id: xarray-encoding
    choice: "_ARRAY_DIMENSIONS attribute in zarr.json"
    rationale: "Standard Xarray encoding convention for dimensional metadata"
  - id: storage-abstraction
    choice: "ReadableWritableListableStorage Arc pattern"
    rationale: "zarrs FilesystemStore not Clone - use Arc<dyn Storage> for sharing"

metrics:
  duration: 22min
  completed: 2026-01-25
---

# Phase 8 Plan 1: Zarr V3 Storage Foundation Summary

**One-liner:** Zarr V3 writer with `_ARRAY_DIMENSIONS` encoding for Xarray-compatible N-dimensional scientific data storage.

## What Was Built

### ZarrWriter Module (`crates/daq-storage/src/zarr_writer.rs`)

High-level API for Zarr V3 N-dimensional array storage with:

1. **ZarrWriter struct** - manages Zarr V3 store lifecycle
   - Creates store directory and root group on initialization
   - Stores arrays by name for efficient chunk writes
   - All I/O wrapped in `tokio::task::spawn_blocking`

2. **ZarrArrayBuilder** - fluent API for array creation
   - `.name()` - array path within store
   - `.shape()` - N-dimensional array dimensions
   - `.chunks()` - chunk sizes for storage optimization
   - `.dimensions()` - named dimensions for Xarray compatibility
   - `.dtype_*()` - support for u8, u16, u32, u64, i8-i64, f32, f64
   - `.attribute()` - custom metadata attributes
   - `.build()` - async array creation

3. **Key methods**:
   - `write_chunk<T>()` - incremental chunk writes by coordinate
   - `add_group_attribute()` - experiment-level metadata

### Feature Flag

- `storage_zarr` feature enables Zarr support
- Includes `zarrs` with `filesystem` feature and `object_store`

## Technical Details

### Xarray Compatibility

Arrays include `_ARRAY_DIMENSIONS` attribute in zarr.json:
```json
{
  "attributes": {
    "_ARRAY_DIMENSIONS": ["wavelength", "position", "y", "x"],
    "units": "counts"
  }
}
```

Python usage:
```python
import xarray as xr
ds = xr.open_zarr("experiment.zarr")
# Dimensions automatically recognized
```

### Storage Pattern

```
experiment.zarr/
+-- zarr.json              # Root group metadata
+-- camera_frames/
    +-- zarr.json          # Array metadata with _ARRAY_DIMENSIONS
    +-- c/                 # Chunk directory
        +-- 0/0/0          # Chunk at indices [0,0,0]
```

### Chunking Strategy

For nested scans (wavelength x position x camera):
- Shape: `[10, 5, 256, 256]`
- Chunks: `[10, 1, 256, 256]` - all wavelengths per position
- Target: 10-100 MB chunks for optimal performance

## Verification Results

1. Feature flag works: `cargo check -p daq-storage --features storage_zarr`
2. All tests pass: 7 zarr tests + 52 total storage tests
3. No HDF5 regressions: 47 tests pass with storage_hdf5
4. Clean lint: Only pre-existing pedantic warnings

## Commits

| Hash | Type | Description |
|------|------|-------------|
| e6d1cd87 | feat | Add zarrs and object_store dependencies with storage_zarr feature |
| dad080db | feat | Implement ZarrWriter with Xarray-compatible encoding |

## Deviations from Plan

None - plan executed exactly as written.

## Next Phase Readiness

Ready for 08-02 (Nested Scan Nodes):
- ZarrWriter available for N-dimensional data storage
- Chunking API supports arbitrary dimensions
- _ARRAY_DIMENSIONS encoding ensures Python/Xarray compatibility

### Open Items

- Cloud storage (S3/GCS) via object_store not exercised yet (deferred to Phase 10)
- Sharding codec for very large datasets not configured (deferred to performance optimization)
