# V4 GUI Integration - Quick Reference

## Files Created

| File | Lines | Purpose |
|------|-------|---------|
| `src/gui/v4_data_bridge.rs` | 330 | Bridge Arrow data to GUI (DataConsumer impl) |
| `src/gui/v4_instrument_panel.rs` | 280 | egui panels for real-time visualization |
| `src/gui/mod.rs` | ±2 | Module exports (updated) |
| `docs/v4_gui_integration.md` | 450+ | Complete integration guide |
| `examples/v4_gui_integration.rs` | 70 | Usage examples |
| `V4_GUI_IMPLEMENTATION_SUMMARY.md` | - | This summary |
| `V4_GUI_QUICK_REFERENCE.md` | - | This reference |

## Compile

```bash
# Check compilation
cargo check --features v4

# Compile
cargo build --features v4

# Run example
cargo run --example v4_gui_integration --features v4

# Run tests
cargo test --features v4 v4_data_bridge::tests
cargo test --features v4 v4_instrument_panel::tests
```

## Basic Usage

### 1. Create Bridge

```rust
use rust_daq::gui::v4_data_bridge::V4DataBridge;

let bridge = V4DataBridge::default_capacity(); // 1000 measurements/instrument
```

### 2. Subscribe to DataPublisher

```rust
use std::sync::Arc;

let bridge_consumer: Arc<dyn DataConsumer> = Arc::new(bridge.clone());
let sub_id = publisher.ask(Subscribe { subscriber: bridge_consumer }).await?;
```

### 3. Display in Panel

```rust
use rust_daq::gui::v4_instrument_panel::V4InstrumentPanel;

let mut panel = V4InstrumentPanel::new("newport_1830c".to_string());

egui::CentralPanel::default().show(ctx, |ui| {
    panel.ui(ui, &bridge);
});
```

## API Overview

### V4DataBridge

```rust
// Creation
let bridge = V4DataBridge::new(1000);
let bridge = V4DataBridge::default_capacity();

// Reading
bridge.get_measurements("instrument_id")     // → VecDeque<GuiMeasurement>
bridge.get_latest("instrument_id")           // → Option<GuiMeasurement>
bridge.get_statistics("instrument_id")       // → Option<(min, max, mean)>
bridge.instruments()                         // → Vec<String>

// Management
bridge.clear("instrument_id")
bridge.clear_all()
```

### GuiMeasurement

```rust
pub struct GuiMeasurement {
    pub timestamp_ns: i64,
    pub power: f64,
    pub unit: String,
    pub wavelength_nm: Option<f64>,
}
```

### V4InstrumentPanel

```rust
// Creation
let mut panel = V4InstrumentPanel::new("instrument_id".to_string());

// Rendering
panel.ui(ui, &bridge);  // Call in egui update loop
```

### V4Dashboard

```rust
// Creation
let mut dashboard = V4Dashboard::new();

// Management
dashboard.add_instrument("inst_id".to_string());
dashboard.remove_instrument("inst_id");

// Rendering
dashboard.ui(ui, &bridge);
```

## Data Schema

Arrow RecordBatch columns (in order):

| Index | Name | Type | Nullable |
|-------|------|------|----------|
| 0 | timestamp | Timestamp(Nanosecond) | No |
| 1 | power | Float64 | No |
| 2 | unit | Utf8 | No |
| 3 | wavelength_nm | Float64 | Yes |

## Thread Safety

- `V4DataBridge` is `Clone + Send + Sync`
- Safe for use across threads
- Actor thread: calls `handle_batch()`
- GUI thread: calls `get_measurements()`, `get_latest()`, etc
- No synchronization needed between calls

## Performance

| Operation | Complexity | Time |
|-----------|-----------|------|
| `handle_batch()` | O(n) | ~1-10µs for n=50 |
| `get_measurements()` | O(1) | <1µs |
| `get_latest()` | O(1) | <1µs |
| `get_statistics()` | O(n) | ~10-100µs |

Memory: ~80 bytes per measurement × 1000 × 10 instruments = 800 KB

## Configuration

### Ringbuffer Capacity

```rust
// High-frequency instruments (100+ meas/sec)
let bridge = V4DataBridge::new(2000);

// Low-frequency instruments
let bridge = V4DataBridge::new(500);

// Default (general purpose)
let bridge = V4DataBridge::default_capacity();  // 1000
```

## Common Patterns

### Single Instrument

```rust
let bridge = V4DataBridge::default_capacity();
let mut panel = V4InstrumentPanel::new("newport_1830c".to_string());

egui::CentralPanel::default().show(ctx, |ui| {
    panel.ui(ui, &bridge);
});
```

### Multiple Instruments

```rust
let bridge = V4DataBridge::default_capacity();
let mut dashboard = V4Dashboard::new();

egui::SidePanel::right("instruments").show(ctx, |ui| {
    dashboard.ui(ui, &bridge);
});
```

### Custom Statistics Display

```rust
if let Some((min, max, mean)) = bridge.get_statistics("instrument_id") {
    ui.label(format!("Min: {:.3}", min));
    ui.label(format!("Max: {:.3}", max));
    ui.label(format!("Mean: {:.3}", mean));
}
```

### Latest Value Display

```rust
if let Some(m) = bridge.get_latest("instrument_id") {
    ui.label(format!("{:.6} {}", m.power, m.unit));
}
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| "Invalid timestamp column" | Verify RecordBatch column order (see schema) |
| GUI not updating | Check subscription to DataPublisher |
| Memory usage high | Reduce capacity: `V4DataBridge::new(500)` |
| Missing data | Verify instrument_id matches in bridge calls |

## Testing

```bash
# Unit tests
cargo test --features v4 v4_data_bridge::
cargo test --features v4 v4_instrument_panel::

# Example
cargo run --example v4_gui_integration --features v4
```

## Documentation

- **Full Guide**: `docs/v4_gui_integration.md`
- **Implementation Details**: `V4_GUI_IMPLEMENTATION_SUMMARY.md`
- **DataConsumer Trait**: `src/actors/data_publisher.rs`
- **PowerMeter Schema**: `src/traits/power_meter.rs`

## Key Structs

```rust
// Main bridge
pub struct V4DataBridge { ... }

// Measurement data
pub struct GuiMeasurement {
    pub timestamp_ns: i64,
    pub power: f64,
    pub unit: String,
    pub wavelength_nm: Option<f64>,
}

// Panel for single instrument
pub struct V4InstrumentPanel { ... }

// Dashboard for multiple instruments
pub struct V4Dashboard { ... }
```

## Feature Flag

Require `v4` feature in Cargo.toml:

```toml
rust_daq = { path = ".", features = ["v4"] }
```

Or build with:

```bash
cargo build --features v4
```

## Example Code

See `examples/v4_gui_integration.rs` for complete examples:

```bash
cargo run --example v4_gui_integration --features v4
```

## Next Steps

1. Enable V4 feature in your build
2. Create V4DataBridge
3. Subscribe to DataPublisher
4. Create panel or dashboard
5. Call `ui()` in egui update loop
6. Run with `--features v4`

For questions, see full documentation in `docs/v4_gui_integration.md`
