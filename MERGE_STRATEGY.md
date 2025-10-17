
---

## MERGE_STRATEGY.md

# Unified Merge Strategy for PR #36 and PR #38

## Executive Summary

This document outlines the unified merge strategy for Pull Requests #36 and #38 in the `rust-daq` repository. Both PRs implement complementary features for real-time instrument data display through a centralized data caching architecture. The PRs contain no destructive conflicts and can be cleanly integrated into a single coherent implementation.

## Overview of Pull Requests

### PR #36: "feat(gui): Display real-time instrument status in left panel"
- **Branch**: bd-31-instrument-status
- **Purpose**: Implements real-time data display for instrument cards in the left sidebar
- **Key Features**:
  - Subscribes to instrument data streams
  - Caches the latest values
  - Displays real-time status in the UI with immediate feedback

### PR #38: "Refactor GUI to use unidirectional data flow"
- **Branch**: bd-30-unidirectional-data-flow
- **Purpose**: Refactors the GUI to use unidirectional data flow for state synchronization
- **Key Features**:
  - Implements a centralized `data_cache` in the `Gui` struct
  - Makes instrument control panels stateless
  - Ensures UI always reflects confirmed hardware state
  - Removes optimistic UI updates

## Conflict Analysis

### Primary Conflict Area: `src/gui/mod.rs`

Both PRs modify the core GUI module with overlapping but complementary changes:

**Specific Conflicts:**
1. **Gui struct definition** - Both add `data_cache: HashMap<String, DataPoint>`
2. **Gui::new() initialization** - Both initialize the cache
3. **update_data() method** - Both update the cache with incoming data points
4. **render_instrument_panel() function** - Both add cache-related parameters and logic
5. **Helper functions** - PR #36 adds `display_cached_value()` function

### Secondary Conflict Areas

**src/gui/instrument_controls.rs**
- Import ordering: `use egui::{Color32, Slider, Ui};`
- Code formatting and line breaks for readability
- No functional logic conflicts

**Instrument Implementation Files** (maitai.rs, newport_1830c.rs, elliptec.rs, esp300.rs, pvcam.rs)
- Identical formatting improvements in both PRs
- Line-break adjustments for code readability
- No functional changes

**src/app.rs**
- Method signature formatting in `send_instrument_command()`
- Identical formatting in both PRs

**src/main.rs**
- Import statement reordering
- Alphabetical ordering of instrument imports

## Unified Resolution Strategy

### Resolution Priority

