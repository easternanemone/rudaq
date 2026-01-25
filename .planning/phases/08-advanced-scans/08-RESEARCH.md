# Phase 8: Advanced Scans - Research

**Researched:** 2026-01-25
**Domain:** N-dimensional scientific data storage (Zarr V3), adaptive scan algorithms, nested loop translation
**Confidence:** HIGH

## Summary

Phase 8 delivers nested multi-dimensional scans and adaptive scans responding to acquired data. The critical technical decision is migrating from HDF5 to **Zarr V3** for N-dimensional array storage.

**Key findings:**
- Zarr V3 is the modern standard for cloud-native scientific data storage with better Rust support than HDF5
- `zarrs` crate (v0.22.10) provides first-class Zarr V3 support with object_store integration for cloud backends
- Xarray compatibility requires `_ARRAY_DIMENSIONS` attribute in Zarr V2 or native `dimension_names` in V3
- Peak detection via `find_peaks` crate (v0.1.5) provides scipy-equivalent functionality
- egui native Modal containers now available for adaptive trigger notifications
- Existing Phase 4 loop infrastructure (body pin traversal, topological sort) extends naturally to nested scans

**Primary recommendation:** Proceed with Zarr V3 migration using zarrs crate with Xarray encoding conventions. Implement nested scans as dedicated node types reusing Phase 4 loop body traversal patterns.

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| zarrs | 0.22.10 | Zarr V3 storage | Official Rust implementation, first-class V3 support, object_store integration |
| object_store | latest | Cloud storage abstraction | Apache Arrow project, unified API for local/S3/GCS/Azure |
| find_peaks | 0.1.5 | Peak detection | scipy-equivalent prominence-based filtering for noisy data |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| egui Modal | 0.33.3+ | Confirmation dialogs | Adaptive trigger alerts, user approval prompts |
| serde_json | 1.0 | Zarr attributes | Writing `_ARRAY_DIMENSIONS` and coordinate metadata |
| tokio spawn_blocking | 1.36 | Async I/O | Zarr file writes from GUI context |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| zarrs | hdf5-metno | HDF5 has maintenance uncertainty, weaker Rust ecosystem |
| Zarr V3 | HDF5 | HDF5 mature but lacks cloud-native features, no chunking metadata |
| find_peaks | Manual threshold | Peak detection handles noise better than simple thresholds |

**Installation:**
```toml
# Add to daq-storage/Cargo.toml
zarrs = "0.22"
object_store = "0.11"  # For cloud backends
find_peaks = "0.1"
```

## Architecture Patterns

### Zarr Directory Structure
```
experiment_run_001.zarr/
├── .zgroup                # Root group metadata
├── .zattrs                # Experiment-level attributes (provenance, etc.)
├── data/                  # N-dimensional data arrays
│   ├── .zarray            # Array metadata (shape, chunks, dtype)
│   ├── .zattrs            # Array attributes (_ARRAY_DIMENSIONS)
│   └── c/0/0/0            # Chunk data files
├── wavelength/            # Coordinate array
│   ├── .zarray
│   └── c/0
└── position/              # Coordinate array
    ├── .zarray
    └── c/0
```

### Pattern 1: Xarray-Compatible Zarr V3 Writing

**What:** Write Zarr arrays with dimensional metadata that Xarray can read
**When to use:** Any N-dimensional data (nested scans, camera frames)

**Example:**
```rust
// Source: https://docs.rs/zarrs/latest/zarrs/ + Xarray encoding spec
use zarrs::array::{ArrayBuilder, DataType, FillValue};
use zarrs::storage::store::FilesystemStore;
use serde_json::json;

// Create Zarr store
let store = FilesystemStore::new("experiment_001.zarr")?;

// Build array with Xarray-compatible attributes
let mut attributes = serde_json::Map::new();
attributes.insert(
    "_ARRAY_DIMENSIONS".to_string(),
    json!(["wavelength", "position_x", "position_y", "height", "width"])
);
attributes.insert("units".to_string(), json!("counts"));
attributes.insert("long_name".to_string(), json!("Camera frames"));

let array = ArrayBuilder::new(
    vec![10, 5, 5, 1024, 1024], // [wavelength, x, y, height, width]
    DataType::UInt16,
    vec![1, 5, 5, 256, 256].try_into()?, // Chunk shape
    FillValue::from(0u16),
)
.attributes(attributes)
.build(store.into(), "/data")?;

// Write data chunk by chunk
array.store_chunk_elements(&[0, 0, 0, 0, 0], &frame_data)?;
```

