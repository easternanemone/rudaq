# V4 GUI Integration Guide

## Overview

The V4 GUI integration provides a bridge between the V4 actor-based DataPublisher and the egui GUI system. It enables real-time visualization of Arrow-formatted measurement data with minimal latency.

## Architecture

```
DataPublisher Actor
    ↓ (PublishBatch)
V4DataBridge (DataConsumer trait)
    ↓ (Arc<Mutex<HashMap>>)
GUI Thread (V4InstrumentPanel)
    ↓ (render)
egui Plot & Status Display
```

### Key Components

1. **V4DataBridge**: Thread-safe consumer that receives Arrow RecordBatch updates
2. **V4InstrumentPanel**: egui panel for displaying real-time power measurements
3. **V4Dashboard**: Multi-instrument status dashboard

## Quick Start

### 1. Enable the V4 Feature

```bash
cargo build --features v4
# or
cargo run --features v4
```

### 2. Create the Bridge

```rust
use rust_daq::gui::v4_data_bridge::V4DataBridge;

let bridge = V4DataBridge::default_capacity(); // 1000 measurements per instrument
```

### 3. Subscribe to DataPublisher

```rust
use std::sync::Arc;
use rust_daq::actors::data_publisher::{Subscribe, DataConsumer};

let bridge_consumer: Arc<dyn DataConsumer> = Arc::new(bridge.clone());

let subscriber_id = publisher
    .ask(Subscribe {
        subscriber: bridge_consumer,
    })
    .await?;
```

### 4. Display in GUI

```rust
use rust_daq::gui::v4_instrument_panel::V4InstrumentPanel;
use eframe::egui;

let mut panel = V4InstrumentPanel::new("newport_1830c".to_string());

// In your update loop:
egui::CentralPanel::default().show(ctx, |ui| {
    panel.ui(ui, &bridge);
});
```

## API Reference

### V4DataBridge

Thread-safe bridge for consuming Arrow data and maintaining measurement history.

#### Creating a Bridge

```rust
// With custom capacity
let bridge = V4DataBridge::new(500); // Keep 500 measurements per instrument

// With default capacity (1000)
let bridge = V4DataBridge::default_capacity();
```

#### Reading Data

```rust
// Get all measurements for an instrument
let measurements = bridge.get_measurements("instrument_id");
for m in measurements {
    println!("{}: {} {}", m.timestamp_ns, m.power, m.unit);
}

// Get the latest measurement
if let Some(measurement) = bridge.get_latest("instrument_id") {
    println!("Latest: {} {}", measurement.power, measurement.unit);
}

// Get statistics
if let Some((min, max, mean)) = bridge.get_statistics("instrument_id") {
    println!("Min: {}, Max: {}, Mean: {}", min, max, mean);
}

// Get list of instruments with data
let instruments = bridge.instruments();
for id in instruments {
    println!("Instrument: {}", id);
}
```

#### Managing Data

```rust
// Clear measurements for one instrument
bridge.clear("instrument_id");

// Clear all measurements
bridge.clear_all();
```

### GuiMeasurement

Represents a single measurement in GUI-friendly format.

```rust
pub struct GuiMeasurement {
    pub timestamp_ns: i64,      // Nanoseconds since epoch
    pub power: f64,              // Power value
    pub unit: String,            // Unit name ("MilliWatts", "Watts", etc)
    pub wavelength_nm: Option<f64>, // Optional wavelength
}
```

### V4InstrumentPanel

Single instrument panel with plot and statistics.

```rust
// Create panel for an instrument
let mut panel = V4InstrumentPanel::new("newport_1830c".to_string());

// Render in UI (call every frame)
egui::CentralPanel::default().show(ctx, |ui| {
    panel.ui(ui, &bridge);
});
```

Features:
- Live power plot with egui_plot
- Connection status indicator
- Real-time statistics (min, max, mean, latest)
- Measurement rate calculation
- Automatic data synchronization from bridge

### V4Dashboard

Multi-instrument status dashboard.

```rust
// Create dashboard
let mut dashboard = V4Dashboard::new();

// Add instruments
dashboard.add_instrument("newport_1830c".to_string());
dashboard.add_instrument("maitai".to_string());

// Render in UI
egui::SidePanel::right("v4_dashboard").show(ctx, |ui| {
    dashboard.ui(ui, &bridge);
});
```

Features:
- Expandable/collapsible instrument cards
- Connection status indicators
- Quick statistics view
- Automatic instrument discovery from bridge

## Data Conversion

### Arrow to GUI Format

The bridge automatically converts Arrow RecordBatch to GuiMeasurement:

```
Arrow RecordBatch Schema:
├── timestamp: Timestamp(Nanosecond)
├── power: Float64
├── unit: Utf8
└── wavelength_nm: Float64 (nullable)
        ↓
GuiMeasurement {
  timestamp_ns: i64,
  power: f64,
  unit: String,
  wavelength_nm: Option<f64>,
}
```

### Column Indices

The bridge expects RecordBatch columns in this order:
- Column 0: `timestamp` (TimestampNanosecondArray)
- Column 1: `power` (Float64Array)
- Column 2: `unit` (StringArray)
- Column 3: `wavelength_nm` (Float64Array)

This matches the standard power meter Arrow schema from `PowerMeter::to_arrow()`.

## Thread Safety

The bridge is designed for concurrent access:

