//! UI panels for the DAQ control application.

pub mod comedi;
mod devices;
mod document_viewer;
mod getting_started;
mod image_viewer;
mod instrument_manager;
mod logging;
mod modules;
mod plan_runner;
mod scan_builder;
mod scans;
mod scripts;
mod signal_plotter;
mod signal_plotter_stream;
mod storage;

// WIP Comedi panels - not yet integrated into the main UI.
// Uncomment when Comedi gRPC interface is complete.
// pub use comedi::{
//     AnalogInputPanel, AnalogOutputPanel, ComediPanel, CounterPanel, DigitalIOPanel,
//     OscilloscopePanel,
// };
pub use devices::DevicesPanel;
pub use document_viewer::DocumentViewerPanel;
pub use getting_started::GettingStartedPanel;
pub use image_viewer::ImageViewerPanel;
pub use instrument_manager::InstrumentManagerPanel;
pub use logging::{ConnectionDiagnostics, ConnectionStatus, LogLevel, LoggingPanel};
pub use modules::ModulesPanel;
pub use plan_runner::PlanRunnerPanel;
pub use scan_builder::ScanBuilderPanel;
pub use scans::ScansPanel;
pub use scripts::ScriptsPanel;
pub use signal_plotter::SignalPlotterPanel;
pub use storage::StoragePanel;
