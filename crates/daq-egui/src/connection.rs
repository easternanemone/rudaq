//! Daemon connection configuration.
//!
//! Most types are re-exported from `daq_client`. Storage helpers remain here.

// Re-export everything from daq-client
pub use daq_client::connection::*;

/// Save a daemon address to eframe::Storage.
pub fn save_daemon_address(storage: &mut dyn eframe::Storage, address: &DaemonAddress) {
    storage.set_string(STORAGE_KEY_DAEMON_ADDR, address.as_str().to_string());
}

/// Load persisted daemon address string from storage.
/// Returns the raw URL string, not a parsed DaemonAddress.
pub fn load_daemon_address(storage: &dyn eframe::Storage) -> Option<String> {
    storage.get_string(STORAGE_KEY_DAEMON_ADDR)
}
