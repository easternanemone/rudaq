//! UI panels for the DAQ control application.

mod code_preview;
pub mod comedi;
mod devices;
mod document_viewer;
mod experiment_designer;
mod getting_started;
mod image_viewer;
mod instrument_manager;
mod live_visualization;
mod logging;
mod modules;
mod multi_detector_grid;
mod plan_runner;
mod run_comparison;
mod run_history;
mod scan_builder;
mod scans;
mod script_editor;
mod scripts;
mod signal_plotter;
mod signal_plotter_stream;
mod storage;

// Comedi panels for NI DAQ control
pub use code_preview::CodePreviewPanel;
pub use comedi::ComediPanel;
pub use devices::DevicesPanel;
pub use document_viewer::DocumentViewerPanel;
pub use experiment_designer::ExperimentDesignerPanel;
pub use getting_started::GettingStartedPanel;
pub use image_viewer::ImageViewerPanel;
pub use instrument_manager::InstrumentManagerPanel;
pub use live_visualization::{
    data_channel, frame_channel, DataUpdate, DataUpdateSender, FrameUpdate, FrameUpdateSender,
    LiveVisualizationPanel,
};
pub use logging::{ConnectionDiagnostics, ConnectionStatus, LogLevel, LoggingPanel};
pub use modules::ModulesPanel;
pub use multi_detector_grid::{DetectorPanel, DetectorType, MultiDetectorGrid};
pub use plan_runner::PlanRunnerPanel;
pub use run_comparison::RunComparisonPanel;
pub use run_history::RunHistoryPanel;
pub use scan_builder::ScanBuilderPanel;
pub use scans::ScansPanel;
pub use script_editor::ScriptEditorPanel;
pub use scripts::ScriptsPanel;
pub use signal_plotter::SignalPlotterPanel;
pub use storage::StoragePanel;
