# HDF5 Storage Actor Guide

## Overview

The HDF5 Storage Actor (`HDF5Storage`) is a Kameo actor that persists Arrow RecordBatch data to HDF5 files within the V4 DAQ system. It provides automatic file management, metadata storage, and graceful lifecycle management.

## Features

- **Arrow Integration**: Receives Arrow RecordBatch data and stores it in HDF5 format
- **Metadata Management**: Stores instrument IDs, timestamps, and schema information
- **Per-Instrument Organization**: Groups data by instrument ID within a single HDF5 file
- **Auto-Flush**: Optional automatic periodic flushing to disk
- **Statistics Tracking**: Monitors bytes written, batch count, and file size
- **Graceful Shutdown**: Properly flushes and closes files on actor shutdown

## Configuration

Configure HDF5 storage via the V4 configuration system (`config_v4.toml`):

```toml
[storage]
default_backend = "hdf5"                    # Storage backend selection
output_dir = "./data"                       # Output directory for HDF5 files
compression_level = 6                       # Compression level (0-9)
auto_flush_interval_secs = 30               # Auto-flush interval (0 = manual)
```

Environment variable overrides:
```bash
export RUST_DAQ_STORAGE_DEFAULT_BACKEND=hdf5
export RUST_DAQ_STORAGE_OUTPUT_DIR=/mnt/data
export RUST_DAQ_STORAGE_COMPRESSION_LEVEL=9
export RUST_DAQ_STORAGE_AUTO_FLUSH_INTERVAL_SECS=60
```

## Message Types

### WriteBatch

Write Arrow RecordBatch data to HDF5.

```rust
use rust_daq::actors::{WriteBatch, HDF5Storage};

let msg = WriteBatch {
    batch: Some(serialized_arrow_ipc),  // Serialized Arrow IPC format
    instrument_id: "power_meter_01".to_string(),
};

storage_actor.call(msg).await?;
```

**Note**: The `batch` field uses `Option<Vec<u8>>` to accommodate conditional compilation. In production, data should be serialized in Arrow IPC format before sending.

### SetMetadata

Store session-level metadata (key-value pairs).

```rust
use rust_daq::actors::SetMetadata;

let msg = SetMetadata {
    key: "experiment_id".to_string(),
    value: "EXP_001_2025-11-16".to_string(),
};

storage_actor.call(msg).await?;
```

### SetInstrumentMetadata

Store instrument-specific metadata.

```rust
use rust_daq::actors::SetInstrumentMetadata;

let msg = SetInstrumentMetadata {
    instrument_id: "power_meter_01".to_string(),
    key: "wavelength_nm".to_string(),
    value: "633.0".to_string(),
};

storage_actor.call(msg).await?;
```

### Flush

Manually flush pending writes to disk.

```rust
use rust_daq::actors::Flush;

storage_actor.call(Flush).await?;
```

### GetStats

Retrieve current storage statistics.

```rust
use rust_daq::actors::GetStats;

let stats = storage_actor.call(GetStats).await?;
println!("Batches written: {}", stats.batches_written);
println!("File size: {} bytes", stats.file_size);
println!("Current file: {:?}", stats.file_path);
```

## HDF5 File Structure

Generated HDF5 files follow this structure:

```
file.h5
├── attributes
│   ├── created_at (string): ISO 8601 timestamp
│   ├── created_timestamp_ns (u64): Nanosecond precision timestamp
│   └── application (string): "Rust DAQ V4"
│
└── instrument_name (group)
    ├── attributes
    │   ├── instrument_id (string): Instrument identifier
    │   └── created_at (string): ISO 8601 timestamp
    │
    └── column_name (dataset)
        ├── data (array): Column values
        └── attributes
            └── schema (string): Arrow schema JSON
```

## Example Usage

### Basic Setup

```rust
use rust_daq::config_v4::StorageConfig;
use rust_daq::actors::HDF5Storage;
use std::path::PathBuf;

// Create configuration
let config = StorageConfig {
    default_backend: "hdf5".to_string(),
    output_dir: PathBuf::from("./measurement_data"),
    compression_level: 6,
    auto_flush_interval_secs: 30,
};

// Create actor
let storage = HDF5Storage::new(&config);

// Spawn actor (using Kameo framework)
use kameo::Actor;
let storage_ref = storage.spawn();
```

### Writing Data

