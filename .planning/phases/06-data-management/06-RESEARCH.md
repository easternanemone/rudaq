# Phase 6: Data Management - Research

**Researched:** 2026-01-22
**Domain:** Scientific data management, metadata capture, run history, data comparison
**Confidence:** MEDIUM

## Summary

Phase 6 extends the existing auto-save infrastructure (Phase 1) to add comprehensive metadata capture, user annotations, run history browsing, and multi-run comparison tools. The research reveals that the foundation is already in place via Bluesky's document model and HDF5 storage, requiring primarily GUI work and metadata enrichment.

Key findings:
- Bluesky document model already captures device settings and timestamps in StartDoc/Manifest
- HDF5 attributes provide extensible metadata storage (h5py 3.15.1 as of Jan 2026)
- gRPC StorageService has acquisition listing/querying infrastructure
- egui_extras TableBuilder exists for run history browsing, with sorting available via third-party crates
- Metadata comparison patterns well-established in scientific Python ecosystem

**Primary recommendation:** Build GUI panels for metadata input/display, extend StartDoc metadata, leverage HDF5 attributes for user notes/tags, and create comparison visualization using existing plot infrastructure.

## Standard Stack

### Core Components

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| HDF5 | 1.10+ | Hierarchical data storage | Industry standard for scientific data, self-describing format |
| h5py (reference) | 3.15.1 | Metadata attribute patterns | Jan 2026 release, canonical Python interface showing attribute usage |
| egui_extras | latest | TableBuilder for run lists | Official egui companion crate for tables |
| Bluesky document model | (existing) | Metadata capture pattern | NSLS-II proven pattern for experiment metadata |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| egui-selectable-table | latest | Sortable table rows | If need row sorting by clicking headers |
| egui-data-table | latest | Generic table widget | Alternative if TableBuilder insufficient |
| serde_json | latest | JSON metadata serialization | For flexible metadata in HDF5 attributes |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| HDF5 attributes | Custom JSON sidecar files | HDF5 keeps metadata with data, self-describing |
| egui_extras Table | Custom ScrollArea + Grid | Table handles column sizing/resizing automatically |
| StartDoc.metadata | Separate metadata table | Document model keeps metadata with run definition |

**Installation:**

Already in Cargo.toml:
```toml
[dependencies]
hdf5 = { version = "0.8", optional = true }
egui_extras = { version = "0.30", features = ["table"] }
serde_json = "1.0"
```

## Architecture Patterns

### Recommended Project Structure

```
crates/daq-egui/src/
├── panels/
│   ├── storage.rs           # Existing acquisition list (extend for metadata)
│   ├── run_history.rs       # NEW: Browsing/filtering past runs
│   └── run_comparison.rs    # NEW: Multi-run comparison plots
├── widgets/
│   ├── metadata_editor.rs   # NEW: Tags/notes input during run
│   └── run_table.rs         # NEW: Sortable/filterable run list
crates/daq-storage/src/
└── document_writer.rs       # Extend for user metadata
```

### Pattern 1: Metadata Capture During Execution

**What:** Enrich StartDoc with user-editable metadata fields before/during run
**When to use:** User wants to tag experiments with sample ID, purpose, notes
**Example:**

```rust
// In ExperimentDesignerPanel or new MetadataPanel
struct RunMetadata {
    sample_id: String,
    operator: String,
    purpose: String,
    notes: String,
    tags: Vec<String>,
}

// Before queuing plan:
let mut start_doc = StartDoc::new("count", "Count Plan");
start_doc.metadata.insert("sample_id".to_string(), metadata.sample_id);
start_doc.metadata.insert("operator".to_string(), metadata.operator);
start_doc.metadata.insert("tags".to_string(), serde_json::to_string(&metadata.tags)?);

// Queue plan with enriched StartDoc
client.queue_plan(plan_request).await?;
```

### Pattern 2: HDF5 Attribute Storage for User Annotations

**What:** Store user notes/tags as HDF5 attributes on run group
**When to use:** Post-acquisition annotation, adding context after data review
**Example:**