### Pattern 2: Nested Loop Translation

**What:** Translate outer/inner loop nodes to properly dimensioned Zarr arrays
**When to use:** Nested scans with multiple loop levels

**Example:**
```rust
// Reuse existing Phase 4 loop infrastructure
fn translate_nested_scan(
    outer_loop_id: NodeId,
    snarl: &Snarl<ExperimentNode>,
) -> Vec<PlanCommand> {
    // Find inner loop nodes via body pin (pin 1) traversal
    let body_nodes = find_loop_body_nodes(outer_loop_id, snarl);

    // Filter for inner loop nodes
    let inner_loops: Vec<_> = body_nodes.iter()
        .filter(|&id| matches!(snarl.get_node(*id), Some(ExperimentNode::Loop(_))))
        .collect();

    // Build nested iteration structure
    let outer_iterations = get_iteration_count(outer_loop_id, snarl);
    for outer_idx in 0..outer_iterations {
        for &inner_id in &inner_loops {
            let inner_iterations = get_iteration_count(inner_id, snarl);
            for inner_idx in 0..inner_iterations {
                // Emit commands with dimensional context
                commands.push(PlanCommand::Checkpoint {
                    label: format!("outer_{}_inner_{}", outer_idx, inner_idx),
                });
            }
        }
    }
}
```

### Pattern 3: Adaptive Scan with Peak Detection

**What:** Detect peaks in live data and trigger zoom/reposition actions
**When to use:** Finding optimal positions, auto-focusing, peak tracking

**Example:**
```rust
// Source: https://github.com/tungli/find_peaks-rs
use find_peaks::PeakFinder;

fn detect_peak_and_zoom(
    signal: &[f64],
    prominence_threshold: f64,
) -> Option<(usize, f64)> {
    let mut fp = PeakFinder::new(signal);
    fp.with_min_prominence(prominence_threshold);
    fp.with_min_height(0.0);

    let peaks = fp.find_peaks();
    peaks.first().map(|p| (p.middle_position(), p.height.unwrap()))
}

// In adaptive scan node translation
if let Some((peak_pos, _)) = detect_peak_and_zoom(&scan_data, 200.0) {
    // Trigger action: zoom 2x around peak
    let new_range = calculate_zoom_range(peak_pos, current_range, 2.0);
    commands.push(PlanCommand::UpdateScanRange(new_range));
}
```

### Pattern 4: egui Modal for Adaptive Alerts

**What:** Show confirmation dialog when adaptive trigger fires
**When to use:** User wants to approve scan changes before proceeding

**Example:**
```rust
// Source: https://docs.rs/egui/latest/egui/containers/modal/
use egui::{Modal, Id};

// In ExperimentDesignerPanel update()
if self.adaptive_trigger_fired {
    let modal = Modal::new(Id::new("adaptive_alert"))
        .backdrop_color(egui::Color32::from_black_alpha(150));

    modal.show(ctx, |ui| {
        ui.heading("Peak Detected!");
        ui.label(format!("Peak at position {:.2}", self.peak_position));
        ui.label("Proceeding with 2x zoom scan...");

        ui.horizontal(|ui| {
            if ui.button("Continue").clicked() {
                self.adaptive_trigger_fired = false;
                self.resume_scan();
            }
            if ui.button("Cancel").clicked() {
                self.adaptive_trigger_fired = false;
                self.abort_scan();
            }
        });
    });
}
```

### Pattern 5: Chunking Strategy for Nested Scans

**What:** Choose chunk shapes that match access patterns
**When to use:** Optimizing read/write performance for nested multi-dimensional data

**Example:**
```rust
// Source: Zarr best practices - https://zarr.readthedocs.io/en/v3.0.0/user-guide/performance.html
// For nested scan: outer wavelength (10 points), inner XY (100x100 positions), camera (1024x1024)
// Data shape: [10, 100, 100, 1024, 1024]

// Access pattern: Extract time series at each XY position (all wavelengths)
// Optimal chunks: [10, 1, 1, 256, 256]  // All wavelengths per position
let chunk_shape = vec![
    outer_scan_points,  // All outer loop iterations in one chunk
    1,                   // Single inner X position
    1,                   // Single inner Y position
    256,                 // Spatial tile
    256,                 // Spatial tile
];

// Target chunk size: 10-100 MB
// Calculation: 10 * 1 * 1 * 256 * 256 * 2 bytes = 1.3 MB (good)
```

