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

mod daemon_manager;
mod safety_sentinel;

use anyhow::Result;
use clap::{Parser, Subcommand};
use driver_mock::{MockCamera, MockStage};
use scripting::{CameraHandle, RhaiEngine, ScriptEngine, ScriptValue, SoftLimits, StageHandle};
use std::path::PathBuf;
use std::sync::Arc;

use protocol::daq::*;
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

    /// Move a device to an absolute position
    Move {
        /// Device ID
        device_id: String,
        /// Target position
        value: f64,
        /// Wait for completion
        #[arg(long, default_value = "true")]
        wait: bool,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🚀 rust-daq - Headless DAQ System");
    println!("Architecture: Headless-First + Scriptable (v5)");
    #[cfg(feature = "networking")]
    println!("DEBUG: Feature networking ENABLED");
    #[cfg(not(feature = "networking"))]
    println!("DEBUG: Feature networking DISABLED");
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

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

async fn run_script_once(script_path: PathBuf, config: Option<PathBuf>) -> Result<()> {
    println!("📜 Loading script: {}", script_path.display());

    // Warn about ignored config in one-shot mode
    if config.is_some() {
        eprintln!("⚠️  Warning: --config flag is ignored in one-shot mode (v0.5.0)");
        eprintln!("   Config files will be supported in v0.6.0.");
        eprintln!("   One-shot mode uses hardcoded mock devices.");
        eprintln!();
    }

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
            soft_limits: SoftLimits::unlimited(),
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
            Err(anyhow::Error::from(e))
        }
    }
}

async fn start_daemon(
    port: u16,
    hardware_config: Option<PathBuf>,
    lab_hardware: bool,
) -> Result<()> {
    use daemon_manager::{DaemonConfig, DaemonInstance};

    let config = DaemonConfig {
        port,
        hardware_config,
        lab_hardware,
    };

    let instance = DaemonInstance::start(config).await?;
    instance.wait_for_shutdown_signal().await;
    instance.shutdown().await
}

#[cfg(feature = "networking")]
async fn handle_client_command(cmd: ClientCommands) -> Result<()> {
    use protocol::daq::control_service_client::ControlServiceClient;

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

        ClientCommands::Move {
            device_id,
            value,
            wait,
            addr,
        } => {
            use protocol::daq::hardware_service_client::HardwareServiceClient;

            println!(
                "🔄 Moving device {} to {} on daemon at {}",
                device_id, value, addr
            );
            let mut client = HardwareServiceClient::connect(addr).await?;
            let _response = client
                .move_absolute(MoveRequest {
                    device_id,
                    value,
                    wait_for_completion: Some(wait),
                    timeout_ms: Some(30000),
                })
                .await?;

            println!("✅ Move command accepted");
            Ok(())
        }
    }
}
