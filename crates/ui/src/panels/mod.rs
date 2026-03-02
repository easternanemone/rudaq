//! UI panels for the DAQ control application.

mod code_preview;
mod devices;
mod document_viewer;
mod getting_started;
mod image_viewer;
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
mod storage;

// --- Native-only panels (depend on hardware/experiment/comedi crates) ---
#[cfg(not(target_arch = "wasm32"))]
pub mod comedi;
#[cfg(not(target_arch = "wasm32"))]
mod experiment_designer;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod instrument_manager;

pub use code_preview::CodePreviewPanel;
pub use devices::DevicesPanel;
pub use document_viewer::DocumentViewerPanel;
pub use getting_started::GettingStartedPanel;
pub use image_viewer::ImageViewerPanel;
#[allow(unused_imports)] // Spectrum types will be consumed when UI wiring is complete
pub use live_visualization::{
    data_channel, frame_channel, spectrum_channel, DataUpdate, DataUpdateSender, FrameUpdate,
    FrameUpdateSender, LiveVisualizationPanel, SpectrumUpdate, SpectrumUpdateSender,
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

// --- Native-only panel exports ---
#[cfg(not(target_arch = "wasm32"))]
pub use comedi::ComediPanel;
#[cfg(not(target_arch = "wasm32"))]
pub use experiment_designer::ExperimentDesignerPanel;
#[cfg(not(target_arch = "wasm32"))]
pub use instrument_manager::InstrumentManagerPanel;

// --- WASM stub types for native-only panels ---
// These provide the same API surface so DaqApp compiles on both platforms.
// Each stub renders a "not available in browser" placeholder.

#[cfg(target_arch = "wasm32")]
pub use wasm_stubs::*;

#[cfg(target_arch = "wasm32")]
mod wasm_stubs {
    use crate::runtime::Runtime;
    use client::DaqClient;
    use protocol::daq::DeviceInfo;

    /// Stub for InstrumentManagerPanel on WASM.
    pub struct InstrumentManagerPanel;

    /// Stub for pop-out request on WASM (always None).
    pub struct PopOutRequest {
        pub device_info: DeviceInfo,
    }

    impl Default for InstrumentManagerPanel {
        fn default() -> Self {
            Self
        }
    }

    impl InstrumentManagerPanel {
        pub fn ui(
            &mut self,
            ui: &mut egui::Ui,
            _client: Option<&mut DaqClient>,
            _runtime: &Runtime,
        ) {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(
                    egui::RichText::new("Instrument Manager")
                        .heading()
                        .color(egui::Color32::GRAY),
                );
                ui.label("Hardware configuration is not available in the browser.");
                ui.label("Connect to a running daemon to control devices via the Devices panel.");
            });
        }

        pub fn reset_refresh_state(&mut self) {}

        pub fn take_pop_out_request(&mut self) -> Option<PopOutRequest> {
            None
        }

        pub fn take_image_viewer_request(&mut self) -> Option<String> {
            None
        }
    }

    /// Stub for ExperimentDesignerPanel on WASM.
    pub struct ExperimentDesignerPanel;

    impl Default for ExperimentDesignerPanel {
        fn default() -> Self {
            Self
        }
    }

    impl ExperimentDesignerPanel {
        pub fn ui(
            &mut self,
            ui: &mut egui::Ui,
            _client: Option<&mut DaqClient>,
            _runtime: Option<&Runtime>,
        ) {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(
                    egui::RichText::new("Experiment Designer")
                        .heading()
                        .color(egui::Color32::GRAY),
                );
                ui.label("The node graph experiment designer requires native execution.");
                ui.label("Use the desktop application for experiment design.");
            });
        }
    }

    /// Stub for ComediPanel on WASM.
    #[derive(Default)]
    pub struct ComediPanel;

    impl ComediPanel {
        pub fn ui(
            &mut self,
            ui: &mut egui::Ui,
            _client: Option<&mut DaqClient>,
            _runtime: &Runtime,
        ) {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(
                    egui::RichText::new("Comedi DAQ")
                        .heading()
                        .color(egui::Color32::GRAY),
                );
                ui.label("NI DAQ hardware control requires a Linux host with Comedi drivers.");
            });
        }
    }
}