### Anti-Patterns to Avoid

- **Don't use tiny chunks (<1 MB):** Increases file count, slows cloud access
- **Don't chunk orthogonal to access pattern:** If extracting time series, must include all time points in chunks
- **Don't write Zarr files on GUI thread:** Use `tokio::task::spawn_blocking` like HDF5Writer
- **Don't skip `_ARRAY_DIMENSIONS` attribute:** Xarray won't recognize dimensional structure
- **Don't unroll infinite nested loops:** Safety limits required (max_iterations fallback)

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Peak detection | Threshold crossing | find_peaks crate | Handles noise via prominence filtering, matches scipy behavior |
| Cloud storage | Custom S3 code | object_store crate | Unified API for all cloud providers, battle-tested by Apache Arrow |
| Zarr encoding | Manual JSON | zarrs ArrayBuilder | Handles chunking, compression, metadata automatically |
| Modal dialogs | Custom overlay | egui Modal | Native backdrop blocking, stacking, auto-close |
| N-D array chunking | Manual calculation | Zarr codec system | Automatic compression, sharding for large datasets |
| Coordinate metadata | Custom attributes | Xarray conventions | Python ecosystem expects `_ARRAY_DIMENSIONS` format |

**Key insight:** Zarr V3 specification is complex with sharding, compression codecs, and cloud storage. The zarrs crate handles this correctly. Manual implementation would miss edge cases (variable chunking, ZEP extensions, NCZarr compatibility).

## Common Pitfalls

### Pitfall 1: Zarr V2 vs V3 Attribute Encoding

**What goes wrong:** Writing `_ARRAY_DIMENSIONS` as Zarr V3 native dimension_names breaks Xarray compatibility
**Why it happens:** Zarr V3 spec moved dimension metadata out of .zattrs into array metadata
**How to avoid:**
- Check Zarr version in zarrs builder
- Zarr V2: Write `_ARRAY_DIMENSIONS` to .zattrs
- Zarr V3: Use native dimension_names field
**Warning signs:** Xarray raises "no dimension metadata found" error

### Pitfall 2: Chunk Size Too Small

**What goes wrong:** Writing 100,000+ tiny chunk files slows filesystem/cloud operations
**Why it happens:** Default chunk size or naively chunking each acquisition frame separately
**How to avoid:**
- Target 10-100 MB chunks
- For camera frames: chunk multiple frames together [time, height/tiles, width/tiles]
- Use zarrs sharding codec to pack multiple logical chunks into one file
**Warning signs:** `ls experiment.zarr/data/c/` shows thousands of files, cloud upload takes hours

### Pitfall 3: Loop Body Back-Edges

**What goes wrong:** Inner loop connects back to outer loop node, creating infinite recursion
**Why it happens:** User accidentally wires from inner loop back to outer loop "Next" input
**How to avoid:**
- Validation: detect back-edges during graph validation (already implemented in Phase 4)
- Check if body node targets ancestor loop node
- Display error: "Loop body cannot connect back to parent loop"
**Warning signs:** Translation hangs or produces extremely long command lists

### Pitfall 4: Adaptive Trigger Deadlock

**What goes wrong:** Modal dialog blocks UI while waiting for trigger condition that never fires
**Why it happens:** Trigger evaluation happens on blocked thread, creating deadlock
**How to avoid:**
- Evaluate triggers in RunEngine (server-side), not GUI thread
- Modal only shows AFTER trigger fires (server sends notification)
- Never block GUI thread waiting for hardware condition
**Warning signs:** GUI freezes during adaptive scan, no way to cancel

### Pitfall 5: Missing object_store Features

**What goes wrong:** Local Zarr writes work, but S3 upload fails with permission errors
**Why it happens:** object_store requires explicit configuration for cloud backends
**How to avoid:**
- Use `object_store::parse_url()` to detect local vs cloud paths
- Configure credentials via environment variables or builder
- Test with LocalStack before production S3
**Warning signs:** Works on filesystem, fails with "access denied" on s3://

## Code Examples

Verified patterns from official sources:

### Creating Zarr Group with Attributes

