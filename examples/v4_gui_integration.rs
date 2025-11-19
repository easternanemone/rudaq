//! V4 GUI Integration Example
//!
//! Demonstrates how to integrate V4DataBridge with the GUI system.
//! This example shows:
//! 1. Creating a V4DataBridge
//! 2. Subscribing it to the DataPublisher
//! 3. Using it in the GUI to display real-time data

#[cfg(feature = "v4")]
fn main() {
    use rust_daq::actors::data_publisher::DataConsumer;
    use rust_daq::gui::v4_data_bridge::V4DataBridge;
    use std::sync::Arc;

    println!("V4 GUI Integration Example");
    println!("==========================\n");

    // Step 1: Create a V4DataBridge with default capacity (1000 measurements per instrument)
    let bridge = V4DataBridge::default_capacity();
    println!("Created V4DataBridge with 1000-measurement capacity per instrument");

    // Step 2: Bridge can be subscribed to DataPublisher
    let bridge_arc: Arc<dyn DataConsumer> = Arc::new(bridge.clone());
    println!(
        "Bridge ready for subscription: {:?}",
        Arc::strong_count(&bridge_arc)
    );

    // Step 3: In real usage, the bridge would receive data from DataPublisher:
    //
    // let publisher = spawn(DataPublisher::new());
    // let subscriber_id = publisher
    //     .ask(Subscribe {
    //         subscriber: bridge_arc.clone(),
    //     })
    //     .await
    //     .expect("Failed to subscribe");
    //
    // When DataPublisher receives batches, it calls bridge.handle_batch()
    // which updates the internal ringbuffers

    println!("\n--- Usage Pattern in GUI ---");
    println!("1. Create bridge: let bridge = V4DataBridge::default_capacity();");
    println!("2. Subscribe: publisher.ask(Subscribe {{ subscriber: bridge_arc }})");
    println!("3. In render loop:");
    println!("   - Get measurements: bridge.get_measurements(instrument_id)");
    println!("   - Get stats: bridge.get_statistics(instrument_id)");
    println!("   - Get latest: bridge.get_latest(instrument_id)");

    println!("\n--- Panel Integration Example ---");
    println!(
        "
let mut panel = V4InstrumentPanel::new(\"newport_1830c\".to_string());

// In update loop (called every frame):
egui::CentralPanel::default().show(ctx, |ui| {{
    panel.ui(ui, &bridge);
}});

// The panel automatically:
// - Syncs data from bridge
// - Displays live plot
// - Shows status and statistics
// - Calculates measurement rate
"
    );

    println!("\n--- Multi-Instrument Dashboard ---");
    println!(
        "
let mut dashboard = V4Dashboard::new();
dashboard.add_instrument(\"newport_1830c\".to_string());
dashboard.add_instrument(\"maitai\".to_string());

egui::SidePanel::right(\"v4_dashboard\").show(ctx, |ui| {{
    dashboard.ui(ui, &bridge);
}});
"
    );

    println!("\n--- Data Access Patterns ---");
    println!("// Get last measurement");
    println!("if let Some(measurement) = bridge.get_latest(\"instrument_id\") {{");
    println!("    println!(\"Power: {{}} {{}}\", measurement.power, measurement.unit);");
    println!("}}");
    println!();
    println!("// Get statistics");
    println!("if let Some((min, max, mean)) = bridge.get_statistics(\"instrument_id\") {{");
    println!("    println!(\"Min: {{}}, Max: {{}}, Mean: {{}}\", min, max, mean);");
    println!("}}");
    println!();
    println!("// Get all measurements for plot");
    println!("let measurements = bridge.get_measurements(\"instrument_id\");");
    println!("for m in measurements {{");
    println!("    println!(\"{{}} ns: {{}}\", m.timestamp_ns, m.power);");
    println!("}}");
}

#[cfg(not(feature = "v4"))]
fn main() {
    println!("This example requires the 'v4' feature: cargo run --example v4_gui_integration --features v4");
}
