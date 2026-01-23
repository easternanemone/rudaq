---
phase: 06-data-management
verified: 2026-01-22T20:30:00Z
status: gaps_found
score: 3/4 must-haves verified
gaps:
  - truth: "Metadata captured automatically with each run (device settings, timestamps, scan parameters)"
    status: partial
    reason: "ExperimentDesignerPanel extracts metadata but doesn't send to server (TODO at line 878)"
    artifacts:
      - path: "crates/daq-egui/src/panels/experiment_designer.rs"
        issue: "metadata_editor.to_metadata_map() called but result not sent via gRPC"
    missing:
      - "Queue plan via gRPC with metadata in ExperimentDesignerPanel"
      - "Pass metadata map to server-side QueuePlanRequest"
---

# Phase 6: Data Management Verification Report

**Phase Goal:** Complete metadata capture, run history browsing, and comparison tools

**Verified:** 2026-01-22T20:30:00Z

**Status:** gaps_found

**Re-verification:** No — initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | Metadata captured automatically with each run (device settings, timestamps, scan parameters) | ⚠️ PARTIAL | ScanBuilderPanel: VERIFIED (metadata flows to QueuePlanRequest). ExperimentDesignerPanel: STUB (metadata extracted but not sent, TODO at line 878) |
| 2 | User can add custom notes and tags to experiments during or after execution | ✓ VERIFIED | MetadataEditor provides notes/tags fields during setup. RunHistoryPanel provides post-acquisition annotation UI with HDF5 persistence |
| 3 | User can browse run history, search by metadata, and view previous results | ✓ VERIFIED | RunHistoryPanel renders table with search filter, detail view shows metadata, async gRPC loading functional |
| 4 | User can compare data from multiple runs with overlaid plots | ✓ VERIFIED | RunComparisonPanel loads HDF5 data, renders multi-line overlay plots with color-coding and legend toggles |

