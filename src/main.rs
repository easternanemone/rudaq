//! Application entry point.
use anyhow::Result;
use eframe::NativeOptions;
use log::{info, LevelFilter};
use rust_daq::{
    app::DaqApp,
    config::Settings,
    data::registry::ProcessorRegistry,
    gui::Gui,
    instrument::{mock::MockInstrument, scpi::ScpiInstrument, InstrumentRegistry},
    log_capture::{LogBuffer, LogCollector},
};
use std::sync::Arc;

fn main() -> Result<()> {
    // --- Custom Log Initialization ---
    // Create a shared buffer for log messages
    let log_buffer = LogBuffer::new();

    // Create a logger that collects messages for the GUI
    let gui_logger = LogCollector::new(log_buffer.clone());

    // Get the desired log level from the environment or default to "info"
    let log_level_filter = std::env::var("RUST_LOG").map_or(LevelFilter::Info, |s| {
        s.parse().unwrap_or(LevelFilter::Info)
    });

    // Create a logger that prints to the console
    let console_logger = env_logger::Builder::new()
        .filter_level(log_level_filter)
        .build();

    // Combine the loggers. All log messages will be sent to both the
    // console and our GUI log collector.
    log::set_max_level(log_level_filter);
    multi_log::MultiLogger::init(
        vec![Box::new(console_logger), Box::new(gui_logger)],
        log_level_filter.to_level().unwrap_or(log::Level::Info),
    )
    .map_err(|e| anyhow::anyhow!("Failed to initialize logger: {}", e))?;
    // --- End of Log Initialization ---

    // Load configuration
    let settings = Arc::new(Settings::new(None)?);
    info!("Configuration loaded successfully.");

    // Create a registry and register available instruments.
    // This is our static "plugin" system.
    let mut instrument_registry = InstrumentRegistry::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));
    instrument_registry.register("scpi_keithley", |_id| Box::new(ScpiInstrument::new()));

    #[cfg(feature = "instrument_visa")]
    {
        use rust_daq::instrument::visa::VisaInstrument;
        // The unwrap here is not ideal, but the factory function doesn't
        // support returning a Result. If creating a VISA instrument fails,
        // the application will panic, which is acceptable for now.
        instrument_registry.register("visa_instrument", |id| {
            Box::new(VisaInstrument::new(id).unwrap())
        });
    }

    let instrument_registry = Arc::new(instrument_registry);

    // Create the processor registry
    let processor_registry = Arc::new(ProcessorRegistry::new());

    // Create the core application state
    let app = DaqApp::new(
        settings.clone(),
        instrument_registry,
        processor_registry,
        log_buffer,
    )?;
    let app_clone = app.clone();

    // Set up and run the GUI
    let options = NativeOptions::default();
    info!("Starting GUI...");

    eframe::run_native(
        "Rust DAQ",
        options,
        Box::new(move |cc| {
            // The `eframe` crate provides the `egui` context `cc`
            // which we can use to style the GUI.
            // Here we are just passing it to our `Gui` struct.
            Box::new(Gui::new(cc, app_clone))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Eframe run error: {}", e))?;

    // The GUI event loop has finished.
    // We can now gracefully shut down the application.
    info!("GUI closed. Shutting down.");
    app.shutdown();

    Ok(())
}
