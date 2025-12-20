//! UI panels for the DAQ control application.

mod connection;
mod devices;
mod scripts;
mod scans;
mod storage;
mod modules;
mod getting_started;
mod plan_runner;
mod document_viewer;

pub use connection::ConnectionPanel;
pub use devices::DevicesPanel;
pub use scripts::ScriptsPanel;
pub use scans::ScansPanel;
pub use storage::StoragePanel;
pub use modules::ModulesPanel;
pub use getting_started::GettingStartedPanel;
pub use plan_runner::PlanRunnerPanel;
pub use document_viewer::DocumentViewerPanel;
