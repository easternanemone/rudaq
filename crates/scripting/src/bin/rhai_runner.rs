//! Generic Rhai script runner for experiment automation.
use scripting::{RhaiEngine, traits::ScriptEngine};
use std::{env, fs};
use tracing_subscriber::EnvFilter;

fn print_usage() {
    eprintln!("Usage: rhai-runner <script.rhai> [--max-ops N]");
    eprintln!("  --max-ops N    Maximum operations (default: 1000000)");
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args[1] == "--help" {
        print_usage();
        std::process::exit(if args.len() < 2 { 1 } else { 0 });
    }

    let script_path = &args[1];
    let mut max_ops: u64 = 1_000_000;

    // Parse --max-ops
    if let Some(pos) = args.iter().position(|a| a == "--max-ops") {
        if let Some(val) = args.get(pos + 1) {
            max_ops = val.parse().expect("Invalid --max-ops value");
        }
    }

    let script = fs::read_to_string(script_path).expect("Failed to read script");
    println!("Running: {} (max_ops: {})", script_path, max_ops);

    let mut engine = RhaiEngine::with_hardware_and_limit(max_ops).expect("Failed to create engine");
    match engine.execute_script(&script).await {
        Ok(r) => println!("Success: {:?}", r),
        Err(e) => {
            eprintln!("Error: {:?}", e);
            std::process::exit(1);
        }
    }
}
