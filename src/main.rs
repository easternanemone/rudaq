//! Application entry point for the native GUI.
//!
//! This file is the executable entry point for the `rust_daq` application. Its primary
//! responsibilities are:
//!
//! 1.  **Initialization**: It sets up the application's core components, including:
//!     -   **Logging**: Configures a multi-backend logger (`multi_log`) that directs log
//!         messages to both the standard console (via `env_logger`) and a custom in-memory
//!         buffer for display in the GUI (`log_capture::LogCollector`). This provides
//!         both persistent and interactive logging.
//!     -   **Configuration**: Loads the `settings.toml` file into the `Settings` struct.
//!     -   **Instrument Registry**: Creates an `InstrumentRegistry` and registers all the
//!         available instrument drivers. This acts as a simple "plugin" system where
//!         instrument types are mapped to their constructor functions. Feature flags
//!         are used to conditionally compile and register drivers that have specific
//!         dependencies.
//!     -   **Processor Registry**: Creates a `ProcessorRegistry` for data processing modules.
//!     -   **Core Application (`DaqApp`)**: Instantiates the central `DaqApp` which manages
//!         the application's state and logic.
//!
//! 2.  **GUI Launch**: It configures and runs the native GUI using the `eframe` crate.
//!     It passes the `DaqApp` instance to the `gui::Gui` struct, which then takes control
//!     of the main event loop.
//!
//! 3.  **Shutdown**: After the `eframe` event loop exits (i.e., the user closes the window),
//!     it calls the `app.shutdown()` method to ensure a graceful termination of all
//!     background tasks, such as instrument communication threads.

use anyhow::Result;
use eframe::NativeOptions;
use log::{info, LevelFilter};
use rust_daq::{
    app::DaqApp,
    config::Settings,
    core::DataPoint,
    data::registry::ProcessorRegistry,
    gui::Gui,
    instrument::{mock::MockInstrument, scpi::ScpiInstrument, InstrumentRegistry},
    log_capture::{LogBuffer, LogCollector},
    measurement::{datapoint::*, InstrumentMeasurement},
    modules::{ModuleRegistry, power_meter::PowerMeterModule},
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
    let settings = Settings::new(None)?;
    info!("Configuration loaded successfully.");

    // Create a registry and register available instruments.
    // This is our static "plugin" system.
    let mut instrument_registry = InstrumentRegistry::<InstrumentMeasurement>::new();
    instrument_registry.register("mock", |_id| Box::new(MockInstrument::new()));

    // Note: V2 instruments (MockInstrumentV2, etc.) will be integrated in Phase 3 (bd-51)
    // via native Arc<Measurement> support. V2InstrumentAdapter removed in Phase 2 (bd-62).

    instrument_registry.register("scpi_keithley", |_id| Box::new(ScpiInstrument::new()));

    #[cfg(feature = "instrument_visa")]
    {
        use rust_daq::instrument::visa::VisaInstrument;
        instrument_registry.register("visa_instrument", |id| {
            Box::new(VisaInstrument::new(id).unwrap())
        });
    }

    #[cfg(feature = "instrument_serial")]
    {
        use rust_daq::instrument::{
            elliptec::Elliptec, esp300::ESP300, maitai::MaiTai, newport_1830c::Newport1830C,
        };
        instrument_registry.register("newport_1830c", |id| Box::new(Newport1830C::new(id)));
        instrument_registry.register("maitai", |id| Box::new(MaiTai::new(id)));
        instrument_registry.register("elliptec", |id| Box::new(Elliptec::new(id)));
        instrument_registry.register("esp300", |id| Box::new(ESP300::new(id)));
    }

    // Register PVCAM camera (no feature gate for now)
    use rust_daq::instrument::pvcam::PVCAMCamera;
    instrument_registry.register("pvcam", |id| Box::new(PVCAMCamera::new(id)));

    // Register V2 PVCAM camera for image support
    use rust_daq::instrument::v2_adapter::V2InstrumentAdapter;
    use rust_daq::instruments_v2::pvcam::PVCAMInstrumentV2;

    instrument_registry.register("pvcam_v2", |id| {
        Box::new(V2InstrumentAdapter::new(
            PVCAMInstrumentV2::new(id.to_string())
        ))
    });

    let instrument_registry = Arc::new(instrument_registry);

    // Create the processor registry
    let processor_registry = Arc::new(ProcessorRegistry::new());

    // Create the module registry and register modules
    let mut module_registry = ModuleRegistry::<InstrumentMeasurement>::new();
    module_registry.register("power_meter", |id| {
        Box::new(PowerMeterModule::<InstrumentMeasurement>::new(id))
    });
    let module_registry = Arc::new(module_registry);

    // Create the core application state
    let app = DaqApp::<InstrumentMeasurement>::new(
        settings,
        instrument_registry,
        processor_registry,
        module_registry,
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
            Ok(Box::new(Gui::<InstrumentMeasurement>::new(cc, app_clone)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Eframe run error: {}", e))?;

    // The GUI event loop has finished.
    // We can now gracefully shut down the application.
    info!("GUI closed. Shutting down.");
    app.shutdown()?;

    Ok(())
}