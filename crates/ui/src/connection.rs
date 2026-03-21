//! Daemon connection configuration.
//!
//! Most types are re-exported from `daq_client`. Storage helpers remain here.

// Re-export everything from daq-client
pub use client::connection::*;

/// LEGACY: Migrate orphaned "daemon_address" key to AppSettings on first load.
/// Returns the migrated address if found, None otherwise.
/// Remove after v1.0 when all labs have loaded the UI at least once.
/// See docs/reference/deprecation-plan.md Section 3.6.
pub fn migrate_legacy_daemon_address(storage: &dyn eframe::Storage) -> Option<String> {
    storage.get_string(STORAGE_KEY_DAEMON_ADDR)
}

/// LEGACY: Blank the legacy daemon_address key after successful migration.
///
/// Sets the value to an empty string rather than removing the key,
/// since `eframe::Storage` does not expose a `remove` method.
pub fn clear_legacy_daemon_address(storage: &mut dyn eframe::Storage) {
    storage.set_string(STORAGE_KEY_DAEMON_ADDR, String::new());
}