```rust
// Source: https://docs.h5py.org/en/stable/high/attr.html
use hdf5::File;

// Open existing HDF5 file
let file = File::open_rw(acquisition_path)?;
let run_group = file.group("start")?;

// Add user notes as attributes
run_group.new_attr::<VarLenUnicode>()
    .create("user_notes")?
    .write_scalar(&notes.parse()?)?;

// Add tags as JSON array
run_group.new_attr::<VarLenUnicode>()
    .create("tags")?
    .write_scalar(&serde_json::to_string(&tags)?.parse()?)?;
```

### Pattern 3: Run History Browsing with Filtering

**What:** Table of past runs with search/filter by metadata fields
**When to use:** User needs to find experiments by sample, date, or tag
**Example:**

```rust
// Using egui_extras TableBuilder
use egui_extras::{TableBuilder, Column};

TableBuilder::new(ui)
    .striped(true)
    .resizable(true)
    .column(Column::auto().at_least(150.0)) // Run UID
    .column(Column::auto().at_least(100.0)) // Date
    .column(Column::auto().at_least(80.0))  // Plan Type
    .column(Column::remainder())            // Notes
    .header(20.0, |mut header| {
        header.col(|ui| { ui.strong("Run UID"); });
        header.col(|ui| { ui.strong("Date"); });
        header.col(|ui| { ui.strong("Plan"); });
        header.col(|ui| { ui.strong("Notes"); });
    })
    .body(|mut body| {
        for run in &filtered_runs {
            body.row(18.0, |mut row| {
                row.col(|ui| { ui.label(&run.uid[..8]); });
                row.col(|ui| { ui.label(format_timestamp(run.time_ns)); });
                row.col(|ui| { ui.label(&run.plan_type); });
                row.col(|ui| { ui.label(&run.notes); });
            });
        }
    });
```

### Pattern 4: Multi-Run Comparison

**What:** Overlay data from multiple runs on same plot axes
**When to use:** Comparing scans across different samples or conditions
**Example:**

```rust
// Reuse PlotWidget infrastructure from Phase 5
for (idx, run_uid) in &selected_runs.iter().enumerate() {
    let data = load_run_data(run_uid)?;
    let color = Color32::from_rgb(colors[idx][0], colors[idx][1], colors[idx][2]);

    let line = Line::new(PlotPoints::new(data.points))
        .color(color)
        .name(&run_uid[..8]);

    plot_ui.line(line);
}

// Add legend
plot_ui.legend(Legend::default().position(Corner::RightTop));
```

### Anti-Patterns to Avoid

- **Hardcoded metadata fields:** Use HashMap<String, String> in StartDoc for extensibility
- **Synchronous HDF5 I/O on UI thread:** Use spawn_blocking for file operations
- **Loading entire run data for browsing:** Use AcquisitionSummary from gRPC, not full datasets
- **String-based filtering only:** Support tag-based filtering for categorization

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Sortable table columns | Custom click handlers + state | egui-selectable-table | Handles ascending/descending, column state |
| Metadata validation | String checks in UI code | Bluesky-style validation functions | Reusable, testable, consistent |
| Timestamp formatting | Manual date parsing | chrono::DateTime | Handles timezones, locales, DST |
| Tag autocomplete | Custom text matching | Existing tags from DB as suggestions | Prevents typos, maintains consistency |

**Key insight:** Metadata management is well-trodden ground in scientific Python. The patterns from Bluesky (NSLS-II) and h5py provide proven approaches that translate directly to Rust.

## Common Pitfalls

### Pitfall 1: Metadata Schema Rigidity

**What goes wrong:** Hardcoding specific metadata fields (e.g., "sample_id", "operator") prevents users from adding custom fields for their domain.

**Why it happens:** Easy to start with a form with fixed fields instead of extensible key-value pairs.

**How to avoid:**
- Store metadata as `HashMap<String, String>` in StartDoc
- Provide UI for adding arbitrary key-value pairs
- Optionally suggest common fields (sample, operator, purpose)
- Validate only required fields, not schema

**Warning signs:** User requests "can you add field X to the metadata?"

### Pitfall 2: Late Metadata Capture

**What goes wrong:** Capturing metadata only at run start misses context discovered during or after acquisition (e.g., "sample degraded during scan", "detector saturated").

