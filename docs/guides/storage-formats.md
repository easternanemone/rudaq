# Storage Format Selection Guide

This guide helps you choose the right storage format for your experiments and configure writers for optimal performance.

## Quick Reference: Format Comparison

| Feature | HDF5 | Arrow IPC | Parquet | CSV | Zarr |
|---------|------|-----------|---------|-----|------|
| **Use Case** | Complex nested data, multi-device | Streaming, IPC, real-time | Analytics, post-processing | Simple tables, Excel | Cloud storage, N-dimensional arrays |
| **Structure** | Hierarchical (groups/datasets) | Columnar, self-describing | Columnar, compressed | Flat rows | Chunked N-dimensional |
| **Compression** | Yes (gzip, lz4) | None (fast streaming) | Yes (snappy, gzip) | No | Yes (built-in) |
| **Read Speed** | Medium (seek overhead) | Fast (memory-mapped) | Fast (columnar) | Slow (text parsing) | Fast (chunk access) |
| **Write Speed** | Medium (serialization) | Fast (direct bytes) | Slow (compression) | Very slow (text) | Medium (chunks) |
| **File Size** | Small (compressed) | Large (uncompressed) | Small (compressed) | Very large (text) | Medium (variable) |
| **Metadata Support** | Excellent (attributes) | Good (schemas) | Good (metadata) | Limited | Good (attributes) |
| **Parallelism** | Limited | Single-writer | Read-parallel | Limited | Read/write-parallel |
| **Cloud-friendly** | No | No | Yes | Yes | Yes |
| **Python/MATLAB Support** | Excellent | Good (via Pandas) | Excellent | Excellent | Excellent (Xarray) |

## When to Use Each Format

### HDF5: Complex Nested Experiments

**Best for:**
- Multi-device, multi-dataset experiments
- Hierarchical data (nested scans, sub-experiments)
- Experiments combining images + scalar measurements
- Archive storage for long-term preservation
- Integration with MATLAB/Igor Pro

**Example:** Wavelength-dependent imaging scan with power meter logging

```toml
[storage]
format = "hdf5"
compression = "gzip"     # 9 for maximum compression
output_path = "experiment_data.h5"
```

**Rust Example:**
```rust
use daq_storage::{HDF5Writer, RingBuffer};
use std::sync::Arc;
use std::path::Path;

let ring = Arc::new(RingBuffer::create(Path::new("/dev/shm/daq_ring"), 100)?);
let writer = HDF5Writer::new(Path::new("data.h5"), ring)?;

tokio::spawn(async move {
    writer.run().await.expect("Writer failed");
});
```

### Arrow IPC: Real-Time Streaming

**Best for:**
- Live data monitoring
- Inter-process communication (gRPC)
- Integration with Pandas/Polars
- Streaming from multiple sources
- When write speed is critical (>10k events/sec)

**Example:** Real-time optical power measurement

```rust
use daq_storage::arrow_writer::ArrowDocumentWriter;

let writer = ArrowDocumentWriter::new(
    Path::new("live_data.arrow"),
    1024,  // buffer size
)?;
```

**Characteristics:**
- No compression overhead
- Self-describing schema (Arrow IPC format)
- Fast writes to tmpfs or SSD
- Ideal for temporary data during acquisition

### Parquet: Post-Processing & Analytics

**Best for:**
- Data you'll analyze with pandas/polars
- Machine learning pipelines
- Sharing datasets with collaborators
- Cloud storage (S3, Azure Blob)
- When file size matters

**Example:** Save experiment results for ML analysis

```rust
use daq_storage::arrow_writer::ParquetDocumentWriter;

let writer = ParquetDocumentWriter::new(
    Path::new("results.parquet"),
    1024,
)?;
```

**Advantages:**
- Built-in compression (snappy/gzip)
- Column-oriented (fast filtering/aggregation)
- Excellent pandas/Polars integration
- Cloud-optimized reading

