# Zarr V3 Acquisition Guide

Zarr V3 provides cloud-native N-dimensional array storage for scientific data, replacing HDF5 as the primary format for nested multi-dimensional scans. Data is stored in a human-inspectable directory structure and is fully compatible with Python's Xarray ecosystem.

## Quick Start

```rust
use daq_storage::zarr_writer::ZarrWriter;
use std::path::Path;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create store
    let writer = ZarrWriter::new(Path::new("experiment.zarr")).await?;

    // Define 4D array for nested scan
    writer.create_array()
        .name("intensity")
        .shape(vec![10, 5, 256, 256])  // wavelengths, positions, y, x
        .chunks(vec![1, 1, 256, 256])   // One frame per chunk
        .dimensions(vec!["wavelength", "position", "y", "x"])
        .dtype_u16()
        .build()
        .await?;

    // Nested scan loop
    for (wl_idx, wavelength) in wavelengths.iter().enumerate() {
        laser.set_wavelength(*wavelength).await?;

        for (pos_idx, position) in positions.iter().enumerate() {
            stage.move_to(*position).await?;

            let frame = camera.capture().await?;
            writer.write_chunk::<u16>(
                "intensity",
                &[wl_idx as u64, pos_idx as u64, 0, 0],
                frame.data.to_vec(),
            ).await?;
        }
    }

    Ok(())
}
```

## Reading in Python

Once data is written, analyze it with Xarray:

```python
import xarray as xr
import numpy as np

# Open store as dataset
ds = xr.open_zarr("experiment.zarr")
print(ds)
# <xarray.Dataset>
# Dimensions:    (wavelength: 10, position: 5, y: 256, x: 256)

# Select wavelength and plot intensity map
intensity_map = ds.intensity.sel(wavelength=750, position=0).values
plt.imshow(intensity_map)

# Compute spectral statistics
spectrum = ds.intensity.mean(dim=["y", "x"])  # Average over image
spectrum.plot(x="wavelength")
```

## Store Structure

Zarr stores are human-readable directories:

```
experiment.zarr/
├── zarr.json          # Root group metadata
└── intensity/         # Array directory
    ├── zarr.json      # Array metadata + _ARRAY_DIMENSIONS
    └── c/             # Chunks directory
        ├── 0/0/0/0    # Chunk [wl=0, pos=0, y=0, x=0]
        ├── 0/0/1/0    # Chunk [wl=0, pos=0, y=1, x=0]
        └── ...
```

This structure enables:
- Direct inspection with file browser
- Parallel chunk uploads to cloud storage
- Streaming append without full file rewrite

## Creating Arrays

### Basic Array

```rust
writer.create_array()
    .name("signal")
    .shape(vec![100])
    .chunks(vec![10])
    .dimensions(vec!["time"])
    .dtype_f64()
    .build()
    .await?;
```

### 2D Camera Frame

```rust
writer.create_array()
    .name("camera_frame")
    .shape(vec![1024, 1024])
    .chunks(vec![1024, 1024])  // Single chunk for full frame
    .dimensions(vec!["y", "x"])
    .dtype_u16()
    .attribute("units", json!("counts"))
    .build()
    .await?;
```

### 3D Spectral Volume

```rust
writer.create_array()
    .name("spectrum")
    .shape(vec![1000, 512, 512])  // wavelengths x height x width
    .chunks(vec![10, 512, 512])   // One wavelength per chunk
    .dimensions(vec!["wavelength", "y", "x"])
    .dtype_u16()
    .attribute("wavelength_range", json!("700-900nm"))
    .build()
    .await?;
```

### 4D Nested Scan (Typical)

```rust
// Most common: wavelength scan + position scan + 2D imaging
writer.create_array()
    .name("scan_data")
    .shape(vec![10, 5, 256, 256])
    .chunks(vec![1, 1, 256, 256])    // One frame per chunk
    .dimensions(vec!["wavelength", "position", "y", "x"])
    .dtype_u16()
    .build()
    .await?;
```

## Data Types

Use the dtype methods to match your hardware:

| Method | Type | Use Case |
|--------|------|----------|
| `dtype_u8()` | Unsigned 8-bit | Legacy 8-bit cameras (rare) |
| `dtype_u16()` | Unsigned 16-bit | **Most cameras (PVCAM, etc.)** |
| `dtype_u32()` | Unsigned 32-bit | High-dynamic-range hardware |
| `dtype_f32()` | 32-bit float | Processed/normalized data |
| `dtype_f64()` | 64-bit float | High-precision measurements |
| `dtype_i16()` | Signed 16-bit | Differential signals |
| `dtype_i32()` | Signed 32-bit | Large range signals |

```rust
// Camera data (typical)
.dtype_u16()

// Power meter readings
.dtype_f64()

// Processed spectral data
.dtype_f32()
```

## Chunking Strategy

Chunks determine I/O granularity. Choose based on access patterns:

### Single Frames (Streaming)

Access pattern: Capture frames sequentially during acquisition.

