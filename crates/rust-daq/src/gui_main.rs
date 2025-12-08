//! Native egui/eframe GUI for rust-daq.
//!
//! This provides a lightweight, cross-platform control panel that talks to the
//! headless daemon over gRPC using a proper message-passing architecture.
//!
//! Architecture (validated by Codex):
//! - UI Thread: egui immediate-mode rendering, never blocks
//! - Backend Thread: tokio runtime with gRPC client, manages streams
//! - Communication: Channel-based message passing (watch + mpsc)
//!
//! Build (with networking enabled):
//! ```bash
//! cargo run --features "networking,gui_egui" --bin rust_daq_gui_egui
//! ```

#![cfg(all(feature = "networking", feature = "gui_egui"))]

use eframe::egui;
use rust_daq::gui::app::DaqGuiApp;

pub fn main() -> eframe::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rust_daq=debug".parse().unwrap())
                .add_directive("gui=debug".parse().unwrap()),
        )
        .init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "rust-daq Control Panel",
        native_options,
        Box::new(|_cc| Ok(Box::new(DaqGuiApp::new()))),
    )
}


