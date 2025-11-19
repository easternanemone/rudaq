# HDF5 Storage Actor - Quick Reference

## Files Overview

| File | Purpose | Size |
|------|---------|------|
| `src/actors/hdf5_storage.rs` | Core implementation | 17 KB |
| `src/actors/mod.rs` | Module exports | Updated |
| `docs/guides/hdf5_storage_guide.md` | User guide | 9.3 KB |
| `docs/architecture/hdf5_actor_design.md` | Architecture | 10 KB |
| `examples/hdf5_storage_example.rs` | Runnable example | 8 KB |
| `IMPLEMENTATION_SUMMARY.md` | Executive summary | 10 KB |

## Core Types

### Actor
```rust
pub struct HDF5Storage { ... }
impl kameo::Actor for HDF5Storage { ... }
```

### Messages
```rust
WriteBatch { batch: Option<Vec<u8>>, instrument_id: String }
SetMetadata { key: String, value: String }
SetInstrumentMetadata { instrument_id: String, key: String, value: String }
Flush
GetStats
```

### Response
```rust
pub struct StorageStats {
    pub bytes_written: u64,
    pub batches_written: u64,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub num_datasets: u64,
}
```

## Configuration

### TOML
```toml
[storage]
default_backend = "hdf5"
output_dir = "./data"
compression_level = 6
auto_flush_interval_secs = 30
```

### Environment
```bash
export RUST_DAQ_STORAGE_DEFAULT_BACKEND=hdf5
export RUST_DAQ_STORAGE_OUTPUT_DIR=/mnt/data
export RUST_DAQ_STORAGE_COMPRESSION_LEVEL=9
export RUST_DAQ_STORAGE_AUTO_FLUSH_INTERVAL_SECS=60
```

## Usage Pattern

```rust
use rust_daq::config_v4::StorageConfig;
use rust_daq::actors::{HDF5Storage, WriteBatch, GetStats};

// Create and spawn
let config = StorageConfig { ... };
let storage = HDF5Storage::new(&config);
let storage_ref = storage.spawn();

// Send data
let batch = WriteBatch {
    batch: Some(arrow_ipc_data),
    instrument_id: "meter_01".to_string(),
};
storage_ref.call(batch).await?;

// Check stats
let stats = storage_ref.call(GetStats).await?;
println!("Wrote {} batches", stats.batches_written);
```

## Building

```bash
# Without HDF5 (always works)
cargo check --features v4
cargo test --lib actors::hdf5_storage

# With HDF5 (requires system library)
brew install hdf5  # macOS
cargo build --features v4,storage_hdf5
```

## HDF5 File Structure

```
daq_session_YYYYMMDD_HHMMSS.h5
├─ Root Attributes
│  ├─ created_at: ISO 8601 timestamp
│  ├─ created_timestamp_ns: u64
│  └─ application: "Rust DAQ V4"
│
└─ instrument_id (Group)
   ├─ Attributes
   │  ├─ instrument_id: String
   │  └─ created_at: String
   │
   └─ column_name (Dataset)
      ├─ Data: Float64, Int64, Int32, or String array
      └─ Attributes
         └─ schema: Arrow schema JSON
```

## Inspecting Files

```bash
# List structure
h5ls -r data/daq_session_*.h5

# View full dump
h5dump data/daq_session_*.h5

# Extract dataset
h5dump -d /instrument_id/column_name data/daq_session_*.h5

# View attributes
h5dump -A data/daq_session_*.h5
```

## Performance Tuning

### For Real-Time
```toml
compression_level = 0              # No compression
auto_flush_interval_secs = 0       # Manual flush only
```

### Balanced (Recommended)
```toml
compression_level = 6              # Good compression
auto_flush_interval_secs = 30      # Periodic flush
```

### High Compression
```toml
compression_level = 9              # Maximum compression
auto_flush_interval_secs = 300     # Infrequent flush
```

## Supported Arrow Types

| Type | Status | Notes |
|------|--------|-------|
| Float64 | Supported | Direct mapping |
| Int64 | Supported | Direct mapping |
| Int32 | Supported | Direct mapping |
| Utf8 | Supported | Variable-length strings |
| Other | Skipped | Warning logged |

## Key Features

- Full Kameo actor with lifecycle management
- Feature-gated compilation (works without HDF5 library)
- 5 message types for complete control
- Per-instrument data organization
- Configurable compression
- Auto-flush option
- Comprehensive error handling
- Production-ready code quality

## Error Handling

All message handlers return `Result<T>`:
- File operations propagate errors to caller
- Write failures logged but don't crash actor
- Missing features log warnings
- Graceful shutdown with cleanup

## Testing

```bash
# Unit tests (no HDF5 required)
cargo test --lib actors::hdf5_storage

# With HDF5
cargo test --lib actors::hdf5_storage --features v4,storage_hdf5

# Run example
cargo run --example hdf5_storage_example --features v4,storage_hdf5
```

## Troubleshooting

**HDF5 library not found**
```bash
brew install hdf5  # macOS
sudo apt-get install libhdf5-dev  # Linux
```

**Feature not enabled**
```bash
cargo build --features v4,storage_hdf5
```

**Permission denied**
```bash
mkdir -p data && chmod 755 data
```

## Next Steps

1. Review `IMPLEMENTATION_SUMMARY.md` for full details
2. Read `docs/guides/hdf5_storage_guide.md` for configuration
3. Check `examples/hdf5_storage_example.rs` for usage patterns
4. Run tests: `cargo test --lib actors::hdf5_storage`
5. Review `docs/architecture/hdf5_actor_design.md` for design details

## Implementation Status

- Implementation: Complete
- Documentation: Complete
- Examples: Complete
- Tests: Included (3 unit tests)
- Ready for: Integration with DataPublisher

---

**Version**: 1.0
**Date**: November 16, 2025
**Status**: Production Ready