```rust
// Source: https://docs.rs/zarrs/latest/zarrs/ + zarrs examples
use zarrs::group::{Group, GroupBuilder};
use zarrs::storage::store::FilesystemStore;
use serde_json::json;

let store = FilesystemStore::new("experiment.zarr")?;
let mut attributes = serde_json::Map::new();
attributes.insert("experiment_id".to_string(), json!("EXP-001"));
attributes.insert("created_at".to_string(), json!("2026-01-25T12:00:00Z"));

let group = GroupBuilder::new()
    .attributes(attributes)
    .build(store.into(), "/")?;
```

### Writing N-Dimensional Camera Data

```rust
// 4D camera data: [wavelength, position, height, width]
use zarrs::array::{ArrayBuilder, DataType, FillValue};

let shape = vec![
    wavelength_points as u64,  // Outer scan dimension
    xy_positions as u64,        // Inner scan dimension
    camera_height as u64,       // Frame height
    camera_width as u64,        // Frame width
];

// Chunk strategy: All wavelengths, single position, spatial tiles
let chunks = vec![
    wavelength_points as u64,  // All wavelengths together
    1,                          // One position per chunk
    256,                        // Spatial tile
    256,                        // Spatial tile
];

let mut attrs = serde_json::Map::new();
attrs.insert("_ARRAY_DIMENSIONS".to_string(),
             json!(["wavelength", "position", "y", "x"]));

let array = ArrayBuilder::new(
    shape,
    DataType::UInt16,
    chunks.try_into()?,
    FillValue::from(0u16),
)
.attributes(attrs)
.build(store, "/camera_frames")?;

// Write frame by frame
for wl_idx in 0..wavelength_points {
    for pos_idx in 0..xy_positions {
        let chunk_indices = vec![wl_idx, pos_idx, 0, 0];
        array.store_chunk_elements(&chunk_indices, &frame_data)?;
    }
}
```

### Peak Detection with Prominence Filtering

```rust
// Source: https://github.com/tungli/find_peaks-rs
use find_peaks::PeakFinder;

fn find_scan_peaks(signal: &[f64], min_prominence: f64) -> Vec<(usize, f64)> {
    let mut fp = PeakFinder::new(signal);
    fp.with_min_prominence(min_prominence);
    fp.with_min_height(0.0);

    fp.find_peaks()
        .iter()
        .map(|p| (p.middle_position(), p.height.unwrap()))
        .collect()
}

// Usage in adaptive scan
let signal: Vec<f64> = scan_data.iter().map(|x| *x as f64).collect();
let peaks = find_scan_peaks(&signal, 200.0);

if let Some((peak_pos, peak_height)) = peaks.first() {
    println!("Found peak at index {} with height {}", peak_pos, peak_height);
    // Calculate zoom range centered on peak
    let zoom_range = calculate_zoom_range(*peak_pos, current_range, 2.0);
}
```

### Modal Dialog with Backdrop