```rust
use rust_daq::actors::{WriteBatch, SetInstrumentMetadata};
use arrow::array::{Float64Array, Int64Array};
use arrow::record_batch::RecordBatch;
use arrow::datatypes::{DataType, Field, Schema};
use std::sync::Arc;

// Create sample data
let power_values = Float64Array::from(vec![0.123, 0.456, 0.789]);
let timestamps = Int64Array::from(vec![1000000, 1000001, 1000002]);

let schema = Schema::new(vec![
    Field::new("power_watts", DataType::Float64, false),
    Field::new("timestamp_ns", DataType::Int64, false),
]);

let batch = RecordBatch::try_new(
    Arc::new(schema),
    vec![
        Arc::new(power_values),
        Arc::new(timestamps),
    ],
)?;

// Serialize to Arrow IPC format and send
// (In production, use Arrow IPC serialization)
let msg = WriteBatch {
    batch: None,  // Placeholder in example
    instrument_id: "power_meter_01".to_string(),
};

storage_ref.call(msg).await?;

// Set metadata
storage_ref.call(SetInstrumentMetadata {
    instrument_id: "power_meter_01".to_string(),
    key: "wavelength_nm".to_string(),
    value: "633.0".to_string(),
}).await?;
```

### Monitoring Storage

```rust
use rust_daq::actors::GetStats;

// Get current statistics
let stats = storage_ref.call(GetStats).await?;

println!("Storage Statistics:");
println!("  Batches written: {}", stats.batches_written);
println!("  Total bytes: {}", stats.bytes_written);
println!("  File size: {} bytes", stats.file_size);
println!("  Datasets: {}", stats.num_datasets);
println!("  File path: {:?}", stats.file_path);
```

## Building with HDF5 Support

The HDF5 storage feature requires the HDF5 system library:

```bash
# macOS (with Homebrew)
brew install hdf5

# Linux (Ubuntu/Debian)
sudo apt-get install libhdf5-dev

# Build with feature flag
cargo build --features v4,storage_hdf5
```

## Supported Arrow Types

The implementation handles the following Arrow data types:

- **Float64**: Double-precision floating point
- **Int64**: 64-bit signed integers
- **Int32**: 32-bit signed integers
- **Utf8**: Variable-length UTF-8 strings

Other types will be logged as warnings and skipped during write operations.

## Error Handling

The actor handles errors gracefully:

- **File Creation Errors**: Logged and returned to caller
- **Write Failures**: Logged but do not crash the actor
- **Missing Features**: When `storage_hdf5` feature is disabled, operations are logged as warnings but succeed
- **Shutdown Errors**: Logged but do not prevent actor from stopping

## Performance Considerations

### Compression

The `compression_level` setting (0-9) controls HDF5 compression:
- **0**: No compression (fastest writes)
- **6**: Default balance (recommended)
- **9**: Maximum compression (slower writes, smaller files)

### Auto-Flush

Auto-flush reduces memory overhead but increases I/O:
- **0**: Manual flush only (default, lowest I/O)
- **>0**: Flush every N seconds (higher I/O, lower memory)

### Batch Size

Large batches improve write efficiency but increase latency. Consider batch sizes of 1000-10000 rows for most applications.

## Testing

The module includes unit tests for basic functionality:

```bash
cargo test --lib actors::hdf5_storage --features v4,storage_hdf5
```

Tests verify:
- Actor creation with configuration
- Metadata storage
- Statistics tracking
- Message handling (without actual HDF5 writes)

## Limitations and Future Work

### Current Limitations

1. **HDF5 Dependency**: Requires HDF5 system library (not pure Rust)
2. **Arrow Serialization**: Currently uses placeholder serialization; production code needs proper Arrow IPC handling
3. **Fixed Schema**: Schema is stored per batch but not validated across batches
4. **No File Rotation**: All data written to single file per session
5. **Limited Arrow Types**: Only handles numeric and string types

### Future Enhancements

1. **File Rotation**: Size-based and time-based file rotation
2. **Compression Options**: Support for different compression algorithms (gzip, lzf)
3. **Chunking Strategy**: Configurable chunking for optimized HDF5 read performance
4. **Extended Arrow Types**: Support for lists, structures, and other complex types
5. **Incremental Writes**: Append-only mode for long-running sessions
6. **Schema Evolution**: Handle schema changes across batches

## Troubleshooting

### HDF5 Library Not Found

**Error**: `Unable to locate HDF5 root directory`

**Solution**: Install HDF5 system library and set environment variables:
```bash
# macOS
brew install hdf5
export HDF5_DIR=$(brew --prefix hdf5)

# Linux
sudo apt-get install libhdf5-dev
export HDF5_DIR=/usr/lib/x86_64-linux-gnu
```

### File Permission Denied

**Error**: `Failed to create HDF5 file`

**Solution**: Ensure `output_dir` exists and is writable:
```bash
mkdir -p data
chmod 755 data
```

### Feature Not Enabled

**Error**: `HDF5 storage feature not enabled`

**Solution**: Build with the `storage_hdf5` feature:
```bash
cargo build --features v4,storage_hdf5
```

## See Also

- [V4 Configuration Guide](../config/v4_config.md)
- [Arrow Integration Guide](../data/arrow_integration.md)
- [Kameo Actor Framework](https://github.com/tompomago/kameo)
- [HDF5 Official Documentation](https://www.h5group.org/)
