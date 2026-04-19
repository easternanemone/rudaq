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

mod calibrate;
mod daemon_manager;
#[cfg(feature = "db")]
mod db_bridge;
#[cfg(feature = "db")]
mod reconciler;
#[cfg(all(feature = "db", feature = "metrics"))]
mod reconciler_metrics;
mod safety_heartbeat_task;
mod safety_sentinel;
#[cfg(feature = "networking")]
mod snapshot;
#[cfg(feature = "db")]
mod watch_reconciler;

use anyhow::{Context, Result};
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

/// Explicit runtime selection for daemon launch behavior.
///
/// When no mode is specified via CLI or env var, defaults to `HybridDb`.
#[derive(clap::ValueEnum, Debug, Clone)]
enum RuntimeMode {
    /// Mock-only local runtime (no hardware config). Good for demos/testing.
    Mock,
    /// Native maitai profile (`config/maitai_universal.toml`).
    Native,
    /// Universal TOML profile (`config/maitai_universal.toml`).
    Universal,
    /// Universal profile with SQLite control-plane (default).
    HybridDb,
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

    /// Start daemon for remote control.
    ///
    /// Default runtime mode is hybrid-db (universal TOML + SQLite control-plane).
    /// Override with --runtime-mode, DAQ_RUNTIME_MODE / RUSTDAQ_RUNTIME_MODE env var,
    /// or --hardware-config / --lab-hardware flags.
    ///
    /// Environment variable overrides (CLI flags take precedence):
    ///   DAQ_PORT         — gRPC listen port (default: 50051)
    ///   DAQ_CONFIG_PATH  — hardware configuration file (maps to --hardware-config)
    ///   DAQ_RUNTIME_MODE — runtime mode (also accepts RUSTDAQ_RUNTIME_MODE)
    ///   DAQ_DATA_PATH    — storage output directory (maps to --data-path)
    Daemon {
        /// gRPC port (also settable via DAQ_PORT env var)
        #[arg(long)]
        port: Option<u16>,

        /// Explicit runtime mode (default: hybrid-db).
        ///
        /// Overrides profile selection logic and maps to built-in configs:
        /// - mock: no hardware config (for demos/testing)
        /// - native: config/maitai_universal.toml
        /// - universal: config/maitai_universal.toml
        /// - hybrid-db: config/maitai_universal.toml (+ SQLite control-plane)
        ///
        /// Also settable via DAQ_RUNTIME_MODE or RUSTDAQ_RUNTIME_MODE env var.
        #[arg(
            long,
            value_enum,
            value_name = "MODE",
            conflicts_with_all = ["hardware_config", "lab_hardware"]
        )]
        runtime_mode: Option<RuntimeMode>,

        /// Hardware configuration file (TOML format).
        /// Also settable via DAQ_CONFIG_PATH env var.
        /// If not provided, uses mock devices only.
        #[arg(long)]
        hardware_config: Option<PathBuf>,

        /// Use the default lab hardware configuration (maitai@100.117.5.12)
        /// Mutually exclusive with --hardware-config
        #[arg(long, conflicts_with = "hardware_config")]
        lab_hardware: bool,

        /// Database path for SQLite file-backed engine (daemon mode).
        /// Omit to use in-memory engine.
        #[cfg(feature = "db")]
        #[arg(long)]
        db_path: Option<PathBuf>,

        /// Optional path to the WASM web UI directory (index.html, .wasm).
        /// If provided, the daemon will serve the UI on the gRPC port.
        #[arg(long)]
        web_ui_path: Option<PathBuf>,

        /// Storage output directory for persistent recordings (HDF5, Arrow, etc.).
        /// Also settable via DAQ_DATA_PATH env var.
        /// Overrides the output_directory from config/config.v4.toml.
        #[arg(long)]
        data_path: Option<PathBuf>,
    },

    /// Remote control commands (connect to daemon)
    #[cfg(feature = "networking")]
    #[command(subcommand)]
    Client(ClientCommands),

    /// Database configuration management
    #[cfg(feature = "db")]
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Capture a single frame from a camera on a running daemon
    #[cfg(feature = "networking")]
    Snapshot {
        /// Camera device ID (e.g., "istar_camera", "mock_camera")
        device_id: String,

        /// Output file path
        #[arg(long, short)]
        output: PathBuf,

        /// Exposure time in milliseconds (uses camera's current setting if omitted)
        #[arg(long)]
        exposure_ms: Option<f64>,

        /// Device parameter to set before capture, format `name=value`.
        /// Repeatable. Applied via the generic SetParameter RPC in order.
        /// Useful for MCP gain, gate mode, trigger mode, etc.
        /// Example: --set mcp_gain=50 --set "gate_mode=CW On"
        #[arg(long = "set", value_parser = parse_snapshot_set)]
        set_params: Vec<(String, String)>,

        /// Device parameter to set AFTER capture, format `name=value`.
        /// Repeatable. Applied immediately after the frame is written,
        /// before the process exits. Used for atomic MCP-gain-reset
        /// semantics on ICCD captures so no frame lingers above the
        /// safety threshold between matrix entries (bd-g22gu.2.2.1).
        /// Example: --after-set mcp_gain=0
        #[arg(long = "after-set", value_parser = parse_snapshot_set)]
        after_set_params: Vec<(String, String)>,

        /// Output format
        #[arg(long, value_enum, default_value = "tiff")]
        format: snapshot::SnapshotFormat,

        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Run echelle wavelength calibration on an arc lamp frame
    Calibrate {
        /// Path to arc lamp frame (TIFF or raw binary)
        #[arg(long)]
        frame: PathBuf,

        /// Optional flat-field frame for trace detection (tungsten halogen).
        /// When provided, order traces are found from this broadband frame
        /// (all orders visible), while arc lines are extracted from --frame.
        #[arg(long)]
        flat: Option<PathBuf>,

        /// Path to calibration config (TOML)
        #[arg(long)]
        config: PathBuf,

        /// Output path for calibration profile (TOML)
        #[arg(long)]
        output: PathBuf,

        /// Emit a verbose per-order diagnostic report (wavelength samples,
        /// monotonicity check, grating-constant consistency). Useful for
        /// debugging scientific correctness of a calibration run.
        #[arg(long, default_value_t = false)]
        diagnose: bool,
    },

    /// Generate a synthetic echelle frame (continuum, arc, or combined)
    Simulate {
        /// Type of frame: "continuum", "arc", or "combined"
        #[arg(long, default_value = "arc")]
        source: String,

        /// Output TIFF path
        #[arg(long, short)]
        output: PathBuf,

        /// Grating constant in nm (default: 36300 for Mechelle 5000)
        #[arg(long, default_value = "36300")]
        grating_constant: f64,

        /// Continuum temperature in K (default: 3200 for tungsten halogen)
        #[arg(long, default_value = "3200")]
        temperature: f64,
    },

    /// Recover data from a corrupt HDF5 file after power loss
    Recover {
        /// Path to the corrupt HDF5 file
        #[arg(long)]
        input: PathBuf,

        /// Path for the recovered output file (must not already exist)
        #[arg(long)]
        output: PathBuf,
    },
}