```rust
// Shape: [wavelengths=10, positions=5, y=256, x=256]
// Write: One frame at a time
writer.create_array()
    .shape(vec![10, 5, 256, 256])
    .chunks(vec![1, 1, 256, 256])  // One frame per chunk
    .build()
    .await?;

// Writing loop
for wl in 0..10 {
    for pos in 0..5 {
        writer.write_chunk::<u16>(
            "data",
            &[wl, pos, 0, 0],
            frame.data
        ).await?;
    }
}
```

### Time Series at Pixel

Access pattern: Extract time series at each spatial location.

```rust
// Shape: [time=1000, y=256, x=256]
// Access: data[t, y, x] for each (y, x)
writer.create_array()
    .shape(vec![1000, 256, 256])
    .chunks(vec![10, 1, 1])  // 10 timepoints per chunk
    .build()
    .await?;
```

### Spectral Analysis

Access pattern: Extract full spectrum at each wavelength.

```rust
// Shape: [wavelengths=100, positions=5, y=256, x=256]
// Access: All wavelengths for each position
writer.create_array()
    .shape(vec![100, 5, 256, 256])
    .chunks(vec![100, 1, 256, 256])  // All wavelengths per chunk
    .build()
    .await?;
```

### Guidelines

- **Chunk size target:** 10-100 MB for optimal balance
- **Write order:** Chunk indices must be in order (0,0), (0,1), (1,0), etc.
- **Match patterns:** Align chunks to how data will be read
- **Avoid overcaching:** Don't make chunks larger than available memory

## Writing Chunks

### Basic Write

```rust
let frame_data: Vec<u16> = vec![/* 256 * 256 values */];
writer.write_chunk::<u16>(
    "intensity",
    &[0, 0, 0, 0],      // Chunk indices
    frame_data
).await?;
```

### Batch Writes

```rust
let mut indices = vec![0u64; 4];

for frame_idx in 0..100 {
    let frame = camera.capture().await?;

    // Update chunk index (only need to increment what changes)
    indices[0] = (frame_idx / 25) as u64;  // wavelength
    indices[1] = (frame_idx % 25) as u64;  // position
    indices[2] = 0;
    indices[3] = 0;

    writer.write_chunk::<u16>(
        "intensity",
        &indices,
        frame.data.to_vec()
    ).await?;
}
```

### Type Safety

Data type must match array dtype:

```rust
// Array created with dtype_f64()
let data: Vec<f64> = vec![1.0, 2.5, 3.7];
writer.write_chunk::<f64>("signal", &[0], data).await?;

// Type mismatch - ERROR
let wrong_data: Vec<u16> = vec![1, 2, 3];
writer.write_chunk::<u16>("signal", &[0], wrong_data).await?;
// Error: array 'signal' has type Float64, cannot write UInt16
```

## Adding Metadata

### Coordinate Arrays

Store physical coordinate values alongside raw indices:

```rust
// Write wavelength coordinates
writer.create_array()
    .name("wavelengths")
    .shape(vec![10])
    .chunks(vec![10])
    .dimensions(vec!["wavelength"])
    .dtype_f64()
    .attribute("units", json!("nm"))
    .build()
    .await?;

let wl_coords: Vec<f64> = (700..800).step_by(10).map(|x| x as f64).collect();
writer.write_chunk::<f64>("wavelengths", &[0], wl_coords).await?;
```

Then in Python:

```python
import xarray as xr

ds = xr.open_zarr("experiment.zarr")

# Dimension still uses indices by default
print(ds.intensity.shape)  # (10, 5, 256, 256)

# But you can assign coordinates manually
ds = ds.assign_coords(
    wavelength=("wavelength", ds.wavelengths.values)
)

# Now select by wavelength value instead of index
ds.intensity.sel(wavelength=750.0)
```

### Experiment Metadata

Store experiment-level attributes:

```rust
writer.add_group_attribute("experiment_id", json!("EXP-2026-001")).await?;
writer.add_group_attribute("created_at", json!("2026-01-27T14:30:00Z")).await?;
writer.add_group_attribute("scanner_id", json!("maitai-001")).await?;
```

### Array Attributes

Add per-array metadata:

```rust
writer.create_array()
    .name("camera_frames")
    .shape(vec![10, 5, 256, 256])
    .chunks(vec![1, 1, 256, 256])
    .dimensions(vec!["wavelength", "position", "y", "x"])
    .dtype_u16()
    .attribute("camera_id", json!("prime_bsi_001"))
    .attribute("pixel_size", json!("6.5e-6"))  // meters
    .attribute("units", json!("counts"))
    .attribute("bit_depth", json!(16))
    .build()
    .await?;
```

## Performance Tips

### Async I/O

ZarrWriter uses `tokio::task::spawn_blocking` for all I/O operations. This prevents blocking the async runtime:

```rust
// These calls are async but don't block tokio
writer.create_array().name("data")...build().await?;
writer.write_chunk::<u16>("data", &[0, 0], frame).await?;
```

### Chunk Size Tuning

