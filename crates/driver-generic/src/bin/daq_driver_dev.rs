//! Interactive driver development REPL for declarative serial drivers.
//!
//! This tool provides an interactive environment for developing and debugging
//! TOML-based serial device drivers without running the full DAQ system.

use anyhow::Result;
use clap::Parser;
use driver_generic::repl::Repl;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "daq-driver-dev")]
#[command(version)]
#[command(about = "Interactive driver development tool")]
#[command(long_about = "
Interactive REPL for developing and testing declarative serial device drivers.

Load a device TOML config, validate its schema, test serial commands interactively,
and debug response parsing without running the full DAQ daemon.

Example usage:
  daq-driver-dev config/devices/my_device.scpi.toml --port /dev/ttyUSB0
  daq-driver-dev config/devices/my_device.scpi.toml --mock
")]
struct Args {
    /// Path to device TOML config
    config: PathBuf,

    /// Serial port (e.g., /dev/ttyUSB0)
    #[arg(short, long)]
    port: Option<String>,

    /// Run in mock mode (no hardware, use test responses)
    #[arg(long)]
    mock: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing for debug output
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let args = Args::parse();

    // Create and run REPL
    let mut repl = Repl::new(&args.config, args.port, args.mock).await?;
    repl.run().await
}
