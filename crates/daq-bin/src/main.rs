//! CLI Entry Point for rust-daq
//!
//! Provides command-line interface for:
//! - Running Rhai scripts (one-shot execution)
//! - Starting gRPC daemon for remote control (Phase 3)
//!
//! # Architecture
//!
//! This is the headless-first architecture (v5):
//! - Scripts control hardware via ScriptEngine trait (backend-agnostic)
//! - RhaiEngine as default embedded scripting backend
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

// Global allocator (Microsoft Rust Guidelines: M-MIMALLOC-APPS)
// Use mimalloc for improved allocation performance in multi-threaded DAQ scenarios
#[cfg(not(test))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rust_daq::hardware::mock::{MockCamera, MockStage};
use rust_daq::scripting::{CameraHandle, RhaiEngine, ScriptEngine, ScriptValue, StageHandle};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;

#[cfg(feature = "networking")]
use rust_daq::grpc::proto::*;
#[cfg(feature = "networking")]
use std::collections::HashMap;

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

        /// Hardware configuration file (TOML format)
        /// If not provided, uses mock devices only
        #[arg(long)]
        hardware_config: Option<PathBuf>,

        /// Use the default lab hardware configuration (maitai@100.117.5.12)
        /// Mutually exclusive with --hardware-config
        #[arg(long, conflicts_with = "hardware_config")]
        lab_hardware: bool,
    },

    /// Remote control commands (connect to daemon)
    #[cfg(feature = "networking")]
    #[command(subcommand)]
    Client(ClientCommands),
}