**Score:** 3/4 truths verified (1 partial)

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/daq-egui/src/widgets/metadata_editor.rs` | MetadataEditor widget with extensible key-value UI | ✓ VERIFIED | 242 lines, exports MetadataEditor/to_metadata_map()/is_empty(), has comprehensive unit tests |
| `crates/daq-egui/src/panels/scan_builder.rs` | MetadataEditor integration in scan builder | ✓ VERIFIED | Imports MetadataEditor, integrates in struct, calls to_metadata_map() and passes to QueuePlanRequest |
| `crates/daq-egui/src/panels/experiment_designer.rs` | MetadataEditor integration in experiment designer | ⚠️ ORPHANED | Has MetadataEditor field and UI rendering, but metadata not sent to server (TODO at line 878) |
| `crates/daq-egui/src/panels/run_history.rs` | RunHistoryPanel with filterable table and detail view | ✓ VERIFIED | 467 lines, uses egui_extras::TableBuilder, async loading via client.list_acquisitions(), search filter functional |
| `crates/daq-storage/src/hdf5_annotation.rs` | HDF5 annotation support (add/read annotations) | ✓ VERIFIED | 109 lines, exports add_run_annotation/read_run_annotations/RunAnnotation, feature-gated with storage_hdf5 |
| `crates/daq-egui/src/panels/run_comparison.rs` | RunComparisonPanel with multi-run overlay plotting | ✓ VERIFIED | 420 lines, implements HDF5 data loading via spawn_blocking, egui_plot multi-line overlay with color palette |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|----|--------|---------|
| MetadataEditor | ScanBuilderPanel.to_metadata_map() | metadata_editor field | ✓ WIRED | Line 904: `let mut metadata = self.metadata_editor.to_metadata_map()` in scan_builder.rs |
| ScanBuilderPanel | QueuePlanRequest.metadata | to_metadata_map() + auto-enrichment | ✓ WIRED | Lines 904-922: metadata map passed to queue_plan with scan_type/actuator/detector auto-added |
| ExperimentDesignerPanel | metadata_editor.to_metadata_map() | metadata extraction | ⚠️ PARTIAL | Line 880: extracted but not sent (TODO comment at line 878) |
| RunHistoryPanel | StorageService.ListAcquisitions | client.list_acquisitions() | ✓ WIRED | Line 144: async gRPC call in refresh() method |
| RunHistoryPanel | HDF5 annotation persistence | spawn_blocking + add_run_annotation | ✓ WIRED | Lines 164-175: spawn_blocking wraps HDF5 I/O, writes user_notes/tags attributes |
| RunComparisonPanel | HDF5 event data loading | spawn_blocking HDF5 read | ✓ WIRED | Lines 143-152: load_run_data spawns blocking task with hdf5::File::open |
| egui_plot::Plot | Multiple Line series | Line::new with color/name | ✓ WIRED | Lines 206-224: iterates loaded_runs, creates colored Line with data.run_name |

### Requirements Coverage

| Requirement | Status | Blocking Issue |
|-------------|--------|----------------|
| DATA-02: Metadata captured with each run | ⚠️ PARTIAL | ExperimentDesignerPanel doesn't send metadata to server |
| DATA-03: User can add custom notes/tags | ✓ SATISFIED | MetadataEditor (pre-execution) + RunHistoryPanel annotations (post-execution) both functional |
| DATA-04: Browse run history and view results | ✓ SATISFIED | RunHistoryPanel table, search, detail view all verified |
| DATA-05: Compare data from multiple runs | ✓ SATISFIED | RunComparisonPanel overlay plots functional |

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| experiment_designer.rs | 878-894 | TODO comment for metadata queueing | ⚠️ Warning | Metadata extracted but not sent to server, graph execution doesn't capture metadata |
| run_history.rs | 369 | TODO for AcquisitionSummary.metadata field | ℹ️ Info | Metadata not displayed in detail view yet (proto limitation, not code issue) |

### Human Verification Required

#### 1. MetadataEditor Widget Usability

**Test:** Open ScanBuilderPanel, expand "Experiment Metadata" section, enter sample ID, operator, notes, add custom field "temperature" = "20C"

**Expected:** 
- Text inputs responsive and clear
- "+ Add Custom Field" button creates new key-value row
- Custom field delete button (✖) removes row
- No visual glitches or layout issues

**Why human:** Widget visual appearance and UX flow can't be verified programmatically

#### 2. Run History Search and Selection

**Test:** Open Run History tab, enter search query (e.g., "calibration"), click on a run row

**Expected:**
- Table filters to matching runs immediately
- Clicking row highlights it and shows detail panel below
- Detail panel displays run UID, date, file path, sample count
- "Copy Run UID" and "Copy File Path" buttons work

**Why human:** UI interaction flow and visual feedback require manual testing

#### 3. Run Comparison Multi-Run Overlay

**Test:** Open "Compare Runs" tab, check 3 runs, observe plot

**Expected:**
- Each run displays as different color line (blue, orange, green)
- Legend shows run names matching colors
- Unchecking legend checkbox hides corresponding line
- Plot auto-scales to fit all visible data

**Why human:** Visual comparison of colors, legend interaction, plot scaling are graphical behaviors

#### 4. HDF5 Annotation Persistence (Round-trip Test)

**Test:** 
1. Select a run in Run History
2. Enter notes "Test annotation" and tags "test, verification"
3. Click "Save Annotations"
4. Select different run, then re-select original run

**Expected:**
- After save: "Annotations saved ✓" message appears
- After re-selection: Notes and tags fields repopulate with saved values
- HDF5 file contains user_notes, tags, annotated_at_ns attributes in /start group

**Why human:** Requires HDF5 file inspection and round-trip persistence verification

### Gaps Summary

**1 gap blocking full goal achievement:**

**ExperimentDesignerPanel metadata not sent to server (Truth 1 - Partial)**

The MetadataEditor is integrated into ExperimentDesignerPanel and the `to_metadata_map()` method is called (line 880), extracting metadata with graph provenance (node count, file name). However, this metadata is stored in a local variable `_metadata` (underscore prefix indicates unused) and never sent to the server.

**Root cause:** Graph-based plan execution doesn't integrate with the existing QueuePlanRequest gRPC infrastructure. The TODO comment at line 878 states:

```rust
// TODO(06-01): Queue plan via gRPC with metadata
// For full implementation, need to either:
// 1. Serialize GraphPlan and send via QueuePlan with plan_type="graph_plan"
// 2. Or convert to an existing plan type the server understands
```

**Impact:** Experiments designed in the node graph editor don't capture metadata, meaning:
- DATA-02 requirement only 50% satisfied (ScanBuilder works, ExperimentDesigner doesn't)
- Graph-based experiments have no provenance tracking
- Users designing complex experiments via the node editor lose metadata context

**What needs to be added:**
1. Serialize GraphPlan or translate to server-compatible plan format
2. Call `client.queue_plan()` or equivalent gRPC method
3. Pass `metadata` map in the request
4. Handle async response and update UI state

**Note:** This is not a verification failure of what was implemented — the plans (06-01 through 06-04) all completed successfully and their deliverables work. This is a **pre-existing limitation** from Phase 3 (Plan Translation and Execution), where graph-based plans were designed to execute locally in the GUI without server integration. The metadata capture work in 06-01 prepared ExperimentDesignerPanel for future server integration but didn't implement it (correctly noted in SUMMARY as a blocker/concern).

---

_Verified: 2026-01-22T20:30:00Z_
_Verifier: Claude (gsd-verifier)_