/// Subcommands for `rust-daq config ...`
#[cfg(feature = "db")]
#[derive(Subcommand)]
enum ConfigCommands {
    /// Import a TOML hardware config into the database
    Import {
        /// Path to hardware config file (TOML)
        file: PathBuf,

        /// Database path (RocksDB). Omit for in-memory.
        #[arg(long)]
        db_path: Option<PathBuf>,
    },

    /// Export database contents as TOML (to stdout)
    Export {
        /// Database path (RocksDB). Omit for in-memory.
        #[arg(long)]
        db_path: Option<PathBuf>,
    },

    /// List instruments stored in the database
    List {
        /// Database path (RocksDB). Omit for in-memory.
        #[arg(long)]
        db_path: Option<PathBuf>,
    },
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

    /// List instruments in the running daemon's database
    #[cfg(feature = "db")]
    ConfigList {
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Get a specific instrument from the running daemon
    #[cfg(feature = "db")]
    ConfigGet {
        /// Device ID to look up
        device_id: String,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Import a TOML hardware config into the running daemon's database
    #[cfg(feature = "db")]
    ConfigImport {
        /// Path to hardware config file (TOML)
        file: PathBuf,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Export the running daemon's database as TOML
    #[cfg(feature = "db")]
    ConfigExport {
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Delete an instrument from the running daemon's database
    #[cfg(feature = "db")]
    ConfigDelete {
        /// Device ID to delete
        device_id: String,
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Show database info from the running daemon
    #[cfg(feature = "db")]
    ConfigInfo {
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },

    /// Stream live config changes from the running daemon
    #[cfg(feature = "db")]
    ConfigWatch {
        /// Daemon address
        #[arg(long, default_value = "http://localhost:50051")]
        addr: String,
    },
}

/// Install a panic hook that emits structured forensics before chaining to the
/// default panic handler. Critical for safety-interlock post-mortems: if the
/// daemon panics, the heartbeat thread stops toggling the Comedi DIO line and
/// the external interlock should fire — operators need a timestamped record of
/// the panic that preceded that event.
/// Clap value parser for `--set name=value` snapshot-subcommand args.
/// Splits on the first `=`; value may contain further `=` characters.
#[cfg(feature = "networking")]
fn parse_snapshot_set(s: &str) -> Result<(String, String), String> {
    let (name, value) = s
        .split_once('=')
        .ok_or_else(|| format!("expected `name=value`, got `{s}`"))?;
    if name.is_empty() {
        return Err(format!("empty parameter name in `{s}`"));
    }
    Ok((name.to_string(), value.to_string()))
}

fn install_safety_panic_hook() {
    let prior = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        let thread = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();

        tracing::error!(
            thread = %thread,
            location = %location,
            payload = %payload,
            "FATAL: daemon panic detected — safety interlock should activate within 100ms"
        );
        eprintln!(
            "FATAL: daemon panic on thread {thread} at {location}: {payload} \
             — safety interlock should activate within 100ms"
        );

        prior(info);
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    install_safety_panic_hook();
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

    // Probe for hardware SDK availability at runtime (dlopen check).
    // This runs early so the discovered SDKs appear in startup logs.
    let discovered_sdks = driver_registry::probe_available();
    tracing::info!(%discovered_sdks, "Runtime SDK probe complete");
    println!("SDK probe: {discovered_sdks}");

    println!();

    let cli = Cli::parse();

    match cli.command {
        Commands::Run { script, config } => run_script_once(script, config).await,
        Commands::Daemon {
            port: cli_port,
            runtime_mode,
            hardware_config,
            lab_hardware,
            #[cfg(feature = "db")]
            db_path,
            web_ui_path,
            data_path,
        } => {
            // Resolve port: CLI flag > DAQ_PORT env var > default (50051)
            let (port, port_source) = resolve_port(cli_port);

            // Resolve hardware_config: CLI flag > DAQ_CONFIG_PATH env var > None
            let (hardware_config, config_source) = resolve_hardware_config(hardware_config);

            // Resolve runtime mode: CLI flag > DAQ_RUNTIME_MODE env > RUSTDAQ_RUNTIME_MODE env > None
            let (runtime_mode, mode_source) = resolve_runtime_mode(runtime_mode);

            // Resolve data_path: CLI flag > DAQ_DATA_PATH env var > None (use config default)
            let (data_path, data_path_source) = resolve_data_path(data_path);

            // Log resolved values (use println for always-visible startup info)
            println!("Port: {port} ({port_source})");
            if let Some(ref cfg) = hardware_config {
                println!("Hardware config: {} ({})", cfg.display(), config_source);
            } else {
                println!("Hardware config: none ({config_source})");
            }
            if let Some(ref ui) = web_ui_path {
                println!("Web UI path: {} (CLI/Env)", ui.display());
            }
            if let Some(ref dp) = data_path {
                println!("Data path: {} ({data_path_source})", dp.display());
            }

            println!(
                "Runtime mode source: {} ({})",
                runtime_mode
                    .as_ref()
                    .map_or("not set".to_string(), |m| format!("{m:?}")),
                mode_source
            );

            start_daemon(
                port,
                runtime_mode,
                hardware_config,
                lab_hardware,
                #[cfg(feature = "db")]
                db_path,
                #[cfg(not(feature = "db"))]
                None,
                web_ui_path,
                data_path,
            )
            .await
        }
        #[cfg(feature = "networking")]
        Commands::Client(cmd) => handle_client_command(cmd).await,
        #[cfg(feature = "networking")]
        Commands::Snapshot {
            device_id,
            output,
            exposure_ms,
            set_params,
            after_set_params,
            format,
            addr,
        } => {
            snapshot::handle_snapshot(
                device_id,
                output,
                exposure_ms,
                set_params,
                after_set_params,
                format,
                addr,
            )
            .await
        }
        Commands::Calibrate {
            frame,
            flat,
            config,
            output,
            diagnose,
        } => calibrate::handle_calibrate(frame, flat, config, output, diagnose).await,
        Commands::Simulate {
            source,
            output,
            grating_constant,
            temperature,
        } => handle_simulate(source, output, grating_constant, temperature),
        #[cfg(feature = "db")]
        Commands::Config(cmd) => handle_config_command(cmd).await,
        Commands::Recover { input, output } => handle_recover(input, output).await,
    }
}

fn handle_simulate(
    source: String,
    output: PathBuf,
    grating_constant: f64,
    temperature: f64,
) -> Result<()> {
    use echelle::simulation::EchelleSimConfig;
    use echelle::wavelength_fitting::load_hgar_atlas;

    let config = EchelleSimConfig {
        grating_constant_nm: grating_constant,
        continuum_temperature_k: temperature,
        ..Default::default()
    };

    println!(
        "Generating {source} echelleogram ({} x {})...",
        config.width, config.height
    );
    println!("  Grating constant: {grating_constant} nm");
    println!(
        "  Orders: m={} to m={}",
        config.first_order, config.last_order
    );

    let frame = match source.as_str() {
        "continuum" => {
            println!("  Source: tungsten halogen ({temperature} K blackbody)");
            config.simulate_continuum()
        }
        "arc" => {
            let atlas = load_hgar_atlas();
            println!(
                "  Source: HgAr arc lamp ({} lines, with continuum floor)",
                atlas.len()
            );
            config.simulate_arc(&atlas)
        }
        "arc-realistic" => {
            let atlas = load_hgar_atlas();
            println!(
                "  Source: HgAr arc lamp ({} lines, no continuum — realistic)",
                atlas.len()
            );
            config.simulate_arc_realistic(&atlas, false)
        }
        "combined" => {
            let atlas = load_hgar_atlas();
            println!(
                "  Source: combined continuum + HgAr ({} lines)",
                atlas.len()
            );
            config.simulate_combined(&atlas)
        }
        other => anyhow::bail!(
            "Unknown source type '{other}'. Use: continuum, arc, arc-realistic, combined"
        ),
    };

    // Scale to u16 and write TIFF.
    let max_val = frame.iter().copied().fold(0.0_f32, f32::max);
    let scale = if max_val > 0.0 {
        60000.0 / f64::from(max_val)
    } else {
        1.0
    };
    let u16_bytes: Vec<u8> = frame
        .iter()
        .flat_map(|&v| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let val = (f64::from(v) * scale).clamp(0.0, 65535.0) as u16;
            val.to_ne_bytes()
        })
        .collect();

    let file = std::fs::File::create(&output)
        .with_context(|| format!("Cannot create {}", output.display()))?;
    let writer = std::io::BufWriter::new(file);
    let encoder = image::codecs::tiff::TiffEncoder::new(writer);
    #[allow(deprecated)]
    encoder
        .encode(
            &u16_bytes,
            config.width,
            config.height,
            image::ExtendedColorType::L16,
        )
        .context("TIFF encoding failed")?;

    let size_kb = std::fs::metadata(&output)
        .map(|m| m.len() / 1024)
        .unwrap_or(0);
    println!(
        "Written to {} ({size_kb} KB, scale={scale:.1})",
        output.display()
    );
    Ok(())
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
            println!("   Result: {result:?}");
            Ok(())
        }
        Err(e) => {
            eprintln!();
            eprintln!("❌ Script error: {e}");
            Err(anyhow::Error::from(e))
        }
    }
}

// ---------------------------------------------------------------------------
// Environment variable resolution helpers
// ---------------------------------------------------------------------------

/// Default gRPC port when neither CLI flag nor env var is set.
const DEFAULT_PORT: u16 = 50051;

/// Resolve the gRPC port from CLI flag or `DAQ_PORT` env var.
///
/// Precedence: CLI flag > `DAQ_PORT` env var > built-in default (50051).
fn resolve_port(cli_port: Option<u16>) -> (u16, &'static str) {
    if let Some(port) = cli_port {
        return (port, "from --port flag");
    }
    if let Ok(val) = std::env::var("DAQ_PORT") {
        match val.parse::<u16>() {
            Ok(port) => return (port, "from DAQ_PORT env var"),
            Err(_) => {
                eprintln!("Warning: DAQ_PORT={val:?} is not a valid port number, using default");
            }
        }
    }
    (DEFAULT_PORT, "default")
}

/// Resolve the hardware config path from CLI flag or `DAQ_CONFIG_PATH` env var.
///
/// Precedence: CLI flag > `DAQ_CONFIG_PATH` env var > None.
fn resolve_hardware_config(cli_config: Option<PathBuf>) -> (Option<PathBuf>, &'static str) {
    if cli_config.is_some() {
        return (cli_config, "from --hardware-config flag");
    }
    if let Ok(val) = std::env::var("DAQ_CONFIG_PATH")
        && !val.is_empty()
    {
        return (Some(PathBuf::from(val)), "from DAQ_CONFIG_PATH env var");
    }
    (None, "default")
}

/// Resolve the runtime mode from CLI flag or env vars.
///
/// Precedence: CLI flag > `DAQ_RUNTIME_MODE` env var > `RUSTDAQ_RUNTIME_MODE` env var > None.
fn resolve_runtime_mode(cli_mode: Option<RuntimeMode>) -> (Option<RuntimeMode>, &'static str) {
    if cli_mode.is_some() {
        return (cli_mode, "from --runtime-mode flag");
    }

    // LEGACY: Falls back to RUSTDAQ_RUNTIME_MODE for old service files and CI scripts.
    // Remove after all deployments use DAQ_RUNTIME_MODE. Low priority.
    let (env_val, source) = if let Ok(val) = std::env::var("DAQ_RUNTIME_MODE") {
        (Some(val), "from DAQ_RUNTIME_MODE env var")
    } else if let Ok(val) = std::env::var("RUSTDAQ_RUNTIME_MODE") {
        (Some(val), "from RUSTDAQ_RUNTIME_MODE env var")
    } else {
        (None, "default")
    };

    if let Some(val) = env_val {
        match val.as_str() {
            "mock" => return (Some(RuntimeMode::Mock), source),
            "native" => return (Some(RuntimeMode::Native), source),
            "universal" => return (Some(RuntimeMode::Universal), source),
            "hybrid-db" => return (Some(RuntimeMode::HybridDb), source),
            other => {
                eprintln!(
                    "Warning: Invalid runtime mode {other:?} in env var, ignoring (valid: mock, native, universal, hybrid-db)"
                );
            }
        }
    }

    (None, "default")
}

/// Resolve the storage data path from CLI flag or `DAQ_DATA_PATH` env var.
///
/// Precedence: CLI flag > `DAQ_DATA_PATH` env var > None (use config file default).
fn resolve_data_path(cli_path: Option<PathBuf>) -> (Option<PathBuf>, &'static str) {
    if cli_path.is_some() {
        return (cli_path, "from --data-path flag");
    }
    if let Ok(val) = std::env::var("DAQ_DATA_PATH")
        && !val.is_empty()
    {
        return (Some(PathBuf::from(val)), "from DAQ_DATA_PATH env var");
    }
    (None, "default")
}

async fn start_daemon(
    port: u16,
    runtime_mode: Option<RuntimeMode>,
    hardware_config: Option<PathBuf>,
    lab_hardware: bool,
    db_path: Option<PathBuf>,
    web_ui_path: Option<PathBuf>,
    data_path: Option<PathBuf>,
) -> Result<()> {
    use daemon_manager::{DaemonConfig, DaemonInstance};

    // runtime_mode already has CLI flag > env var precedence applied
    // by resolve_runtime_mode() in main(). Map it to config paths here.
    let effective_mode = runtime_mode;

    let resolved_runtime_mode;
    let mut resolved_hardware_config = hardware_config;
    let mut resolved_lab_hardware = lab_hardware;

    if let Some(mode) = effective_mode {
        match mode {
            RuntimeMode::Mock => {
                resolved_runtime_mode = "mock";
                resolved_hardware_config = None;
                resolved_lab_hardware = false;
            }
            RuntimeMode::Native => {
                resolved_runtime_mode = "native";
                resolved_hardware_config = Some(PathBuf::from("config/maitai_universal.toml"));
                resolved_lab_hardware = false;
            }
            RuntimeMode::Universal => {
                resolved_runtime_mode = "universal";
                resolved_hardware_config = Some(PathBuf::from("config/maitai_universal.toml"));
                resolved_lab_hardware = false;
            }
            RuntimeMode::HybridDb => {
                resolved_runtime_mode = "hybrid-db";
                resolved_hardware_config = Some(PathBuf::from("config/maitai_universal.toml"));
                resolved_lab_hardware = false;
            }
        }
    } else if lab_hardware {
        resolved_runtime_mode = "native";
    } else if resolved_hardware_config.is_some() {
        resolved_runtime_mode = "custom";
        resolved_lab_hardware = false;
    } else {
        // Default: hybrid-db — the universal TOML driver system with SQLite
        resolved_runtime_mode = "hybrid-db";
        resolved_hardware_config = Some(PathBuf::from("config/maitai_universal.toml"));
    }

    println!("🧭 Runtime mode: {resolved_runtime_mode}");
    #[cfg(feature = "db")]
    if let Some(path) = db_path.as_ref() {
        println!("🗃️  Database path: {}", path.display());
    }
    #[cfg(not(feature = "db"))]
    if resolved_runtime_mode == "hybrid-db" {
        eprintln!(
            "⚠️  hybrid-db mode active but this build has no db feature; running universal TOML without DB persistence."
        );
    }
    #[cfg(not(feature = "db"))]
    let _ = db_path;

    let daemon_config = DaemonConfig {
        port,
        hardware_config: resolved_hardware_config,
        lab_hardware: resolved_lab_hardware,
        runtime_mode: resolved_runtime_mode.to_string(),
        #[cfg(feature = "db")]
        db_path,
        web_ui_path,
        data_path,
    };

    let instance = DaemonInstance::start(daemon_config).await?;
    instance.wait_for_shutdown_signal().await;
    instance.shutdown().await
}

#[cfg(feature = "networking")]
async fn handle_client_command(cmd: ClientCommands) -> Result<()> {
    use protocol::daq::control_service_client::ControlServiceClient;

    match cmd {
        ClientCommands::Upload { script, name, addr } => {
            println!("📤 Uploading script to daemon at {addr}");
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
            println!("▶️  Starting script {script_id} on daemon at {addr}");
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
            println!("⏹️  Stopping execution {execution_id} on daemon at {addr}");
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
            println!("📊 Checking status of execution {execution_id} on daemon at {addr}");
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
            println!("📡 Streaming data from daemon at {addr}");
            println!("   Channels: {channels:?}");
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

            println!("🔄 Moving device {device_id} to {value} on daemon at {addr}");
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

        #[cfg(feature = "db")]
        ClientCommands::ConfigList { addr } => client_config_list(addr).await,

        #[cfg(feature = "db")]
        ClientCommands::ConfigGet { device_id, addr } => client_config_get(device_id, addr).await,

        #[cfg(feature = "db")]
        ClientCommands::ConfigImport { file, addr } => client_config_import(file, addr).await,

        #[cfg(feature = "db")]
        ClientCommands::ConfigExport { addr } => client_config_export(addr).await,

        #[cfg(feature = "db")]
        ClientCommands::ConfigDelete { device_id, addr } => {
            client_config_delete(device_id, addr).await
        }

        #[cfg(feature = "db")]
        ClientCommands::ConfigInfo { addr } => client_config_info(addr).await,

        #[cfg(feature = "db")]
        ClientCommands::ConfigWatch { addr } => client_config_watch(addr).await,
    }
}

// ---------------------------------------------------------------------------
// Config subcommands (db feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "db")]
async fn handle_config_command(cmd: ConfigCommands) -> Result<()> {
    match cmd {
        ConfigCommands::Import { file, db_path } => config_import(file, db_path).await,
        ConfigCommands::Export { db_path } => config_export(db_path).await,
        ConfigCommands::List { db_path } => config_list(db_path).await,
    }
}

/// Helper: open a DB connection from an optional path.
#[cfg(feature = "db")]
async fn open_db(db_path: Option<PathBuf>) -> Result<db::DaqDb> {
    let db_config = match db_path {
        Some(ref path) => db::DbConfig::file(path.display().to_string()),
        None => db::DbConfig::in_memory(),
    };
    db::DaqDb::init(db_config)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to open database: {e}"))
}

/// `rust-daq config import <file>` — load TOML hardware config into DB.
#[cfg(feature = "db")]
async fn config_import(file: PathBuf, db_path: Option<PathBuf>) -> Result<()> {
    use driver_registry::register_all_factories;
    use hardware::registry::{DeviceRegistry, HardwareConfig};

    let hw_config = HardwareConfig::from_file(&file)
        .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", file.display()))?;

    let db = open_db(db_path).await?;
    let registry = DeviceRegistry::new();

    let config_dir = file
        .parent()
        .map(|parent| parent.join("devices"))
        .filter(|path| path.exists())
        .or_else(|| {
            let default = PathBuf::from("config/devices");
            default.exists().then_some(default)
        });

    if let Err(e) = register_all_factories(&registry, config_dir.as_deref()).await {
        tracing::warn!(
            error = %e,
            config_dir = ?config_dir,
            "Factory registration failed during config import; preserving existing driver metadata"
        );
    }

    let (drivers, instruments) = db_bridge::shadow_write_with_registry(&db, &hw_config, &registry)
        .await
        .map_err(|e| anyhow::anyhow!("Import failed: {e}"))?;

    println!("Imported {drivers} driver(s), {instruments} instrument(s)");
    Ok(())
}

/// `rust-daq config export` — dump DB instruments as TOML to stdout.
#[cfg(feature = "db")]
async fn config_export(db_path: Option<PathBuf>) -> Result<()> {
    let db = open_db(db_path).await?;
    let instruments = db
        .get_all_instruments()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if instruments.is_empty() {
        eprintln!("No instruments in database.");
        return Ok(());
    }

    let toml_str = db_bridge::db_to_hardware_toml(&instruments)
        .map_err(|e| anyhow::anyhow!("TOML serialization failed: {e}"))?;
    print!("{toml_str}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Online config commands (talk to running daemon via gRPC)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "networking", feature = "db"))]
async fn config_client(
    addr: &str,
) -> Result<protocol::daq::config_service_client::ConfigServiceClient<tonic::transport::Channel>> {
    protocol::daq::config_service_client::ConfigServiceClient::connect(addr.to_string())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon at {addr}: {e}"))
}

#[cfg(all(feature = "networking", feature = "db"))]
async fn client_config_list(addr: String) -> Result<()> {
    let mut client = config_client(&addr).await?;
    let resp = client
        .list_instruments(protocol::daq::ListInstrumentsRequest {})
        .await?
        .into_inner();

    if resp.instruments.is_empty() {
        println!("No instruments in database.");
        return Ok(());
    }

    println!(
        "{:<20} {:<30} {:<15} ENABLED",
        "DEVICE_ID", "NAME", "DRIVER"
    );
    println!("{}", "-".repeat(75));
    for inst in &resp.instruments {
        println!(
            "{:<20} {:<30} {:<15} {}",
            inst.device_id,
            inst.name,
            inst.driver_type,
            if inst.enabled { "yes" } else { "no" },
        );
    }
    println!("\n{} instrument(s) total", resp.instruments.len());
    Ok(())
}

#[cfg(all(feature = "networking", feature = "db"))]
async fn client_config_get(device_id: String, addr: String) -> Result<()> {
    let mut client = config_client(&addr).await?;
    let resp = client
        .get_instrument(protocol::daq::GetInstrumentRequest { device_id })
        .await?
        .into_inner();

    println!("Device ID:    {}", resp.device_id);
    println!("Name:         {}", resp.name);
    println!("Driver Type:  {}", resp.driver_type);
    println!("Enabled:      {}", resp.enabled);
    println!("Config (JSON): {}", resp.config_json);
    Ok(())
}

#[cfg(all(feature = "networking", feature = "db"))]
async fn client_config_import(file: PathBuf, addr: String) -> Result<()> {
    let toml_content = tokio::fs::read_to_string(&file).await?;
    let mut client = config_client(&addr).await?;
    let resp = client
        .import_config(protocol::daq::ImportConfigRequest { toml_content })
        .await?
        .into_inner();

    if resp.success {
        println!(
            "Imported {} driver(s), {} instrument(s)",
            resp.drivers_imported, resp.instruments_imported
        );
    } else {
        eprintln!("Import failed: {}", resp.message);
    }
    Ok(())
}

#[cfg(all(feature = "networking", feature = "db"))]
async fn client_config_export(addr: String) -> Result<()> {
    let mut client = config_client(&addr).await?;
    let resp = client
        .export_config(protocol::daq::ExportConfigRequest {})
        .await?
        .into_inner();

    if resp.toml_content.is_empty() {
        println!("No instruments in database.");
    } else {
        print!("{}", resp.toml_content);
    }
    Ok(())
}

#[cfg(all(feature = "networking", feature = "db"))]
async fn client_config_delete(device_id: String, addr: String) -> Result<()> {
    let mut client = config_client(&addr).await?;
    let resp = client
        .delete_instrument(protocol::daq::DeleteInstrumentRequest {
            device_id: device_id.clone(),
        })
        .await?
        .into_inner();

    if resp.success {
        println!("Deleted '{device_id}'");
    } else {
        eprintln!("Delete failed: {}", resp.message);
    }
    Ok(())
}

#[cfg(all(feature = "networking", feature = "db"))]
async fn client_config_watch(addr: String) -> Result<()> {
    let mut client = config_client(&addr).await?;
    println!("Watching config changes on {addr}... (Ctrl+C to stop)");
    println!();

    let mut stream = client
        .subscribe_config_changes(protocol::daq::SubscribeConfigRequest {})
        .await?
        .into_inner();

    while let Some(event) = stream.message().await? {
        let ts = event.timestamp_ns / 1_000_000_000;
        match event.change_type.as_str() {
            "delete" => {
                println!("[{ts}s] DELETE {}", event.device_id);
            }
            _ => {
                if let Some(inst) = &event.instrument {
                    println!(
                        "[{ts}s] {} {} (driver={}, enabled={})",
                        event.change_type.to_uppercase(),
                        inst.device_id,
                        inst.driver_type,
                        inst.enabled,
                    );
                } else {
                    println!(
                        "[{ts}s] {} {}",
                        event.change_type.to_uppercase(),
                        event.device_id,
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(all(feature = "networking", feature = "db"))]
async fn client_config_info(addr: String) -> Result<()> {
    let mut client = config_client(&addr).await?;
    let resp = client
        .get_db_info(protocol::daq::GetDbInfoRequest {})
        .await?
        .into_inner();

    println!("Engine:           {}", resp.engine);
    println!("Namespace:        {}", resp.namespace);
    println!("Database:         {}", resp.database);
    println!("Schema Version:   {}", resp.schema_version);
    println!("Uptime:           {}s", resp.uptime_secs);
    println!("Healthy:          {}", resp.healthy);
    println!("Drivers:          {}", resp.driver_count);
    println!("Instruments:      {}", resp.instrument_count);
    Ok(())
}

/// `rust-daq config list` — list instruments in the database.
#[cfg(feature = "db")]
async fn config_list(db_path: Option<PathBuf>) -> Result<()> {
    let db = open_db(db_path).await?;
    let instruments = db
        .list_instruments()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if instruments.is_empty() {
        println!("No instruments in database.");
        return Ok(());
    }

    println!(
        "{:<20} {:<30} {:<15} ENABLED",
        "DEVICE_ID", "NAME", "DRIVER"
    );
    println!("{}", "-".repeat(75));
    for inst in &instruments {
        println!(
            "{:<20} {:<30} {:<15} {}",
            inst.device_id,
            inst.name,
            inst.driver_type,
            if inst.enabled { "yes" } else { "no" },
        );
    }
    println!("\n{} instrument(s) total", instruments.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// Recover subcommand
// ---------------------------------------------------------------------------

async fn handle_recover(input: PathBuf, output: PathBuf) -> Result<()> {
    println!("HDF5 Recovery Tool");
    println!("  Input:  {}", input.display());
    println!("  Output: {}", output.display());
    println!();

    // Run the blocking HDF5 recovery in a spawn_blocking task
    let report =
        tokio::task::spawn_blocking(move || storage::recover_hdf5(&input, &output)).await??;

    println!("{report}");

    if report.lost_datasets > 0 {
        eprintln!(
            "Warning: {} dataset(s) could not be recovered.",
            report.lost_datasets
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests for environment variable resolution
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(unsafe_code)] // edition 2024: env::set_var/remove_var require unsafe
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize env-var tests so they don't race on shared process environment.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Helper: clear all DAQ-related env vars before each test.
    ///
    /// # Safety
    /// Caller must hold `ENV_LOCK` to prevent concurrent env mutation.
    unsafe fn clear_env_vars() {
        unsafe {
            std::env::remove_var("DAQ_PORT");
            std::env::remove_var("DAQ_CONFIG_PATH");
            std::env::remove_var("DAQ_RUNTIME_MODE");
            std::env::remove_var("RUSTDAQ_RUNTIME_MODE");
            std::env::remove_var("DAQ_DATA_PATH");
        }
    }

    // ---- resolve_port tests ----

    #[test]
    fn port_cli_flag_wins_over_env() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_PORT", "9999");
        }
        let (port, source) = resolve_port(Some(8080));
        assert_eq!(port, 8080);
        assert_eq!(source, "from --port flag");
    }

    #[test]
    fn port_env_var_used_when_no_cli_flag() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_PORT", "9999");
        }
        let (port, source) = resolve_port(None);
        assert_eq!(port, 9999);
        assert_eq!(source, "from DAQ_PORT env var");
    }

    #[test]
    fn port_default_when_nothing_set() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe { clear_env_vars() };
        let (port, source) = resolve_port(None);
        assert_eq!(port, DEFAULT_PORT);
        assert_eq!(source, "default");
    }

    #[test]
    fn port_invalid_env_var_falls_back_to_default() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_PORT", "not_a_number");
        }
        let (port, source) = resolve_port(None);
        assert_eq!(port, DEFAULT_PORT);
        assert_eq!(source, "default");
    }

    // ---- resolve_hardware_config tests ----

    #[test]
    fn config_cli_flag_wins_over_env() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_CONFIG_PATH", "/env/path.toml");
        }
        let cli_path = Some(PathBuf::from("/cli/path.toml"));
        let (config, source) = resolve_hardware_config(cli_path);
        assert_eq!(
            config.as_deref(),
            Some(std::path::Path::new("/cli/path.toml"))
        );
        assert_eq!(source, "from --hardware-config flag");
    }

    #[test]
    fn config_env_var_used_when_no_cli_flag() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_CONFIG_PATH", "/env/path.toml");
        }
        let (config, source) = resolve_hardware_config(None);
        assert_eq!(
            config.as_deref(),
            Some(std::path::Path::new("/env/path.toml"))
        );
        assert_eq!(source, "from DAQ_CONFIG_PATH env var");
    }

    #[test]
    fn config_none_when_nothing_set() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe { clear_env_vars() };
        let (config, source) = resolve_hardware_config(None);
        assert!(config.is_none());
        assert_eq!(source, "default");
    }

    #[test]
    fn config_empty_env_var_treated_as_unset() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_CONFIG_PATH", "");
        }
        let (config, source) = resolve_hardware_config(None);
        assert!(config.is_none());
        assert_eq!(source, "default");
    }

    // ---- resolve_runtime_mode tests ----

    #[test]
    fn runtime_mode_cli_flag_wins_over_env() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_RUNTIME_MODE", "mock");
        }
        let (mode, source) = resolve_runtime_mode(Some(RuntimeMode::Native));
        assert!(matches!(mode, Some(RuntimeMode::Native)));
        assert_eq!(source, "from --runtime-mode flag");
    }

    #[test]
    fn runtime_mode_daq_env_wins_over_legacy_env() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_RUNTIME_MODE", "mock");
            std::env::set_var("RUSTDAQ_RUNTIME_MODE", "native");
        }
        let (mode, source) = resolve_runtime_mode(None);
        assert!(matches!(mode, Some(RuntimeMode::Mock)));
        assert_eq!(source, "from DAQ_RUNTIME_MODE env var");
    }

    #[test]
    fn runtime_mode_legacy_env_used_as_fallback() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("RUSTDAQ_RUNTIME_MODE", "universal");
        }
        let (mode, source) = resolve_runtime_mode(None);
        assert!(matches!(mode, Some(RuntimeMode::Universal)));
        assert_eq!(source, "from RUSTDAQ_RUNTIME_MODE env var");
    }

    #[test]
    fn runtime_mode_none_when_nothing_set() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe { clear_env_vars() };
        let (mode, source) = resolve_runtime_mode(None);
        assert!(mode.is_none());
        assert_eq!(source, "default");
    }

    #[test]
    fn runtime_mode_invalid_env_var_falls_back_to_none() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_RUNTIME_MODE", "invalid_mode");
        }
        let (mode, source) = resolve_runtime_mode(None);
        assert!(mode.is_none());
        assert_eq!(source, "default");
    }

    #[test]
    fn runtime_mode_hybrid_db_parsed_correctly() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_RUNTIME_MODE", "hybrid-db");
        }
        let (mode, source) = resolve_runtime_mode(None);
        assert!(matches!(mode, Some(RuntimeMode::HybridDb)));
        assert_eq!(source, "from DAQ_RUNTIME_MODE env var");
    }

    // ---- resolve_data_path tests ----

    #[test]
    fn data_path_cli_flag_wins_over_env() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_DATA_PATH", "/env/data");
        }
        let cli_path = Some(PathBuf::from("/cli/data"));
        let (path, source) = resolve_data_path(cli_path);
        assert_eq!(path.as_deref(), Some(std::path::Path::new("/cli/data")));
        assert_eq!(source, "from --data-path flag");
    }

    #[test]
    fn data_path_env_var_used_when_no_cli_flag() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_DATA_PATH", "/env/data");
        }
        let (path, source) = resolve_data_path(None);
        assert_eq!(path.as_deref(), Some(std::path::Path::new("/env/data")));
        assert_eq!(source, "from DAQ_DATA_PATH env var");
    }

    #[test]
    fn data_path_none_when_nothing_set() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe { clear_env_vars() };
        let (path, source) = resolve_data_path(None);
        assert!(path.is_none());
        assert_eq!(source, "default");
    }

    #[test]
    fn data_path_empty_env_var_treated_as_unset() {
        let _lock = ENV_LOCK.lock().expect("test mutex poisoned");
        // SAFETY: ENV_LOCK held; no concurrent env access.
        unsafe {
            clear_env_vars();
            std::env::set_var("DAQ_DATA_PATH", "");
        }
        let (path, source) = resolve_data_path(None);
        assert!(path.is_none());
        assert_eq!(source, "default");
    }
}
