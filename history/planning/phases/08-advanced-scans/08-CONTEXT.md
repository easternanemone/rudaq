# Phase 8: Advanced Scans - Context

**Gathered:** 2026-01-25
**Status:** Ready for planning

<domain>
## Phase Boundary

This phase delivers nested multi-dimensional scans and adaptive scans that respond to acquired data. Users can create outer/inner scan combinations (e.g., wavelength sweep with XY position scan) and define triggers that automatically adjust scan parameters based on measurements (e.g., zoom into detected peaks).

**Includes:**
- Nested scan configuration (outer loop containing inner scans)
- Adaptive scan triggers (threshold crossing, peak detection)
- Proper N-dimensional data storage with axis labels
- Progress feedback for nested/adaptive execution

**Does NOT include:**
- Template library (Phase 9)
- Subgraph grouping (Phase 9)
- Performance optimization for 50+ nodes (Phase 10)

</domain>

<decisions>
## Implementation Decisions

### Nested Scan Configuration
- **Nesting specification:** Explicit nesting nodes (dedicated 'Outer Loop' and 'Inner Loop' node types)
- **Visual layout:** Claude's discretion (likely body output pin pattern from Phase 4 loops)
- **Maximum depth:** Unlimited (with UI warnings for deep nesting)
- **Node reuse:** Claude's discretion (may reuse existing 1D/2D scan nodes or create dedicated nested variants)

### Adaptive Scan Triggers
- **Trigger types:** Support BOTH threshold crossing AND peak detection
- **Peak response:** User-defined action (from predefined menu)
- **Action configuration:** Predefined action menu with common options: 'Zoom 2x', 'Move to peak', 'Acquire at peak', etc.
- **Multi-trigger logic:** Support AND/OR combinations (e.g., 'signal > 1000 AND derivative > 50')

### Data Dimensionality - MAJOR DECISION: Zarr Migration
- **Storage format:** Migrate from HDF5 to **Zarr V3** in Phase 8
  - Use `zarrs` Rust crate for writing
  - Follow Xarray Zarr encoding conventions (`_ARRAY_DIMENSIONS` attribute)
  - Python analysis via `xarray.open_zarr()`
- **Data structure:** N-dimensional arrays with shape [outer_points, inner_points, ...]
- **Camera frames:** 4D+ arrays with proper chunking: [outer, inner, height, width]
- **Axis metadata:** Xarray-compatible Zarr encoding (dimension scales via `.zattrs`)
- **Backwards compatibility:** Claude's discretion (likely read both HDF5/Zarr, write Zarr)
- **Data scale:** Optimize for medium datasets (1-10GB per run)
- **Storage backend:** Cloud-ready architecture using `object_store` crate (local + S3/GCS)

### Execution Feedback
- **Progress display:** Both nested and flattened progress with toggle (default nested: "Outer 3/10, Inner 45/100")
- **Adaptive alerts:** Modal popup when trigger fires ("Peak detected! Proceeding with zoom scan...")
- **Trigger pause:** Optional per-trigger setting (auto-proceed or require approval)
- **Visual preview:** Show shaded region on live plot highlighting zoom target before executing

### Claude's Discretion
- Visual layout for nested scan nodes (body pin vs contained subgraph)
- Whether to reuse existing scan nodes or create dedicated nested variants
- HDF5 backwards compatibility approach during Zarr migration
- Chunking strategy details for Zarr based on zarrs crate capabilities

</decisions>

<specifics>
## Specific Ideas

### Zarr Migration Rationale (from Gemini research)
- **zarrs** crate is the leading Rust Zarr V3 implementation
- Zarr is the modern standard for cloud-native N-dimensional data
- Better Rust ecosystem support than HDF5 (hdf5 crate maintenance uncertainty)
- Native Xarray interoperability via standard encoding conventions
- Chunked storage enables parallel DAQ writes and analysis reads

### Adaptive Action Menu Options
User-defined from predefined list:
- Zoom 2x (narrow range, increase resolution)
- Zoom 4x
- Move to peak (position actuator at detected peak)
- Acquire at peak (trigger detector at peak location)
- Mark and continue (record location, don't change scan)

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

Note: Zarr migration could be a separate phase, but user explicitly chose to include it in Phase 8.

</deferred>

---

*Phase: 08-advanced-scans*
*Context gathered: 2026-01-25*
