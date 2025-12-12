//! UI panels for the DAQ control application.

mod connection;
mod devices;
mod scripts;
mod scans;

pub use connection::ConnectionPanel;
pub use devices::DevicesPanel;
pub use scripts::ScriptsPanel;
pub use scans::ScansPanel;
