//! HDF5 Storage Actor Example
//!
//! This example demonstrates how to use the HDF5 Storage actor in the V4 DAQ system.
//!
//! # Building
//! ```bash
//! cargo build --example hdf5_storage_example --features v4,storage_hdf5
//! ```
//!
//! # Running
//! ```bash
//! cargo run --example hdf5_storage_example --features v4,storage_hdf5
//! ```

use std::path::PathBuf;

// This example is feature-gated to compile with v4 enabled
#[cfg(feature = "v4")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use rust_daq::actors::{Flush, GetStats, HDF5Storage, SetInstrumentMetadata, SetMetadata};
    use rust_daq::config_v4::StorageConfig;

    println!("=== HDF5 Storage Actor Example ===\n");

    // Step 1: Create configuration
    println!("Step 1: Creating storage configuration...");
    let config = StorageConfig {
        default_backend: "hdf5".to_string(),
        output_dir: PathBuf::from("./example_data"),
        compression_level: 6,
        auto_flush_interval_secs: 0,
    };
    println!("  Output directory: {:?}", config.output_dir);
    println!("  Compression level: {}", config.compression_level);

    // Step 2: Create actor instance
    println!("\nStep 2: Creating HDF5Storage actor...");
    let _storage = HDF5Storage::new(&config);
    println!("  Actor created successfully");
    println!("  Note: In a real application, spawn the actor with Kameo:");
    println!("    let storage_ref = storage.spawn();");

    // Step 3: Demonstrate message types
    println!("\nStep 3: Message Types Available:");
    println!("  1. WriteBatch");
    println!("     - Purpose: Write Arrow RecordBatch to HDF5");
    println!("     - Format: WriteBatch {{ batch: Option<Vec<u8>>, instrument_id: String }}");
    println!();
    println!("  2. SetMetadata");
    println!("     - Purpose: Store session-level metadata");
    println!("     - Format: SetMetadata {{ key: String, value: String }}");
    println!();
    println!("  3. SetInstrumentMetadata");
    println!("     - Purpose: Store instrument-specific metadata");
    println!("     - Format: SetInstrumentMetadata {{ instrument_id, key, value }}");
    println!();
    println!("  4. Flush");
    println!("     - Purpose: Manually flush pending writes");
    println!("     - Format: Flush");
    println!();
    println!("  5. GetStats");
    println!("     - Purpose: Retrieve current storage statistics");
    println!("     - Format: GetStats");
    println!("     - Returns: StorageStats {{ bytes_written, batches_written, file_path, ... }}");

    // Step 4: Show configuration examples
    println!("\n=== Configuration Examples ===\n");

    println!("Example 1: Low Latency (Real-time)");
    println!("  compression_level: 0");
    println!("  auto_flush_interval_secs: 0");
    println!("  Best for: Real-time data acquisition with minimal overhead\n");

    println!("Example 2: Balanced (Recommended)");
    println!("  compression_level: 6");
    println!("  auto_flush_interval_secs: 30");
    println!("  Best for: Most DAQ applications\n");

    println!("Example 3: High Compression (Post-processing)");
    println!("  compression_level: 9");
    println!("  auto_flush_interval_secs: 300");
    println!("  Best for: Storage optimization, lower I/O priority\n");

    // Step 5: Show HDF5 file structure
    println!("=== Generated HDF5 File Structure ===\n");
    println!("daq_session_YYYYMMDD_HHMMSS.h5");
    println!("├─ Root Attributes");
    println!("│  ├─ created_at: \"2025-11-16T10:30:45Z\"");
    println!("│  ├─ created_timestamp_ns: 1731750645000000000");
    println!("│  └─ application: \"Rust DAQ V4\"");
    println!("│");
    println!("└─ Instrument Groups");
    println!("   ├─ power_meter_01 (Group)");
    println!("   │  ├─ Attributes");
    println!("   │  │  ├─ instrument_id: \"power_meter_01\"");
    println!("   │  │  └─ created_at: \"2025-11-16T10:30:45Z\"");
    println!("   │  │");
    println!("   │  └─ Datasets");
    println!("   │     ├─ power_watts (Float64 array)");
    println!("   │     ├─ timestamp_ns (Int64 array)");
    println!("   │     └─ schema (String)");
    println!("   │");
    println!("   └─ power_meter_02 (Group)");
    println!("      └─ [Similar structure]");

    // Step 6: Usage tips
    println!("\n=== Usage Tips ===\n");
    println!("1. Always set metadata early in the session for context");
    println!("2. Use descriptive instrument IDs (e.g., 'power_meter_01', not just 'pm1')");
    println!("3. Call Flush() periodically or use auto_flush_interval_secs");
    println!("4. Monitor GetStats() for storage health");
    println!(
        "5. Use appropriate compression for your throughput (0 for real-time, 6-9 for storage)"
    );

    // Step 7: Example workflow
    println!("\n=== Example Workflow ===\n");
    println!("Async workflow in your application:");
    println!();
    println!("  // Spawn actor");
    println!("  let storage_ref = HDF5Storage::new(&config).spawn();");
    println!();
    println!("  // Set session metadata");
    println!("  storage_ref.call(SetMetadata {{");
    println!("    key: \"experiment_id\".into(),");
    println!("    value: \"EXP_001\".into(),");
    println!("  }}).await?;");
    println!();
    println!("  // For each instrument");
    println!("  storage_ref.call(SetInstrumentMetadata {{");
    println!("    instrument_id: \"power_meter_01\".into(),");
    println!("    key: \"wavelength_nm\".into(),");
    println!("    value: \"633.0\".into(),");
    println!("  }}).await?;");
    println!();
    println!("  // Acquire and write data");
    println!("  let batch = create_arrow_batch()?;");
    println!("  storage_ref.call(WriteBatch {{");
    println!("    batch: Some(serialize_to_arrow_ipc(&batch)?,");
    println!("    instrument_id: \"power_meter_01\".into(),");
    println!("  }}).await?;");
    println!();
    println!("  // Periodically check stats");
    println!("  let stats = storage_ref.call(GetStats).await?;");
    println!("  println!(\"Written: {{}} batches, {{}} bytes\",");
    println!("    stats.batches_written, stats.bytes_written);");
    println!();
    println!("  // Manual flush (if not using auto-flush)");
    println!("  storage_ref.call(Flush).await?;");
    println!();
    println!("  // Actor automatically flushes and closes on shutdown");

    // Step 8: Building and inspecting files
    println!("\n=== Inspecting Generated Files ===\n");
    println!("After running the application:");
    println!();
    println!("  # View file structure");
    println!("  h5ls -r example_data/daq_session_*.h5");
    println!();
    println!("  # Dump complete file contents");
    println!("  h5dump example_data/daq_session_*.h5");
    println!();
    println!("  # Extract a specific dataset");
    println!("  h5dump -d /power_meter_01/power_watts example_data/daq_session_*.h5");
    println!();
    println!("  # View attributes");
    println!("  h5dump -A example_data/daq_session_*.h5");

    // Step 9: Troubleshooting
    println!("\n=== Troubleshooting ===\n");
    println!("Issue: \"Unable to locate HDF5 root directory\"");
    println!("Solution: Install HDF5 system library");
    println!("  macOS: brew install hdf5");
    println!("  Linux: sudo apt-get install libhdf5-dev");
    println!();
    println!("Issue: \"Failed to create output directory\"");
    println!("Solution: Ensure parent directory exists and is writable");
    println!("  mkdir -p example_data && chmod 755 example_data");
    println!();
    println!("Issue: \"HDF5 storage feature not enabled\"");
    println!("Solution: Build with storage_hdf5 feature");
    println!("  cargo build --features v4,storage_hdf5");

    println!("\n=== Example Complete ===\n");
    Ok(())
}

#[cfg(not(feature = "v4"))]
fn main() {
    println!("This example requires the 'v4' feature to be enabled.");
    println!("Build with: cargo build --example hdf5_storage_example --features v4,storage_hdf5");
}