```rust
// Safe to share across threads
let bridge = Arc::new(V4DataBridge::default_capacity());

// Actor thread: subscribe and send data
let bridge_consumer = Arc::clone(&bridge) as Arc<dyn DataConsumer>;
publisher.ask(Subscribe { subscriber: bridge_consumer }).await?;

// GUI thread: read data without blocking
let measurements = bridge.get_measurements("instrument_id");
```

**Implementation Details:**
- Uses `Arc<Mutex<HashMap>>` for interior mutability
- Lock is held only during HashMap access, not during clone operations
- Cloning VecDeque<GuiMeasurement> is fast (small payload)
- No blocking between actor and GUI threads

## Performance Considerations

### Memory Usage

Default capacity (1000 measurements per instrument):
- Per measurement: ~80 bytes
- Per instrument: ~80 KB
- 10 instruments: ~800 KB

Adjust capacity based on measurement rate and memory constraints:

```rust
// For high-frequency instruments (100+ measurements/sec):
let bridge = V4DataBridge::new(2000);

// For low-frequency instruments:
let bridge = V4DataBridge::new(500);
```

### CPU Usage

- `handle_batch()`: O(n) where n = batch size (typically 10-100 rows)
- `get_measurements()`: O(1) clone of internal VecDeque
- `get_statistics()`: O(n) linear scan of measurements
- `sync_from_bridge()`: O(n) copy to plot cache (unavoidable for plotting)

### Data Freshness

- Bridge maintains a ringbuffer of recent measurements
- GUI reads latest data on each frame update
- No built-in downsampling (plot library does this)

## Integration Examples

### Basic Single Instrument Panel

```rust
let mut panel = V4InstrumentPanel::new("newport_1830c".to_string());

egui::Window::new("Power Meter")
    .resizable(true)
    .default_size([600.0, 400.0])
    .show(ctx, |ui| {
        panel.ui(ui, &bridge);
    });
```

### Dashboard with Multiple Instruments

```rust
let mut dashboard = V4Dashboard::new();

egui::SidePanel::right("instruments")
    .resizable(true)
    .min_width(300.0)
    .show(ctx, |ui| {
        dashboard.ui(ui, &bridge);
    });
```

### Custom Statistics Display

```rust
if let Some((min, max, mean)) = bridge.get_statistics("instrument_id") {
    ui.group(|ui| {
        ui.heading("Statistics");
        ui.label(format!("Min: {:.6}", min));
        ui.label(format!("Max: {:.6}", max));
        ui.label(format!("Mean: {:.6}", mean));
    });
}
```

### Trend Detection

```rust
let measurements = bridge.get_measurements("instrument_id");
if measurements.len() >= 2 {
    let latest = measurements.back().unwrap().power;
    let previous = measurements.get(measurements.len() - 2).unwrap().power;

    if latest > previous * 1.1 {
        ui.colored_label(egui::Color32::RED, "Power increasing!");
    }
}
```

## Testing

### Unit Tests

The modules include comprehensive tests:

```bash
cargo test --features v4 v4_data_bridge::tests
cargo test --features v4 v4_instrument_panel::tests
```

### Integration Testing

See `examples/v4_gui_integration.rs` for usage patterns:

```bash
cargo run --example v4_gui_integration --features v4
```

## Common Issues

### "Invalid timestamp column" Error

**Cause**: RecordBatch has wrong column order or type.

**Solution**: Ensure batch matches expected schema:
```rust
// Correct schema
Field::new("timestamp", DataType::Timestamp(TimeUnit::Nanosecond, None), false),
Field::new("power", DataType::Float64, false),
Field::new("unit", DataType::Utf8, false),
Field::new("wavelength_nm", DataType::Float64, true),
```

### GUI Not Updating

**Cause**: Bridge not subscribed to DataPublisher.

**Solution**:
```rust
let subscriber_id = publisher
    .ask(Subscribe { subscriber: bridge_arc })
    .await?;
println!("Subscribed: {}", subscriber_id);
```

### Memory Leaks

**Cause**: Not clearing old instruments.

**Solution**:
```rust
if some_condition {
    bridge.clear("old_instrument_id");
}

// Or periodically clean up:
for inst_id in bridge.instruments() {
    if should_remove(&inst_id) {
        bridge.clear(&inst_id);
    }
}
```

## Future Enhancements

1. **Adaptive Downsampling**: Automatically downsample for efficiency
2. **Data Export**: Export measurements to CSV/HDF5
3. **Alarm Thresholds**: Visual indicators for out-of-range values
4. **Time Window Selection**: User-configurable plot window
5. **Multi-Plot Comparison**: Side-by-side instrument comparison
6. **Data Logging**: Background logging to persistent storage

## Architecture Notes

### Why VecDeque?

- Efficient FIFO ringbuffer
- O(1) push_back and pop_front
- Memory-efficient wraparound
- Good cache locality for iteration

### Why Clone on Read?

- No lifetime issues for GUI rendering
- Decouples bridge update from GUI rendering
- Small payload (~80 bytes per measurement)
- Avoids long lock holds

### Why Mutex over RwLock?

- Simple, predictable performance
- Lock held for short periods (<1ms)
- Contention unlikely (actor rarely updates during frame render)
- RwLock overhead not justified for this workload

## Related Documentation

- [PowerMeter Trait](../src/traits/power_meter.rs) - Arrow schema definition
- [DataPublisher Actor](../src/actors/data_publisher.rs) - Publisher implementation
- [GUI Architecture](./mod.rs) - Main GUI module documentation
- [V4 Architecture](./v4_system_architecture.md) - V4 system overview
