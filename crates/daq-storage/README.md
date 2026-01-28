# daq-storage

High-throughput data storage and buffering infrastructure for rust-daq.

## Overview

This crate provides the storage layer for rust-daq, handling:

- **Ring Buffers**: Memory-mapped circular buffers for high-speed frame streaming
- **Format Writers**: HDF5, Arrow/Parquet, Zarr V3, and TIFF output
- **Cross-Process Access**: Python and Julia can read ring buffers via mmap

## Features

| Feature | Description | Default |
|---------|-------------|---------|
| `storage_hdf5` | HDF5 file output with compression | No |
| `storage_arrow` | Arrow IPC and Parquet output | No |
| `storage_parquet` | Parquet columnar format | No |
| `storage_tiff` | TIFF image stacks | No |
| `storage_zarr` | Zarr V3 chunked arrays | No |

## Key Components

### RingBuffer

Memory-mapped circular buffer optimized for high-FPS camera streaming:

```rust,ignore
use daq_storage::ring_buffer::RingBuffer;
use std::path::Path;

// Create a ring buffer with 100 frame slots
let buffer = RingBuffer::create(Path::new("/tmp/daq_ring"), 100)?;

// Write frames (from camera callback)
buffer.write(&frame_bytes)?;

// Read frames (for storage or visualization)
let frame = buffer.read_latest()?;
```

**Features:**
- Lock-free SeqLock pattern for concurrent access
- Cross-process readable via mmap (Python, Julia)
- Automatic wrap-around with sequence tracking
- Data taps for live visualization

### Storage Writers

#### HDF5 Writer
```rust,ignore
use daq_storage::hdf5_writer::Hdf5Writer;

let writer = Hdf5Writer::create("experiment.h5")?;
writer.write_frame("camera_1", &frame)?;
writer.write_metadata("exposure_ms", 100.0)?;
writer.flush()?;
```

#### Arrow/Parquet Writer
```rust,ignore
use daq_storage::arrow_writer::ArrowWriter;

let writer = ArrowWriter::new("data.parquet")?;
writer.append_batch(&record_batch)?;
writer.finish()?;
```

#### Zarr V3 Writer
```rust,ignore
use daq_storage::zarr_writer::ZarrWriter;

let writer = ZarrWriter::create("experiment.zarr", shape, chunks)?;
writer.write_chunk(coords, &data)?;
```

#### TIFF Writer
```rust,ignore
use daq_storage::tiff_writer::TiffWriter;

let writer = TiffWriter::create("stack.tiff")?;
writer.write_frame(&frame)?;  // Appends to stack
writer.finish()?;
```

### Ring Buffer Reader

For external consumers (Python, visualization):

```rust,ignore
use daq_storage::ring_buffer_reader::RingBufferReader;

let reader = RingBufferReader::open("/tmp/daq_ring")?;
while let Some(frame) = reader.read_next()? {
    process_frame(&frame);
}
```

## Architecture

```
Camera Frame → Pool<FrameData> → RingBuffer (mmap)
                                      │
                    ┌─────────────────┼─────────────────┐
                    ▼                 ▼                 ▼
              HDF5 Writer      Arrow Writer      Python Reader
              (archival)       (analysis)        (real-time viz)
```

## Cross-Language Access

The ring buffer uses a memory-mapped file with a well-defined header format,
allowing Python and other languages to read frames directly:

```python
# Python example
import numpy as np
import mmap

with open("/tmp/daq_ring", "r+b") as f:
    mm = mmap.mmap(f.fileno(), 0)
    # Read header, get latest frame...
```

See the Python client library (`clients/python/`) for a full implementation.

## Performance

- **Ring Buffer Write**: < 10µs per frame (640x480x16bit)
- **Ring Buffer Read**: < 5µs per frame
- **HDF5 Write**: ~100µs per frame with compression
- **Arrow Write**: ~50µs per batch

## Configuration

Storage paths and buffer sizes are configured via TOML:

```toml
[storage]
ring_buffer_path = "/dev/shm/daq_ring"
ring_buffer_frames = 1000
hdf5_compression = "gzip"
hdf5_compression_level = 4
```

## Examples

See `examples/` in the workspace root:

- `ring_buffer_demo.rs` - Ring buffer creation and access
- `hdf5_storage_example.rs` - HDF5 file writing
- `ring_arrow_bench.rs` - Arrow format benchmarking (requires `storage_arrow`)

## Dependencies

- `memmap2` - Memory-mapped file access
- `hdf5-metno` - HDF5 bindings (optional)
- `arrow` / `parquet` - Arrow ecosystem (optional)
- `zarrs` - Zarr V3 implementation (optional)
- `image` - TIFF encoding (optional)

## See Also

- [`common`](../common/) - Frame types and core abstractions
- [`daq-pool`](../daq-pool/) - Zero-allocation frame pooling
- [`daq-experiment`](../daq-experiment/) - Experiment orchestration