### CSV: Maximum Interoperability

**Best for:**
- Simple 2D data (time series, measurements)
- Sharing with non-scientists (Excel)
- Quick exploratory exports
- When human readability matters
- Legacy system integration

**Not recommended for:**
- Image data (use Arrow, HDF5, or Zarr)
- Large datasets (text overhead)
- Performance-critical acquisition
- Nested or complex structures

### Zarr: Cloud-Native N-Dimensional Arrays

**Best for:**
- Large multi-dimensional scans (4D+)
- Cloud storage (S3, Azure, GCS)
- Parallel/distributed reading
- Xarray workflows
- Complex hierarchical data

**Example:** 4D nested scan (wavelength × position × y × x)

```rust
use daq_storage::zarr_writer::ZarrWriter;

let writer = ZarrWriter::new(Path::new("experiment.zarr")).await?;

// Create 4D array for nested scan
writer.create_array()
    .name("camera_frames")
    .shape(vec![10, 5, 256, 256])  // wl, pos, y, x
    .chunks(vec![10, 1, 256, 256])  // Full wl, one position
    .dimensions(vec!["wavelength", "position", "y", "x"])
    .dtype_u16()
    .build()
    .await?;

// Write chunks efficiently
writer.write_chunk::<u16>(
    "camera_frames",
    &[0, 0, 0, 0],  // Start at [wl=0, pos=0, y=0, x=0]
    frame_data,
).await?;
```

**Python Analysis:**
```python
import xarray as xr
ds = xr.open_zarr("experiment.zarr")
# Dimensions automatically recognized
print(ds.camera_frames)  # Access as named dimensions
```

## Ring Buffer Configuration

The ring buffer is the high-speed in-memory store that feeds writers. Configure it based on:

1. **Data rate** (bytes/sec)
2. **Desired buffering duration** (seconds)
3. **Available system memory**

### Buffer Sizing Formula

```
buffer_size_bytes = data_rate_bytes_per_sec * desired_duration_seconds
```

**Examples:**

| Experiment | Data Rate | Duration | Buffer Size |
|-----------|-----------|----------|------------|
| Single camera (10 fps) | 400 MB/s | 5 sec | 2 GB |
| Multi-device (10 DAQ ch) | 1.2 MB/s | 60 sec | 72 MB |
| Streaming sensor | 10 MB/s | 10 sec | 100 MB |

### Memory vs Performance Tradeoff

```
Larger Buffer          Smaller Buffer
- Less writer lag     - Lower memory usage
- Smoother writes     - Tighter latency requirements
- Tolerate I/O pauses - Must flush frequently
```

### Configuration Example

```rust
use daq_storage::RingBuffer;
use std::path::Path;

// 500 MB buffer for smooth streaming
let ring = RingBuffer::create(
    Path::new("/dev/shm/daq_ring"),
    500 * 1024 * 1024,  // bytes
)?;

// Spawn background writer
let writer = HDF5Writer::new(
    Path::new("data.h5"),
    ring.clone(),
)?;

tokio::spawn(async move {
    writer.run().await
});
```

### Overflow Modes

When the buffer fills and new data arrives:

- **Discard oldest** (default): Drop old frames, keep acquisition running
- **Block**: Pause acquisition (clean but slow)
- **Resize** (rarely available): Expand buffer dynamically

Check your ring buffer implementation for which modes are supported.

## Writer Configuration Examples

### HDF5 with Compression

```toml
[storage]
# Output file
output_path = "experiment_data.h5"

# Compression algorithm: none, gzip(0-9), lz4
compression = "gzip"
compression_level = 5  # 0=off, 9=maximum

# Chunking strategy (None uses defaults)
chunk_size = 1024  # samples per chunk

# Buffer flushing
flush_interval_ms = 1000  # Write to disk every 1 second

[ring_buffer]
size_mb = 500
```