**Why it happens:** Natural to think metadata belongs at experiment start.

**How to avoid:**
- Allow metadata editing during paused runs
- Support post-acquisition annotation via HDF5 attributes
- Separate StartDoc.metadata (immutable intent) from mutable notes/tags
- Consider StopDoc.reason field for exit status notes

**Warning signs:** Users writing notes in separate lab notebooks because GUI can't capture them.

### Pitfall 3: Inefficient Run History Loading

**What goes wrong:** Loading full HDF5 files just to display run summaries causes UI lag with many acquisitions.

**Why it happens:** Convenient to read metadata directly from HDF5 headers.

**How to avoid:**
- Use StorageService.ListAcquisitions for summaries (already returns AcquisitionSummary)
- Cache metadata in SQLite index (future optimization)
- Load full HDF5 only when viewing/comparing specific run
- Paginate acquisition lists (already supported in gRPC)

**Warning signs:** UI freezes when navigating to storage panel with 100+ runs.

### Pitfall 4: String-Only Search

**What goes wrong:** Searching only by text substring in notes misses structured queries like "all runs with tag:calibration from last week".

**Why it happens:** Text search is simplest to implement.

**How to avoid:**
- Parse structured queries: "tag:calibration date:2026-01-15:"
- Support filter combinators (AND, OR)
- Provide filter UI elements (date range picker, tag checkboxes)
- Index common fields for fast filtering

**Warning signs:** Users asking "how do I find all yesterday's calibration runs?"

## Code Examples

Verified patterns from official sources:

### HDF5 Attribute Writing (Post-Acquisition Annotation)

```rust
// Source: https://docs.h5py.org/en/stable/high/attr.html (h5py patterns)
use hdf5::{File, types::VarLenUnicode};

async fn add_user_notes(
    acquisition_id: &str,
    notes: String,
    tags: Vec<String>,
) -> Result<()> {
    let file_path = resolve_acquisition_path(acquisition_id)?;

    tokio::task::spawn_blocking(move || -> Result<()> {
        let file = File::open_rw(&file_path)?;
        let start_group = file.group("start")?;

        // Add freeform notes
        start_group.new_attr::<VarLenUnicode>()
            .create("user_notes")?
            .write_scalar(&notes.parse()?)?;

        // Add tags as JSON
        let tags_json = serde_json::to_string(&tags)?;
        start_group.new_attr::<VarLenUnicode>()
            .create("tags")?
            .write_scalar(&tags_json.parse()?)?;

        // Timestamp the annotation
        start_group.new_attr::<u64>()
            .create("annotated_at_ns")?
            .write_scalar(&now_ns())?;

        Ok(())
    }).await??;

    Ok(())
}
```

### Reading Acquisition Metadata for History View

```rust
// Using existing gRPC infrastructure
async fn load_run_history(client: &DaqClient) -> Result<Vec<RunSummary>> {
    let response = client.list_acquisitions(
        daq_proto::daq::ListAcquisitionsRequest {
            limit: Some(100),
            offset: Some(0),
            name_pattern: None,
            after_timestamp_ns: None,
            before_timestamp_ns: None,
        }
    ).await?;

    // Convert AcquisitionSummary to UI-friendly RunSummary
    let runs = response.acquisitions.into_iter()
        .map(|acq| RunSummary {
            uid: acq.acquisition_id,
            name: acq.name,
            date: format_timestamp(acq.created_at_ns),
            plan_type: extract_plan_type(&acq.file_path)?, // Parse from HDF5
            file_size_mb: acq.file_size_bytes as f64 / 1_000_000.0,
            sample_count: acq.sample_count,
        })
        .collect();

    Ok(runs)
}

fn format_timestamp(ns: u64) -> String {
    use chrono::{DateTime, Utc, TimeZone};
    let secs = ns / 1_000_000_000;
    let dt = Utc.timestamp_opt(secs as i64, 0).unwrap();
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}
```

### Metadata Input Widget During Execution

