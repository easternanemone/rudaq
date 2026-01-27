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
//! # Use lab hardware (future - currently placeholder)
//! rust-daq-gui --lab-hardware
//! ```

#[cfg(feature = "standalone")]
mod app;
mod client;
mod connection;
mod connection_state_ext;
mod daemon_launcher;
#[cfg(feature = "standalone")]
mod graph;
#[cfg(feature = "standalone")]
mod gui_log_layer;
#[cfg(feature = "standalone")]
mod icons;
#[cfg(feature = "standalone")]
mod layout;
#[cfg(feature = "standalone")]
mod panels;
mod reconnect;
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

    /// Use real lab hardware configuration (future implementation)
    ///
    /// TODO: When implemented, this will launch the daemon with --lab-hardware flag
    /// to use the pre-configured lab setup at maitai@100.117.5.12
    #[arg(long)]
    lab_hardware: bool,

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