**Rust API:**
```rust
use daq_storage::{ComediStreamWriter, ChannelConfig, CompressionType};

let channels = vec![
    ChannelConfig::new(0, "power_meter", -0.1, 10.0),
    ChannelConfig::new(1, "detector", 0.0, 5.0),
];

let writer = ComediStreamWriter::builder()
    .output_path(Path::new("data.h5"))
    .channels(channels)
    .sample_rate(10000.0)
    .compression(CompressionType::Gzip)
    .build()?;
```

### Arrow IPC for Real-Time

```rust
use daq_storage::arrow_writer::ArrowDocumentWriter;

// Fast write, no compression
let writer = ArrowDocumentWriter::new(
    Path::new("/dev/shm/live_stream.arrow"),
    8192,  // Record batch size
)?;

// Receive documents from RunEngine
writer.consume_document(start_doc)?;
writer.consume_document(descriptor_doc)?;
writer.consume_document(event_doc)?;
writer.consume_document(stop_doc)?;
```

### Multi-Format Simultaneous Write

```rust
use daq_storage::comedi_writer::{
    ComediStreamWriter, StorageFormat, ChannelConfig,
};

let channels = vec![
    ChannelConfig::new(0, "AI0", -10.0, 10.0),
];

let writer = ComediStreamWriter::builder()
    .output_path(Path::new("data.h5"))
    .channels(channels)
    .storage_format(StorageFormat::Both)  // Write HDF5 AND Arrow
    .sample_rate(100000.0)
    .build()?;

// Simultaneously writes to:
// - data.h5 (HDF5)
// - data.arrow (Arrow IPC)
```

### CSV Export (Simple Time Series)

```python
# After experiment completes, export to CSV
import h5py
import pandas as pd

with h5py.File("data.h5", "r") as f:
    times = f["timestamps"][:]
    values = f["power_meter"][:]

df = pd.DataFrame({
    "timestamp": times,
    "power_mW": values * 1000  # Convert to mW
})
df.to_csv("data.csv", index=False)
```

## Performance Considerations

### Write Throughput

Measured on modern hardware (NVMe SSD, 16 GB RAM):

| Format | Throughput | Notes |
|--------|-----------|-------|
| HDF5 (gzip) | 50-100 MB/s | Depends on compression level |
| HDF5 (lz4) | 200-400 MB/s | Faster but larger files |
| Arrow IPC | 500-1000 MB/s | Direct memory writes |
| Parquet | 20-50 MB/s | Compression adds latency |
| CSV | 5-20 MB/s | Text serialization overhead |
| Zarr | 100-300 MB/s | Chunk-dependent |

### Memory Overhead per Writer

| Format | Base Memory | Per-Channel | Notes |
|--------|-----------|------------|-------|
| HDF5 | 10 MB | 1 MB | Metadata, open datasets |
| Arrow | 20 MB | 5 MB | Schema, buffer cache |
| Zarr | 15 MB | 2 MB | Chunk metadata |
| CSV | 5 MB | 0.5 MB | Minimal buffering |

### Disk Space Examples (1 hour continuous data)

All numbers assume 8-bit grayscale camera (640×480@30fps):

- **Raw binary**: 5.1 GB (baseline)
- **HDF5 (gzip)**: 1.2 GB (75% reduction)
- **HDF5 (lz4)**: 2.1 GB (60% reduction)
- **Arrow**: 5.1 GB (no compression)
- **Parquet**: 1.0 GB (80% reduction)
- **CSV**: Not practical (>100 GB)

## Integration with Rhai Scripts

Writers are typically used as background tasks spawned from the daemon, but you can trigger acquisitions from scripts:

### Basic Rhai Pattern