| File | Conflict Type | Priority | Resolution |
|------|---------------|----------|-----------|
| src/gui/mod.rs | Structural (caching & display) | HIGH | **MERGE** - Combine both approaches |
| src/gui/instrument_controls.rs | Formatting | MEDIUM | **KEEP** - All formatting improvements |
| src/instrument/*.rs | Formatting | MEDIUM | **KEEP** - All formatting improvements |
| src/app.rs | Formatting | LOW | **KEEP** - Formatting from either PR |
| src/main.rs | Formatting | LOW | **KEEP** - Import reordering |

### Detailed Resolution for Primary Conflict

#### 1. Gui Struct Definition
**Action**: Add the `data_cache` field from PR #38
```rust
pub struct Gui {
    // ... existing fields ...
    data_cache: HashMap<String, DataPoint>,
    // ... existing fields ...
}
```

#### 2. Gui::new() Implementation
**Action**: Initialize the cache in the constructor
```rust
impl Gui {
    pub fn new(app: DaqApp) -> Self {
        // ... existing initialization code ...
        Self {
            // ... existing fields ...
            data_cache: HashMap::new(),
            // ... existing fields ...
        }
    }
}
```

#### 3. update_data() Method
**Action**: Combine both implementations
```rust
fn update_data(&mut self) {
    while let Ok(data_point) = self.data_receiver.try_recv() {
        // Update plot tabs (from PR #38 base logic)
        for (_location, tab) in self.dock_state.iter_all_tabs_mut() {
            if let DockTab::Plot(plot_tab) = tab {
                if plot_tab.channel == data_point.channel {
                    if plot_tab.plot_data.len() >= PLOT_DATA_CAPACITY {
                        plot_tab.plot_data.pop_front();
                    }
                    let timestamp =
                        data_point.timestamp.timestamp_micros() as f64 / 1_000_000.0;
                    if plot_tab.last_timestamp == 0.0 {
                        plot_tab.last_timestamp = timestamp;
                    }
                    plot_tab
                        .plot_data
                        .push_back([timestamp - plot_tab.last_timestamp, data_point.value]);
                }
            }
        }
        
        // Update the cache with the latest data point (from PR #38)
        self.data_cache
            .insert(data_point.channel.clone(), data_point);
    }
}
```

#### 4. render_instrument_panel() Signature
**Action**: Add the `data_cache` parameter from PR #36
```rust
fn render_instrument_panel(
    ui: &mut egui::Ui,
    instruments: &[(String, toml::Value, bool)],
    app: &DaqApp,
    dock_state: &mut DockState<DockTab>,
    data_cache: &HashMap<String, DataPoint>,
) {
    // ... implementation ...
}
```

#### 5. Add Helper Function
**Action**: Include the `display_cached_value()` helper from PR #36
```rust
/// Helper function to display a cached value in the UI
fn display_cached_value(
    ui: &mut egui::Ui,
    data_cache: &HashMap<String, DataPoint>,
    channel: &str,
    label: &str,
) {
    if let Some(data_point) = data_cache.get(channel) {
        ui.label(format!(
            "{}: {:.3} {}",
            label, data_point.value, data_point.unit
        ));
    } else {
        ui.label(format!("{}: No data", label));
    }
}
```

#### 6. Update render_instrument_panel() Implementation
**Action**: Include all cache display calls from PR #36 for each instrument type

**For MaiTai instruments:**
```rust
"maitai" => {
    ui.separator();
    if let Some(wl) = config.get("wavelength").and_then(|v| v.as_float()) {
        ui.label(format!("Wavelength: {:.1} nm", wl));
    }
    if let Some(port) = config.get("port").and_then(|v| v.as_str()) {
        ui.label(format!("Port: {}", port));
    }
    
    // Display real-time power and wavelength from data stream
    display_cached_value(
        ui,
        data_cache,
        &format!("{}_power", id),
        "Power",
    );
    display_cached_value(
        ui,
        data_cache,
        &format!("{}_wavelength", id),
        "Wavelength",
    );
    display_cached_value(
        ui,
        data_cache,
        &format!("{}_shutter", id),
        "Shutter",
    );
    ui.label("💡 Drag to main area or double-click");
}
```

**For Newport 1830-C instruments:**
```rust
"newport_1830c" => {
    ui.separator();
    if let Some(wl) = config.get("wavelength").and_then(|v| v.as_float()) {
        ui.label(format!("Wavelength: {:.1} nm", wl));
    }
    if let Some(port) = config.get("port").and_then(|v| v.as_str()) {
        ui.label(format!("Port: {}", port));
    }
    // Display real-time power reading
    display_cached_value(
        ui,
        data_cache,
        &format!("{}_power", id),
        "Power",
    );
    ui.label("💡 Drag to main area or double-click");
}
```

**For Elliptec instruments:**
```rust
"elliptec" => {
    ui.separator();
    if let Some(port) = config.get("port").and_then(|v| v.as_str()) {
        ui.label(format!("Port: {}", port));
    }
    if let Some(addrs) = config.get("device_addresses").and_then(|v| v.as_array()) {
        ui.label(format!("Devices: {}", addrs.len()));
        for addr in addrs.iter().filter_map(|a| a.as_integer()) {
            display_cached_value(
                ui,
                data_cache,
                &format!("{}_device{}_position", id, addr),
                &format!("Device {}", addr),
            );
        }
    }
    ui.label("💡 Drag to main area or double-click");
}
```

**For ESP300 instruments:**
```rust
"esp300" => {
    ui.separator();
    if let Some(port) = config.get("port").and_then(|v| v.as_str()) {
        ui.label(format!("Port: {}", port));
    }
    let num_axes = config
        .get("num_axes")
        .and_then(|v| v.as_integer())
        .unwrap_or(3) as usize;
    ui.label(format!("Axes: {}", num_axes));
    for axis in 1..=num_axes as u8 {
        display_cached_value(
            ui,
            data_cache,
            &format!("{}_axis{}_position", id, axis),
            &format!("Axis {} Pos", axis),
        );
        display_cached_value(
            ui,
            data_cache,
            &format!("{}_axis{}_velocity", id, axis),
            &format!("Axis {} Vel", axis),
        );
    }
    ui.label("💡 Drag to main area or double-click");
}
```

**For PVCAM instruments:**
```rust
"pvcam" => {
    ui.separator();
    if let Some(cam) = config.get("camera_name").and_then(|v| v.as_str()) {
        ui.label(format!("Camera: {}", cam));
    }
    if let Some(exp) = config.get("exposure_ms").and_then(|v| v.as_float()) {
        ui.label(format!("Exposure: {} ms", exp));
    }
    // Display acquisition status
    display_cached_value(
        ui,
        data_cache,
        &format!("{}_mean_intensity", id),
        "Mean",
    );
    display_cached_value(
        ui,
        data_cache,
        &format!("{}_min_intensity", id),
        "Min",
    );
    display_cached_value(
        ui,
        data_cache,
        &format!("{}_max_intensity", id),
        "Max",
    );
    ui.label("💡 Drag to main area or double-click");
}
```

### Secondary Resolutions

#### Formatting Changes (All Secondary Files)
**Action**: Keep all formatting improvements from both PRs
- Import reordering (alphabetical)
- Line-break improvements for readability
- Consistent code formatting

**Files affected:**
- src/gui/instrument_controls.rs
- src/instrument/maitai.rs
- src/instrument/newport_1830c.rs
- src/instrument/elliptec.rs
- src/instrument/esp300.rs
- src/instrument/pvcam.rs
- src/app.rs
- src/main.rs

## Integration Points

### How the Two Approaches Work Together

```
Data Flow Architecture:
┌─────────────────────────────────────────────────┐
│  Instrument Data Streams                        │
└────────────────────┬────────────────────────────┘
                     │
                     ▼
        ┌─────────────────────────┐
        │  Broadcast Channel      │
        │  (tokio::sync::broadcast)
        └────────┬────────────────┘
                 │
                 ▼
      ┌──────────────────────────┐
      │  Gui::update_data()      │ (from PR #38)
      │  - Processes data points │
      │  - Updates plot tabs     │
      │  - Caches latest values  │
      └────────┬─────────────────┘
               │
               ▼
    ┌─────────────────────────────────┐
    │  Gui.data_cache                 │
    │  HashMap<String, DataPoint>     │
    │  Single source of truth for UI  │
    └────────┬────────────────────────┘
             │
             ▼
    ┌────────────────────────────────────────┐
    │  render_instrument_panel()             │ (from PR #36)
    │  - Reads from data_cache               │
    │  - Displays cached values in left panel│
    │  - Shows real-time instrument status   │
    └────────────────────────────────────────┘
             │
             ▼
    ┌────────────────────────────────────────┐
    │  UI Display                            │
    │  - Real-time status indicators         │
    │  - Power readings                      │
    │  - Position values                     │
    │  - Velocity readings                   │
    └────────────────────────────────────────┘
```

### Key Benefits of This Architecture

1. **Unidirectional Data Flow** (from PR #38)
   - Single source of truth (data_cache)
   - Eliminates state desynchronization
   - Simplifies debugging and testing

2. **Real-Time Display** (from PR #36)
   - Immediate UI feedback on instrument status
   - No polling overhead
   - Cached values are always current

3. **Stateless UI Components**
   - Simpler component logic
   - Reduced memory footprint
   - Easier to reason about UI state

4. **Extensibility**
   - Easy to add new instrument types
   - Simple to add new display fields
   - Helper function (`display_cached_value`) reduces boilerplate

## Implementation Checklist

- [ ] Back up current main branch
- [ ] Create new branch from main: `merge/unify-data-flow-and-status`
- [ ] Apply changes to src/gui/mod.rs (combine both approaches)
- [ ] Apply all formatting changes from both PRs to secondary files
- [ ] Verify imports and dependencies are correct
- [ ] Run cargo check to verify compilation
- [ ] Run cargo test to verify functionality
- [ ] Create new PR with combined changes
- [ ] Reference both PR #36 and PR #38 in the new PR description
- [ ] Close PR #36 and PR #38 with reference to unified PR

## Testing Strategy

### Unit Tests
- Verify `data_cache` is populated correctly in `update_data()`
- Test `display_cached_value()` with valid and missing data points
- Verify cache updates don't interfere with plot rendering

### Integration Tests
- Test complete data flow from instrument to display
- Verify UI updates reflect latest cached values
- Test with multiple simultaneous instruments
- Test cache behavior with dropped instruments

### Manual Testing
- Verify real-time status displays update smoothly
- Check all instrument types display correctly
- Confirm no performance degradation
- Validate state synchronization between hardware and UI

## Expected Outcomes

After implementing this unified merge strategy:

1. **Single coherent architecture** combining both PR features
2. **Elimination of merge conflicts** through strategic combination
3. **Enhanced user experience** with real-time instrument feedback
4. **Improved code maintainability** through unidirectional data flow
5. **Foundation for future enhancements** to real-time monitoring

## References

- **PR #36**: https://github.com/TheFermiSea/rust-daq/pull/36
- **PR #38**: https://github.com/TheFermiSea/rust-daq/pull/38
- **Repository**: https://github.com/TheFermiSea/rust-daq

---

This comprehensive strategy document provides all the information needed to successfully merge the two pull requests into a single, unified implementation that combines the best aspects of both approaches.