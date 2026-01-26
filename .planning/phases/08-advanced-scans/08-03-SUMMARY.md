# Phase 08 Plan 03: AdaptiveScan Node Type Summary

**One-liner:** AdaptiveScan node with trigger conditions (threshold/peak detection) and predefined actions (Zoom, MoveToPeak, etc.)

## What Was Built

Added AdaptiveScan node type for scans that automatically respond to acquired data during execution.

### New Types Added (crates/daq-egui/src/graph/nodes.rs)

1. **TriggerCondition enum** - Conditions that fire adaptive actions:
   - `Threshold` - Signal crosses a threshold (device_id, operator, value)
   - `PeakDetection` - Peak detected with prominence/height criteria

2. **AdaptiveAction enum** - Actions when trigger fires:
   - `Zoom2x` / `Zoom4x` - Narrow range and increase resolution
   - `MoveToPeak` - Move actuator to detected peak position
   - `AcquireAtPeak` - Trigger acquisition at peak
   - `MarkAndContinue` - Record peak, continue unchanged

3. **TriggerLogic enum** - How to combine multiple triggers:
   - `Any` (OR) - Fire if any trigger matches
   - `All` (AND) - Fire only if all triggers match

4. **AdaptiveScanConfig struct** - Configuration for AdaptiveScan node:
   - `scan: ScanDimension` - Base scan parameters
   - `triggers: Vec<TriggerCondition>` - One or more triggers
   - `trigger_logic: TriggerLogic` - AND/OR combination
   - `action: AdaptiveAction` - What to do when triggered
   - `require_approval: bool` - Pause for user confirmation

### Files Modified

| File | Changes |
|------|---------|
| `crates/daq-egui/Cargo.toml` | Added `find_peaks = "0.1"` dependency |
| `crates/daq-egui/src/graph/nodes.rs` | Added AdaptiveScan variant and config types |
| `crates/daq-egui/src/graph/codegen.rs` | Added Rhai code generation for AdaptiveScan |
| `crates/daq-egui/src/graph/translation.rs` | Added plan translation for AdaptiveScan |
| `crates/daq-egui/src/graph/viewer.rs` | Added inline editor for AdaptiveScan |
| `crates/daq-egui/src/widgets/node_palette.rs` | Added AdaptiveScan to palette (dark orange) |
| `crates/daq-egui/src/widgets/property_inspector.rs` | Added property inspector panel |
| `crates/daq-egui/src/panels/experiment_designer.rs` | Added validation and parameter collection |

### UI Features

**Node Palette:**
- AdaptiveScan appears with dark orange color indicator
- Description: "Scan that responds to data triggers"

**Property Inspector Panel:**
- Collapsible "Base Scan" section with actuator, start/stop/points
- Collapsible "Triggers" section with:
  - Logic selector (Any/All)
  - Trigger list with type selector (Threshold/Peak Detection)
  - Add/Remove trigger buttons (minimum 1 required)
- Action dropdown with 5 predefined options
- "Require approval before action" checkbox

**Inline Node Editor:**
- Compact trigger configuration
- Real-time validation feedback

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Use find_peaks crate | scipy-equivalent prominence-based peak detection |
| Fallback scan in codegen | Runtime trigger evaluation deferred to future |
| require_approval default false | Most adaptive scans auto-execute |
| Minimum 1 trigger required | Prevents misconfigured nodes |

## Deviations from Plan

**1. [Rule 3 - Blocking] Property inspector module structure**
- Plan specified creating `property_inspector/` directory with `mod.rs` and `adaptive_scan_panel.rs`
- Actual: Added methods directly to existing `property_inspector.rs` single file
- Reason: Consistent with existing codebase pattern, simpler structure

## Verification Results

- [x] AdaptiveScan node visible in palette
- [x] Threshold trigger configurable (device, operator, value)
- [x] Peak detection trigger configurable (device, prominence, height)
- [x] Action dropdown shows all 5 options
- [x] Multiple triggers combinable with AND/OR
- [x] Require approval checkbox functional
- [x] Serialization/deserialization works (Serde derives)

## Known Limitations

1. **Runtime trigger evaluation not implemented** - AdaptiveScan falls back to basic scan during execution. Full trigger evaluation requires RunEngine runtime support.

2. **find_peaks integration pending** - The find_peaks crate is added but not yet wired into the trigger evaluation pipeline.

## Next Steps

- Implement runtime trigger evaluation in RunEngine
- Wire find_peaks into PeakDetection trigger
- Add visual feedback when triggers fire during execution
- Consider adding custom trigger condition support

## Commit

Commit: f761e06f (style: fix clippy warnings - suppress FFI/unsafe and fix easy wins)
Note: Changes were included as part of a larger commit that also addressed clippy warnings.

## Duration

Plan execution: ~15 minutes