- **Too small** (<1 MB): Excessive metadata overhead, slow I/O
- **Too large** (>500 MB): Memory pressure, longer I/O latency
- **Optimal** (10-100 MB): Good balance

Example tuning:

```rust
// For 256x256 u16 frames
// Frame size: 256 * 256 * 2 bytes = 131 KB
// Target 50 MB chunks: 50 MB / 131 KB ≈ 380 frames per chunk

let frames_per_chunk = 380;
let chunk_height = ((frames_per_chunk as f64).sqrt()) as u64;

writer.create_array()
    .shape(vec![wavelengths, positions, 256, 256])
    .chunks(vec![1, 1, chunk_height, chunk_height])
    .build()
    .await?;
```

### Memory Efficiency

Keep data in flight minimal:

```rust
// GOOD: Process and write immediately
for frame in camera.stream() {
    let data = frame.process()?;
    writer.write_chunk::<u16>("data", &indices, data).await?;
}

// AVOID: Buffering many frames
let mut buffer = Vec::new();
for frame in camera.stream().take(100) {
    buffer.push(frame);  // Large memory overhead
}
for (idx, frame) in buffer.drain(..) {
    writer.write_chunk::<u16>("data", &idx, frame).await?;
}
```

## File Structure Reference

### Root Group Metadata

`experiment.zarr/zarr.json`:

```json
{
  "zarr_format": 3,
  "kind": "group",
  "attributes": {
    "experiment_id": "EXP-2026-001",
    "created_at": "2026-01-27T14:30:00Z"
  }
}
```

### Array Metadata

`experiment.zarr/intensity/zarr.json`:

```json
{
  "zarr_format": 3,
  "kind": "array",
  "attributes": {
    "_ARRAY_DIMENSIONS": ["wavelength", "position", "y", "x"],
    "units": "counts"
  },
  "shape": [10, 5, 256, 256],
  "chunk_grid": {
    "configuration": {
      "chunk_shape": [1, 1, 256, 256]
    },
    "name": "regular"
  },
  "data_type": "<u2",
  "fill_value": 0,
  "codecs": [
    {
      "name": "bytes",
      "configuration": {"endian": "little"}
    }
  ]
}
```

The `_ARRAY_DIMENSIONS` attribute enables Xarray to automatically recognize the dimensional structure.

## Error Handling

### Common Errors

```rust
// Missing array
writer.write_chunk::<u16>("nonexistent", &[0], data).await?;
// Error: Array 'nonexistent' not found

// Type mismatch
writer.create_array().name("data").dtype_f64().build().await?;
writer.write_chunk::<u16>("data", &[0], data).await?;
// Error: array 'data' has type Float64, cannot write UInt16

// Index out of bounds
writer.create_array().shape(vec![10]).chunks(vec![10]).build().await?;
writer.write_chunk::<u16>("data", &[5], data).await?;  // OK
writer.write_chunk::<u16>("data", &[10], data).await?; // Error: index out of bounds

// Dimension mismatch
writer.create_array()
    .shape(vec![10, 10])
    .chunks(vec![10])  // Only 1 dimension
    .build().await?;
// Error: Shape dimensions (2) must match chunk dimensions (1)
```

### Recovery

Most errors are fatal for a chunk (disk I/O errors, permission issues). Best practice:

```rust
for (wl_idx, wavelength) in wavelengths.iter().enumerate() {
    laser.set_wavelength(*wavelength).await?;

    for (pos_idx, position) in positions.iter().enumerate() {
        stage.move_to(*position).await?;
        let frame = camera.capture().await?;

        match writer.write_chunk::<u16>(
            "intensity",
            &[wl_idx as u64, pos_idx as u64, 0, 0],
            frame.data.to_vec()
        ).await {
            Ok(_) => println!("Wrote chunk [{}, {}]", wl_idx, pos_idx),
            Err(e) => {
                eprintln!("Failed to write chunk: {}", e);
                // Decide: retry, skip, or abort
                return Err(e);
            }
        }
    }
}
```

## Comparison: Zarr vs HDF5

| Feature | Zarr | HDF5 |
|---------|------|------|
| **Format** | Cloud-native, hierarchical | Traditional binary |
| **File Structure** | Directory of chunks | Single file |
| **Access** | Individual chunks | Full dataset needed |
| **Python** | Xarray native | h5py wrapper |
| **Parallel Writes** | Safe | Risky |
| **Cloud Storage** | Optimized | Not designed |
| **Human Readable** | Yes (JSON metadata) | No |
| **Append** | Efficient | Full rewrite |

**Use Zarr for:**
- Cloud-native workflows
- Parallel acquisition
- Large datasets
- Streaming data

**Use HDF5 for:**
- Legacy compatibility
- Complex hierarchies
- Single-file simplicity

## See Also

- [Zarr Specification](https://zarr-specs.readthedocs.io/en/latest/v3/core/v3.0.0.html)
- [Xarray Documentation](http://xarray.pydata.org/)
- [rust-daq storage module](../../crates/daq-storage/README.md)
