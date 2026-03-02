//! DAQ Control Panel - egui desktop application
//!
//! A lightweight GUI for controlling the headless rust-daq daemon via gRPC.
//!
//! # Usage
//!
//! ```bash
//! # Default: auto-start local mock daemon and connect
//! rust-daq-gui
//!
//! # Connect to a remote daemon (skip auto-start)
//! rust-daq-gui --daemon-url http://192.168.1.100:50051
//!
//! # Explicit local runtime mode
//! rust-daq-gui --runtime-mode universal
//!
//! # Backward-compatible alias for native lab mode
//! rust-daq-gui --lab-hardware
//! ```

mod runtime;
mod time;

#[cfg(feature = "standalone")]
mod app;
#[cfg(feature = "standalone")]
mod client;
#[cfg(feature = "standalone")]
mod connection;
#[cfg(feature = "standalone")]
mod connection_state_ext;
#[cfg(feature = "standalone")]
mod daemon_launcher;
#[cfg(feature = "standalone")]
mod device_ext;
#[cfg(feature = "standalone")]
mod export;
#[cfg(feature = "standalone")]
mod graph;
#[cfg(feature = "standalone")]
mod gui_config;
#[cfg(feature = "standalone")]
mod gui_log_layer;
#[cfg(feature = "standalone")]
mod icons;
#[cfg(feature = "standalone")]
mod layout;
#[cfg(feature = "standalone")]
mod panels;
#[cfg(feature = "standalone")]
mod reconnect;
#[cfg(feature = "standalone")]
mod settings;
#[cfg(feature = "standalone")]
mod shortcuts;
#[cfg(feature = "standalone")]
mod theme;
#[cfg(feature = "standalone")]
mod widgets;

#[cfg(feature = "standalone")]
use clap::Parser;
#[cfg(feature = "standalone")]
use daemon_launcher::DaemonMode;
#[cfg(feature = "standalone")]
use eframe::egui;
#[cfg(feature = "standalone")]
use tracing_subscriber::layer::SubscriberExt;
#[cfg(feature = "standalone")]
use tracing_subscriber::util::SubscriberInitExt;

/// Explicit local runtime mode for daemon auto-start.
#[cfg(feature = "standalone")]
#[derive(clap::ValueEnum, Debug, Clone)]
enum RuntimeModeArg {
    Mock,
    Native,
    Universal,
    HybridDb,
}

#[cfg(feature = "standalone")]
impl RuntimeModeArg {
    fn to_daemon_mode(&self, port: u16) -> DaemonMode {
        match self {
            Self::Mock => DaemonMode::LocalAuto { port },
            Self::Native => DaemonMode::LabHardware { port },
            Self::Universal => DaemonMode::LabUniversal { port },
            Self::HybridDb => DaemonMode::LabHybridDb { port },
        }
    }
}

/// DAQ Control Panel - GUI for controlling the rust-daq daemon
#[cfg(feature = "standalone")]
#[derive(Parser)]
#[command(name = "rust-daq-gui")]
#[command(about = "DAQ Control Panel GUI for controlling the rust-daq daemon")]
#[command(version)]
struct Cli {
    /// Connect to a remote daemon at the specified URL (skips auto-start)
    ///
    /// Example: --daemon-url http://192.168.1.100:50051
    #[arg(long, value_name = "URL")]
    daemon_url: Option<String>,

    /// Use real lab hardware configuration
    ///
    /// Launches the daemon with --lab-hardware flag to use the pre-configured
    /// lab setup (see config/maitai_universal.toml)
    #[arg(long)]
    lab_hardware: bool,

    /// Explicit local runtime mode for auto-started daemon.
    ///
    /// Modes:
    /// - mock: local mock devices
    /// - native: native maitai hardware config
    /// - universal: universal TOML maitai profile
    /// - hybrid-db: universal profile with SurrealDB control-plane expectations
    #[arg(
        long,
        value_enum,
        value_name = "MODE",
        conflicts_with_all = ["daemon_url", "preset", "lab_hardware"]
    )]
    runtime_mode: Option<RuntimeModeArg>,

    /// Use a named connection preset from gui.toml
    ///
    /// Loads config/gui.toml and connects using the named preset.
    /// Example: --preset "Maitai Lab"
    #[arg(long, value_name = "NAME")]
    preset: Option<String>,

    /// Daemon port when auto-starting (default: 50051)
    #[arg(long, default_value = "50051")]
    port: u16,
}

#[cfg(feature = "standalone")]
fn main() -> eframe::Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Determine daemon mode from CLI arguments
    let daemon_mode = if let Some(url) = cli.daemon_url {
        DaemonMode::Remote { url }
    } else if let Some(preset_name) = cli.preset {
        // Load gui.toml and find the named preset
        match gui_config::load_gui_config() {
            Ok(Some(config)) => {
                let preset = config
                    .presets
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(&preset_name));
                match preset {
                    Some(p) => DaemonMode::Remote {
                        url: p.grpc_url.clone(),
                    },
                    None => {
                        let names: Vec<_> =
                            config.presets.iter().map(|p| p.name.as_str()).collect();
                        let available = if names.is_empty() {
                            "(none)".to_string()
                        } else {
                            names.join(", ")
                        };
                        eprintln!("Unknown preset '{}'. Available: {}", preset_name, available);
                        std::process::exit(1);
                    }
                }
            }
            Ok(None) => {
                eprintln!("No gui.toml found. Cannot use --preset flag.");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("Failed to load gui.toml: {}", e);
                std::process::exit(1);
            }
        }
    } else if let Some(runtime_mode) = cli.runtime_mode {
        runtime_mode.to_daemon_mode(cli.port)
    } else if cli.lab_hardware {
        DaemonMode::LabHardware { port: cli.port }
    } else {
        DaemonMode::LocalAuto { port: cli.port }
    };

    // Create channel for GUI log events
    let (log_sender, log_receiver) = gui_log_layer::create_log_channel();

    // Initialize logging with GUI layer
    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(gui_log_layer::GuiLogLayer::new(log_sender))
        .init();

    tracing::info!(
        "Starting DAQ Control Panel (mode: {}, url: {})",
        daemon_mode.label(),
        daemon_mode.daemon_url()
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("DAQ Control Panel"),
        ..Default::default()
    };

    eframe::run_native(
        "DAQ Control Panel",
        options,
        Box::new(move |cc| Ok(Box::new(app::DaqApp::new(cc, daemon_mode, log_receiver)))),
    )
}

#[cfg(not(feature = "standalone"))]
fn main() {
    eprintln!("The rust-daq-gui binary requires the 'standalone' feature (enabled by default).");
    std::process::exit(1);
}
