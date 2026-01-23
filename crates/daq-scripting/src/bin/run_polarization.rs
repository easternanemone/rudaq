//! Run the polarization characterization experiment
use daq_scripting::traits::ScriptEngine;
use daq_scripting::RhaiEngine;
use tracing_subscriber::EnvFilter;

const SCRIPT: &str =
    include_str!("../../../daq-examples/examples/polarization_characterization.rhai");

/// Operations limit for long-running experiments.
///
/// The polarization characterization does:
/// - 73 steps × 3 rotators = 219 scan iterations
/// - Each iteration: move_abs, wait_settled (~3 checks), 3 power reads, array operations
/// - Plus setup, HDF5 operations, analysis
///
/// 1,000,000 operations provides headroom for the full experiment.
const MAX_OPERATIONS: u64 = 1_000_000;

#[tokio::main]
async fn main() {
    // Initialize tracing with RUST_LOG env var
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    println!("Starting polarization characterization experiment...\n");
    println!("  Operations limit: {}", MAX_OPERATIONS);

    let mut engine =
        RhaiEngine::with_hardware_and_limit(MAX_OPERATIONS).expect("Failed to create RhaiEngine");

    match engine.execute_script(SCRIPT).await {
        Ok(result) => {
            println!("\n=== Experiment completed successfully ===");
            println!("Result: {:?}", result);
        }
        Err(e) => {
            eprintln!("\n=== Experiment failed ===");
            eprintln!("Error: {:?}", e);
            std::process::exit(1);
        }
    }
}
