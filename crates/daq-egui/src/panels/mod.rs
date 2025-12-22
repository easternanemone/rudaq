//! UI panels for the DAQ control application.

mod devices;
mod scripts;
mod scans;
mod storage;
mod modules;
mod getting_started;
mod plan_runner;
mod document_viewer;
mod instrument_manager;
mod signal_plotter;
mod signal_plotter_stream;
mod image_viewer;
mod logging;

pub use devices::DevicesPanel;
pub use scripts::ScriptsPanel;
pub use scans::ScansPanel;
pub use storage::StoragePanel;
pub use modules::ModulesPanel;
pub use getting_started::GettingStartedPanel;
pub use plan_runner::PlanRunnerPanel;
pub use document_viewer::DocumentViewerPanel;
pub use instrument_manager::InstrumentManagerPanel;
pub use signal_plotter::SignalPlotterPanel;
pub use image_viewer::ImageViewerPanel;
pub use logging::{LoggingPanel, LogLevel, ConnectionStatus};