```rhai
// scripts/image_with_storage.rhai
let camera = create_camera("camera0");

// Configure frame acquisition
camera.set_trigger_mode("software");
camera.set_exposure(1.0);  // milliseconds

// Acquire 100 frames (writer saves in background)
for i in range(0, 100) {
    let frame = camera.grab_frame();
    print(`Acquired frame ${i}: ${frame.width}x${frame.height}`);
}

// Writer continues saving in background
print("Done - writer will flush remaining data");
```

### Coordinated Multi-Device

```rhai
// Acquire while scanning wavelength
let laser = create_laser("maitai");
let camera = create_camera("prime_bsi");
let powerMeter = create_power_meter("power0");

let start_wl = 700;
let end_wl = 1000;
let step_wl = 10;

for wl in range(start_wl, end_wl, step_wl) {
    laser.set_wavelength(wl);
    sleep(0.5);  // Settle time

    let frame = camera.grab_frame();
    let power = powerMeter.read_value();

    print(`λ=${wl}nm, P=${power}mW, Frame=${frame.width}x${frame.height}`);
}
```

### Error Handling

```rhai
// Graceful handling of acquisition failures
try {
    let frame = camera.grab_frame();
} catch(e) {
    print(`Frame grab failed: ${e}`);
    // Writer will stop cleanly on script exit
}
```

## Choosing a Format: Decision Tree

```
START: "What are you storing?"
  ├─ Images only?
  │  ├─ Yes, need compression? HDF5
  │  └─ Yes, cloud access? Zarr
  │
  ├─ Time series + scalars?
  │  ├─ Need speed (>1k Hz)? Arrow IPC
  │  ├─ Post-processing? Parquet
  │  └─ Simple/Excel? CSV
  │
  ├─ Complex nested data (4D+)?
  │  ├─ Local storage? HDF5 or Zarr
  │  └─ Cloud? Zarr
  │
  └─ Multi-format safety?
     └─ Use Both (HDF5 + Arrow)
```

## File Naming Conventions

Recommended naming pattern:

```
{experiment}_{date}_{time}_{version}.{ext}

Examples:
- wavelength_scan_2026-01-25_14-32-45_v1.h5
- polarization_analysis_2026-01-25_14-32-45_v2.parquet
- live_detector_2026-01-25_14-32-45.arrow
```

Rust implementation:
```rust
use chrono::Local;

let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
let filename = format!(
    "data/wavelength_scan_{}_{}.h5",
    timestamp, 1
);
```

## Metadata Best Practices

Always include experiment metadata:

**HDF5 attributes:**
```rust
// Stored in file
hdf5_file.new_attr("experiment")?
    .emit_scalar("wavelength_scan")?;
hdf5_file.new_attr("date")?
    .emit_scalar("2026-01-25")?;
hdf5_file.new_attr("instrument")?
    .emit_scalar("Prime BSI Camera")?;
```

**Arrow schema:**
```python
# Python reading
import pyarrow.parquet as pq

table = pq.read_table("data.parquet")
print(table.schema.metadata)  # Contains experiment info
```

## Related Documentation

- [Architecture: Storage & Streaming](../architecture/adr-storage-streaming.md)
- [daq-storage Crate Reference](../../crates/daq-storage/README.md)
- [Rhai Scripting Guide](./scripting.md)
- [Hardware Integration Tests](./testing.md)

## Troubleshooting

**Issue: "Buffer full" errors**
- Solution: Increase ring buffer size or reduce data rate

**Issue: Writer consuming too much memory**
- Solution: Reduce chunk size or flush interval

**Issue: CSV file too large**
- Solution: Use HDF5 or Zarr with compression

**Issue: Can't read Parquet in MATLAB**
- Solution: Use HDF5 instead (better MATLAB support)

## Examples Repository

Complete working examples in `examples/storage/`:
```
examples/storage/
├── hdf5_imaging.rs      # Camera streaming to HDF5
├── arrow_realtime.rs    # Real-time monitoring
├── zarr_nested_scan.rs  # 4D array storage
└── csv_export.py        # Post-processing export
```