```rust
// MetadataEditorWidget for run configuration
pub struct MetadataEditor {
    sample_id: String,
    operator: String,
    purpose: String,
    custom_fields: Vec<(String, String)>,
}

impl MetadataEditor {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Experiment Metadata");

        ui.horizontal(|ui| {
            ui.label("Sample ID:");
            ui.text_edit_singleline(&mut self.sample_id);
        });

        ui.horizontal(|ui| {
            ui.label("Operator:");
            ui.text_edit_singleline(&mut self.operator);
        });

        ui.horizontal(|ui| {
            ui.label("Purpose:");
            ui.text_edit_singleline(&mut self.purpose);
        });

        ui.separator();
        ui.label("Custom Fields:");

        for (key, value) in &mut self.custom_fields {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(key);
                ui.label("=");
                ui.text_edit_singleline(value);
            });
        }

        if ui.button("+ Add Field").clicked() {
            self.custom_fields.push((String::new(), String::new()));
        }
    }

    pub fn to_metadata_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("sample_id".to_string(), self.sample_id.clone());
        map.insert("operator".to_string(), self.operator.clone());
        map.insert("purpose".to_string(), self.purpose.clone());

        for (key, value) in &self.custom_fields {
            if !key.is_empty() {
                map.insert(key.clone(), value.clone());
            }
        }

        map
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Fixed metadata schema | Extensible HashMap<String, String> | Bluesky v1.0 (2018) | Users can add domain-specific fields |
| Metadata only at start | Post-acquisition annotation | h5py attributes pattern | Captures context discovered during analysis |
| Text-only search | Structured query language | Modern scientific tools (2020+) | Precise filtering (tag:X date:Y) |
| Manual CSV metadata files | HDF5 self-describing attributes | HDF5 widespread adoption | Metadata travels with data |

**Deprecated/outdated:**
- Separate metadata spreadsheets: HDF5 attributes keep metadata with data
- Hardcoded forms: Key-value pairs support arbitrary fields
- Scan-only metadata: Document model captures all experiment types (scans, time series, counts)

## Open Questions

Things that couldn't be fully resolved:

1. **Metadata persistence strategy**
   - What we know: StartDoc.metadata captured in HDF5 /start group attributes
   - What's unclear: Should post-acquisition annotations modify HDF5 directly or use separate database?
   - Recommendation: Start with HDF5 attributes (simple, self-describing), add SQLite index if performance requires

2. **Tag vocabulary management**
   - What we know: Users want consistent tags to avoid typos
   - What's unclear: Global tag vocabulary vs. per-user tags?
   - Recommendation: Suggest recent tags as autocomplete, don't enforce schema

3. **Comparison plot limits**
   - What we know: Can overlay multiple runs on same axes
   - What's unclear: How many runs before UI becomes cluttered?
   - Recommendation: Limit to 5-10 runs, provide opacity slider, allow toggling visibility

## Sources

### Primary (HIGH confidence)

- [Recording Metadata - Bluesky 1.14.7 documentation](https://blueskyproject.io/bluesky/main/metadata.html) - Metadata patterns from NSLS-II
- [HDF5 Attributes - h5py 3.15.1 documentation](https://docs.h5py.org/en/stable/high/attr.html) - Canonical metadata storage pattern (Jan 2026)
- [Documents - Bluesky Event Model](https://nsls-ii.github.io/event-model/data-model.html) - Document model structure
- [egui_extras TableBuilder](https://docs.rs/egui_extras/latest/egui_extras/struct.TableBuilder.html) - Table widget for run lists

### Secondary (MEDIUM confidence)

- [egui-selectable-table](https://lib.rs/crates/egui-selectable-table) - Sortable table rows
- Existing rust-daq codebase (daq-storage/document_writer.rs, daq-proto/proto/daq.proto)

### Tertiary (LOW confidence)

- General scientific data management patterns (MLflow, Weights & Biases) - not Rust-specific

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - Bluesky and HDF5 are proven, h5py documentation is current (Jan 2026)
- Architecture: MEDIUM - Patterns verified from Bluesky, but need to validate egui table performance with large datasets
- Pitfalls: HIGH - Well-documented issues in scientific Python community

**Research date:** 2026-01-22
**Valid until:** 2026-02-22 (30 days - stable domain, HDF5 and Bluesky mature)