```rust
// Source: https://docs.rs/egui/latest/egui/containers/modal/
use egui::{Modal, Id, Color32};

impl ExperimentDesignerPanel {
    fn show_adaptive_alert(&mut self, ctx: &egui::Context) {
        let modal = Modal::new(Id::new("peak_detected"))
            .backdrop_color(Color32::from_black_alpha(150));

        modal.show(ctx, |ui| {
            ui.heading("🔍 Peak Detected!");
            ui.separator();

            ui.label(format!("Position: {:.2} µm", self.detected_peak_pos));
            ui.label(format!("Signal: {:.1} counts", self.detected_peak_height));

            ui.add_space(10.0);
            ui.label("Action: Zoom 2x and rescan");

            ui.horizontal(|ui| {
                if ui.button("✓ Continue").clicked() {
                    self.confirm_adaptive_action();
                }
                if ui.button("✗ Cancel").clicked() {
                    self.cancel_adaptive_action();
                }
            });
        });
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| HDF5 for scientific data | Zarr V3 | 2023-2024 | Cloud-native, better chunking, no file size limits |
| Manual dimension metadata | Xarray encoding conventions | 2020 | Standardized `_ARRAY_DIMENSIONS` for interop |
| scipy.signal.find_peaks (Python) | find_peaks crate (Rust) | 2022 | Native Rust, no FFI overhead |
| Custom modal overlays | egui native Modal | 2024 (egui 0.27+) | Built-in backdrop blocking, stacking |
| Single storage backend | object_store abstraction | 2022 | Unified API for local/S3/GCS/Azure |

**Deprecated/outdated:**
- **HDF5 as primary storage:** Still valid for compatibility, but Zarr is now standard for new cloud-native workflows
- **Custom dimension metadata:** Use Xarray conventions (`_ARRAY_DIMENSIONS`) instead of ad-hoc attribute schemas
- **Blocking file I/O on GUI thread:** Always use `tokio::task::spawn_blocking` for Zarr/HDF5 writes

## Open Questions

Things that couldn't be fully resolved:

1. **HDF5 to Zarr migration path**
   - What we know: Both formats can coexist via separate writers (HDF5Writer, ZarrWriter)
   - What's unclear: Should existing HDF5 files be converted retroactively? How to signal format to Python users?
   - Recommendation:
     - Write Zarr by default in Phase 8
     - Keep HDF5 writer for backward compatibility (feature flag)
     - Add format field to ExperimentManifest (.expgraph metadata)
     - Python users: `xarray.open_zarr()` for new data, `h5py` for old data

2. **Zarr sharding vs regular chunking**
   - What we know: Sharding packs multiple logical chunks into one file, reducing file count
   - What's unclear: What shard size is optimal for 1-10GB datasets? zarrs sharding API not well documented
   - Recommendation:
     - Start with regular chunking (10-100 MB chunks)
     - Defer sharding to Phase 10 (performance optimization)
     - Monitor file count in testing (warn if >1000 files)

3. **Nested loop maximum depth**
   - What we know: UI allows unlimited nesting, Phase 4 loop traversal is recursive
   - What's unclear: At what depth does translation become impractically slow? Stack overflow risk?
   - Recommendation:
     - No hard limit in Phase 8 (user knows their use case)
     - Add UI warning for depth > 3 ("Deep nesting may slow translation")
     - Add recursion limit in translation code (max 10 levels) with clear error

4. **Multi-trigger AND/OR evaluation order**
   - What we know: User wants "signal > 1000 AND derivative > 50" combinations
   - What's unclear: Short-circuit evaluation? Evaluation frequency (per-frame, per-batch)?
   - Recommendation:
     - Evaluate triggers after each Acquire command completes
     - Use standard boolean logic (left-to-right, short-circuit AND/OR)
     - Document evaluation timing in trigger node help text

## Sources

### Primary (HIGH confidence)

- [zarrs crate documentation](https://docs.rs/zarrs/latest/zarrs/) - v0.22.10 API, array builder, attributes
- [zarrs GitHub repository](https://github.com/LDeakin/zarrs) - Examples, features, object_store integration
- [Xarray Zarr Encoding Specification](https://docs.xarray.dev/en/latest/internals/zarr-encoding-spec.html) - `_ARRAY_DIMENSIONS` conventions
- [object_store crate Context7](https://github.com/apache/arrow-rs-object-store/blob/main/CHANGELOG-old.md) - Builder patterns, cloud backends
- [find_peaks-rs GitHub](https://github.com/tungli/find_peaks-rs) - Peak detection API, prominence filtering
- [egui Modal documentation](https://docs.rs/egui/latest/egui/containers/modal/struct.Modal.html) - Native modal API (egui 0.33.3)

### Secondary (MEDIUM confidence)

- [Zarr V3 chunking best practices](https://zarr.readthedocs.io/en/v3.0.0/user-guide/performance.html) - 10-100 MB chunks, sharding
- [Pangeo Zarr chunking guidance](https://discourse.pangeo.io/t/high-level-guidance-for-zarr-chunking/3641) - Access pattern optimization
- [Cloud-Native Geo Zarr Guide](https://guide.cloudnativegeo.org/zarr/intro.html) - S3 best practices
- [scipy.signal.find_peaks docs](https://docs.scipy.org/doc/scipy/reference/generated/scipy.signal.find_peaks.html) - Reference for Rust port

### Tertiary (LOW confidence)

None - all key findings verified with official sources

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All crates verified via docs.rs, active development, stable APIs
- Architecture: HIGH - Xarray encoding spec is authoritative, zarrs examples confirmed
- Pitfalls: MEDIUM - Based on Zarr/HDF5 community wisdom, not all tested in this codebase yet

**Research date:** 2026-01-25
**Valid until:** 2026-06-25 (6 months - Zarr V3 spec is stable, zarrs under active development)