#[cfg(feature = "networking")]
#[derive(Subcommand)]
enum ClientCommands {
    /// Upload a script to the daemon
    Upload {
        /// Path to script file
        script: PathBuf,
        /// Optional script name
        #[arg(long)]
        name: Option<String>,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Start a previously uploaded script
    Start {
        /// Script ID (from upload response)
        script_id: String,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Stop a running script
    Stop {
        /// Execution ID (from start response)
        execution_id: String,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Get status of a script execution
    Status {
        /// Execution ID
        execution_id: String,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Stream measurement data from daemon
    Stream {
        /// Channel names to subscribe to
        #[arg(long)]
        channels: Vec<String>,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🚀 rust-daq - Headless DAQ System");
    println!("Architecture: Headless-First + Scriptable (v5)");
    println!();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run { script, config } => run_script_once(script, config).await,
        Commands::Daemon {
            port,
            hardware_config,
            lab_hardware,
        } => start_daemon(port, hardware_config, lab_hardware).await,
        #[cfg(feature = "networking")]
        Commands::Client(cmd) => handle_client_command(cmd).await,
    }
}

async fn run_script_once(script_path: PathBuf, _config: Option<PathBuf>) -> Result<()> {
    println!("📜 Loading script: {}", script_path.display());

    let script_content = tokio::fs::read_to_string(&script_path).await?;

    println!("🔧 Initializing mock hardware...");
    let stage = MockStage::new();
    let camera = MockCamera::new(1920, 1080);

    println!("⚙️  Creating script engine (Rhai backend)...");
    let mut engine = RhaiEngine::with_hardware()?;

    // Set hardware globals accessible to script
    println!("📌 Registering hardware handles...");
    engine.set_global(
        "stage",
        ScriptValue::new(StageHandle {
            driver: Arc::new(stage),
            data_tx: None, // No data plane in one-shot script mode
        }),
    )?;
    engine.set_global(
        "camera",
        ScriptValue::new(CameraHandle {
            driver: Arc::new(camera),
            data_tx: None, // No data plane in one-shot script mode
        }),
    )?;

    println!("▶️  Executing script...");
    println!();

    match engine.execute_script(&script_content).await {
        Ok(result) => {
            println!();
            println!("✅ Script completed successfully");
            println!("   Result: {:?}", result);
            Ok(())
        }
        Err(e) => {
            eprintln!();
            eprintln!("❌ Script error: {}", e);
            Err(e.into())
        }
    }
}

async fn start_daemon(
    port: u16,
    hardware_config: Option<PathBuf>,
    lab_hardware: bool,
) -> Result<()> {
    use rust_daq::health::sys_monitor::SystemMetricsCollector;
    use rust_daq::health::{HealthMonitorConfig, SystemHealthMonitor};

    println!("🌐 Starting Headless DAQ Daemon");
    println!("   Architecture: V5 (Headless-First + Scriptable)");
    println!("   gRPC Port: {}", port);
    println!();

    // Phase 5: Health Monitoring (bd-3ti1)
    println!("❤️  Initializing health monitoring...");
    let health_monitor = Arc::new(SystemHealthMonitor::new(HealthMonitorConfig::default()));

    // Start system metrics collection
    let metrics_collector = SystemMetricsCollector::new(health_monitor.clone());
    tokio::spawn(async move {
        metrics_collector.run().await;
    });

    // Phase 4: Data Plane - Ring Buffer + HDF5 Writer (optional)
    #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
    let (ring_buffer, writer_handle) = {
        use rust_daq::data::hdf5_writer::HDF5Writer;
        use rust_daq::data::ring_buffer::RingBuffer;
        use std::path::Path;
        use std::sync::Arc;

        println!("📊 Initializing data plane (Phase 4)...");
        println!("   - Ring buffer: 100 MB in /tmp/rust_daq_ring");
        println!("   - HDF5 output: experiment_data.h5");
        println!("   - Background flush: every 1 second");

        // Create ring buffer (100 MB)
        let ring_buffer = Arc::new(RingBuffer::create(Path::new("/tmp/rust_daq_ring"), 100)?);

        // Start background HDF5 writer
        let writer = HDF5Writer::new(Path::new("experiment_data.h5"), ring_buffer.clone())?;
        let writer_arc = Arc::new(writer);
        let writer_clone = writer_arc.clone();

        let handle = tokio::spawn(async move {
            writer_clone.run().await;
        });

        println!("✅ Data plane ready");
        println!();

        (Some(ring_buffer), Some((writer_arc, handle)))
    };

    #[cfg(not(all(feature = "storage_hdf5", feature = "storage_arrow")))]
    let _ring_buffer: Option<Arc<rust_daq::data::ring_buffer::RingBuffer>> = None;

    // Phase 3: Start gRPC server
    #[cfg(feature = "networking")]
    {
        use rust_daq::grpc::start_server_with_hardware;
        use rust_daq::hardware::registry::{
            create_lab_registry, create_mock_registry, create_registry_from_file,
        };
        use std::sync::Arc;
        use tokio::sync::RwLock;

        let addr = format!("0.0.0.0:{}", port).parse()?;

        // Create device registry based on configuration
        println!("🔧 Initializing hardware registry...");
        let registry = if let Some(config_path) = hardware_config {
            println!("   Loading from config: {}", config_path.display());
            create_registry_from_file(&config_path).await?
        } else if lab_hardware {
            println!("   Using lab hardware configuration (maitai@100.117.5.12)");
            create_lab_registry().await?
        } else {
            println!("   Using mock devices (no hardware config specified)");
            create_mock_registry().await?
        };

        let device_count = registry.len();
        println!("   Registered {} device(s)", device_count);
        for info in registry.list_devices() {
            println!(
                "     - {}: {} ({:?})",
                info.id, info.name, info.capabilities
            );
        }
        println!();

        let registry = Arc::new(RwLock::new(registry));

        // Start registry monitoring
        let mon_registry = registry.clone();
        let mon_health = health_monitor.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let count = mon_registry.read().await.len();
                mon_health
                    .heartbeat_with_message(
                        "hardware_registry",
                        Some(format!("Managing {} devices", count)),
                    )
                    .await;
            }
        });

        println!("✅ gRPC server ready");
        println!("   Listening on: {}", addr);
        println!("   Features:");
        println!("     - Script upload & execution");
        println!("     - Remote hardware control (HardwareService)");
        println!("     - Module system (ModuleService)");
        println!("     - Coordinated scans (ScanService)");
        println!("     - Preset save/load (PresetService)");
        println!("     - System Health Monitoring (HealthService)");
        println!();
        println!("📡 Daemon running - Press Ctrl+C to stop");
        println!();

        // Setup graceful shutdown handler
        let shutdown_signal = async {
            signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
            println!("\n🛑 Shutdown signal received, cleaning up...");
        };

        // Race server against shutdown signal
        tokio::select! {
            result = start_server_with_hardware(addr, registry, health_monitor) => {
                if let Err(e) = result {
                    eprintln!("❌ gRPC server error: {}", e);
                }
            }
            _ = shutdown_signal => {
                println!("   Initiating graceful shutdown...");
            }
        }

        // Perform cleanup
        #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
        if let Some((writer, handle)) = writer_handle {
            println!("   Flushing HDF5 writer...");
            // Final flush to ensure all data is written
            if let Err(e) = writer.flush_to_disk().await {
                eprintln!("   Warning: HDF5 flush error during shutdown: {}", e);
            }
            // Abort the background writer task
            handle.abort();
            println!("   ✓ HDF5 writer flushed and stopped");
        }

        println!("👋 Daemon shutdown complete");
        Ok(())
    }

    // Fallback if networking feature not enabled
    #[cfg(not(feature = "networking"))]
    {
        // Silence unused variable warnings
        let _ = (hardware_config, lab_hardware);

        println!("⚠️  Networking feature not enabled - daemon mode requires 'networking' feature");
        println!("   Rebuild with: cargo build --features networking");
        println!();
        println!("   Keeping daemon alive for data plane... Press Ctrl+C to stop");

        tokio::signal::ctrl_c().await?;

        println!("\n🛑 Shutdown signal received, cleaning up...");

        #[cfg(all(feature = "storage_hdf5", feature = "storage_arrow"))]
        if let Some((writer, handle)) = writer_handle {
            println!("   Flushing HDF5 writer...");
            // Final flush to ensure all data is written
            if let Err(e) = writer.flush_to_disk().await {
                eprintln!("   Warning: HDF5 flush error during shutdown: {}", e);
            }
            // Abort the background writer task
            handle.abort();
            println!("   ✓ HDF5 writer flushed and stopped");
        }

        println!("👋 Daemon shutdown complete");
        Ok(())
    }
}

#[cfg(feature = "networking")]
async fn handle_client_command(cmd: ClientCommands) -> Result<()> {
    use rust_daq::grpc::proto::control_service_client::ControlServiceClient;

    match cmd {
        ClientCommands::Upload { script, name, addr } => {
            println!("📤 Uploading script to daemon at {}", addr);
            let mut client = ControlServiceClient::connect(addr).await?;
            let content = tokio::fs::read_to_string(&script).await?;

            let response = client
                .upload_script(UploadRequest {
                    script_content: content,
                    name: name.unwrap_or_else(|| script.display().to_string()),
                    metadata: HashMap::new(),
                })
                .await?;

            let resp = response.into_inner();
            if resp.success {
                println!("✅ Script uploaded successfully");
                println!("   Script ID: {}", resp.script_id);
                println!();
                println!("   Next: Start the script with:");
                println!("   rust-daq client start {}", resp.script_id);
            } else {
                eprintln!("❌ Upload failed: {}", resp.error_message);
            }
            Ok(())
        }

        ClientCommands::Start { script_id, addr } => {
            println!("▶️  Starting script {} on daemon at {}", script_id, addr);
            let mut client = ControlServiceClient::connect(addr).await?;
            let response = client
                .start_script(StartRequest {
                    script_id,
                    parameters: HashMap::new(),
                })
                .await?;

            let resp = response.into_inner();
            if resp.started {
                println!("✅ Script started successfully");
                println!("   Execution ID: {}", resp.execution_id);
                println!();
                println!("   Monitor with:");
                println!("   rust-daq client status {}", resp.execution_id);
            } else {
                eprintln!("❌ Failed to start script");
            }
            Ok(())
        }

        ClientCommands::Stop { execution_id, addr } => {
            println!(
                "⏹️  Stopping execution {} on daemon at {}",
                execution_id, addr
            );
            let mut client = ControlServiceClient::connect(addr).await?;
            let response = client
                .stop_script(StopRequest {
                    execution_id,
                    force: false, // Try graceful stop first
                })
                .await?;

            let resp = response.into_inner();
            if resp.stopped {
                println!("✅ Script stopped successfully");
            } else {
                println!("⚠️  Script did not stop (may have already completed)");
            }
            Ok(())
        }

        ClientCommands::Status { execution_id, addr } => {
            println!(
                "📊 Checking status of execution {} on daemon at {}",
                execution_id, addr
            );
            let mut client = ControlServiceClient::connect(addr).await?;
            let response = client
                .get_script_status(StatusRequest { execution_id })
                .await?;

            let status = response.into_inner();
            println!();
            println!("Status: {}", status.state);
            if status.start_time_ns > 0 {
                println!("Started: {} ns", status.start_time_ns);
            }
            if status.end_time_ns > 0 {
                println!("Ended: {} ns", status.end_time_ns);
            }
            if !status.error_message.is_empty() {
                println!("Error: {}", status.error_message);
            }
            Ok(())
        }

        ClientCommands::Stream { channels, addr } => {
            println!("📡 Streaming data from daemon at {}", addr);
            println!("   Channels: {:?}", channels);
            println!("   Press Ctrl+C to stop");
            println!();

            let mut client = ControlServiceClient::connect(addr).await?;
            let mut stream = client
                .stream_measurements(MeasurementRequest {
                    channels,
                    max_rate_hz: 100,
                })
                .await?
                .into_inner();

            while let Some(data) = stream.message().await? {
                println!("[{}] {} = {}", data.timestamp_ns, data.channel, data.value);
            }
            Ok(())
        }
    }
}
