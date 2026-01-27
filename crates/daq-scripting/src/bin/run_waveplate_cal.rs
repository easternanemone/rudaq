//! Run the 4D waveplate calibration experiment
use daq_scripting::traits::ScriptEngine;
use daq_scripting::RhaiEngine;
use tracing_subscriber::EnvFilter;

const SCRIPT: &str =
    include_str!("../../../daq-examples/examples/waveplate_calibration_4d.rhai");

/// Operations limit for 4D calibration.
///
/// The 4D waveplate calibration does:
/// - 13 wavelengths × 19 LP × 10 HWP × 10 QWP = 24,700 points
/// - Each point: 3 rotator moves, 3 power reads, array operations
/// - Plus wavelength changes, stabilization, HDF5 operations
///
/// 10,000,000 operations provides headroom for the full experiment.
const MAX_OPERATIONS: u64 = 10_000_000;

#[tokio::main]
async fn main() {
    // Initialize tracing with RUST_LOG env var
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    println!("Starting 4D waveplate calibration experiment...\n");
    println!("  Operations limit: {}", MAX_OPERATIONS);
    println!("  Expected duration: 2-3 hours for full sweep");
    println!();

    let mut engine =
        RhaiEngine::with_hardware_and_limit(MAX_OPERATIONS).expect("Failed to create RhaiEngine");

    match engine.execute_script(SCRIPT).await {
        Ok(result) => {
            println!("\n=== Calibration completed successfully ===");
            println!("Result: {:?}", result);
        }
        Err(e) => {
            eprintln!("\n=== Calibration failed ===");
            eprintln!("Error: {:?}", e);
            std::process::exit(1);
        }
    }
}
