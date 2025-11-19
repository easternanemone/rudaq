//! CLI Entry Point for rust-daq
//!
//! Provides command-line interface for:
//! - Running Rhai scripts (one-shot execution)
//! - Starting gRPC daemon for remote control (Phase 3)
//!
//! # Architecture
//!
//! This is the headless-first architecture (v5):
//! - Scripts control hardware via Rhai bindings
//! - Mock hardware for testing without physical devices
//! - Daemon mode for remote control (to be implemented in Phase 3)
//!
//! # Usage
//!
//! Run a script:
//! ```bash
//! rust-daq run examples/simple_scan.rhai
//! ```
//!
//! Start daemon:
//! ```bash
//! rust-daq daemon --port 50051
//! ```

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use anyhow::Result;
use rust_daq::scripting::{ScriptHost, StageHandle, CameraHandle};
use rust_daq::hardware::mock::{MockStage, MockCamera};
use std::sync::Arc;

#[cfg(feature = "networking")]
use tonic::transport::Server;

#[derive(Parser)]
#[command(name = "rust-daq")]
#[command(about = "Headless DAQ system with scriptable control", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a Rhai script once (for testing/development)
    Run {
        /// Path to .rhai script file
        script: PathBuf,

        /// Optional hardware config file
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Start daemon for remote control
    Daemon {
        /// gRPC port
        #[arg(long, default_value = "50051")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🚀 rust-daq - Headless DAQ System");
    println!("Architecture: Headless-First + Scriptable (v5)");
    println!();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run { script, config } => {
            run_script_once(script, config).await
        },
        Commands::Daemon { port } => {
            start_daemon(port).await
        },
    }
}

async fn run_script_once(script_path: PathBuf, _config: Option<PathBuf>) -> Result<()> {
    println!("📜 Loading script: {}", script_path.display());

    let script_content = tokio::fs::read_to_string(&script_path).await?;

    println!("🔧 Initializing mock hardware...");
    let stage = MockStage::new();
    let camera = MockCamera::new(1920, 1080);

    println!("⚙️  Creating script host...");
    let mut host = ScriptHost::with_hardware(tokio::runtime::Handle::current());

    // Register hardware
    let mut scope = rhai::Scope::new();
    scope.push("stage", StageHandle { driver: Arc::new(stage) });
    scope.push("camera", CameraHandle { driver: Arc::new(camera) });

    println!("▶️  Executing script...");
    println!();

    match host.engine_mut().eval_with_scope::<rhai::Dynamic>(&mut scope, &script_content) {
        Ok(result) => {
            println!();
            println!("✅ Script completed successfully");
            println!("   Result: {:?}", result);
            Ok(())
        },
        Err(e) => {
            eprintln!();
            eprintln!("❌ Script error: {}", e);
            Err(e.into())
        }
    }
}

async fn start_daemon(port: u16) -> Result<()> {
    println!("🌐 Starting Headless DAQ Daemon");
    println!("   Architecture: V5 (Headless-First + Scriptable)");
    println!("   gRPC Port: {}", port);
    println!();

    // Phase 4: Data Plane - Ring Buffer + HDF5 Writer (optional)
    #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
    {
        use std::path::Path;
        use std::sync::Arc;
        use rust_daq::data::ring_buffer::RingBuffer;
        use rust_daq::data::hdf5_writer::HDF5Writer;

        println!("📊 Initializing data plane (Phase 4)...");
        println!("   - Ring buffer: 100 MB in /tmp/rust_daq_ring");
        println!("   - HDF5 output: experiment_data.h5");
        println!("   - Background flush: every 1 second");

        // Create ring buffer (100 MB)
        let ring_buffer = Arc::new(
            RingBuffer::create(Path::new("/tmp/rust_daq_ring"), 100)?
        );

        // Start background HDF5 writer
        let writer = HDF5Writer::new(
            Path::new("experiment_data.h5"),
            ring_buffer.clone()
        )?;

        tokio::spawn(async move {
            writer.run().await;
        });

        println!("✅ Data plane ready");
        println!();
    }

    // Phase 3: Start gRPC server
    #[cfg(feature = "networking")]
    {
        use rust_daq::grpc::server::DaqServer;
        use rust_daq::grpc::proto::control_service_server::ControlServiceServer;

        let addr = format!("0.0.0.0:{}", port).parse()?;
        let server = DaqServer::new();

        println!("✅ gRPC server ready");
        println!("   Listening on: {}", addr);
        println!("   Features:");
        println!("     - Script upload & execution");
        println!("     - Remote hardware control");
        println!("     - Real-time status streaming");
        println!();
        println!("📡 Daemon running - Press Ctrl+C to stop");
        println!();

        Server::builder()
            .add_service(ControlServiceServer::new(server))
            .serve(addr)
            .await?;

        return Ok(());
    }

    // Fallback if networking feature not enabled
    #[cfg(not(feature = "networking"))]
    {
        println!("⚠️  Networking feature not enabled - daemon mode requires 'networking' feature");
        println!("   Rebuild with: cargo build --features networking");
        println!();
        println!("   Keeping daemon alive for data plane... Press Ctrl+C to stop");
        tokio::signal::ctrl_c().await?;
    }
    println!("\n👋 Daemon shutting down...");
    Ok(())
}
