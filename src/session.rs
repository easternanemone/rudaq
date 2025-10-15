//! Session management for the DAQ application.

use crate::config::StorageSettings;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::app::DaqApp;

use std::collections::VecDeque;

/// Represents the state of the application to be saved in a session.
#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub active_instruments: HashSet<String>,
    pub storage_settings: StorageSettings,
    pub gui_state: GuiState,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GuiState {
    pub plot_data: VecDeque<[f64; 2]>,
}

impl Session {
    /// Creates a new `Session` from the current application state.
    pub fn from_app(app: &DaqApp, gui_state: GuiState) -> Self {
        app.with_inner(|inner| {
            let active_instruments = inner.instruments.keys().cloned().collect();
            let storage_settings = inner.settings.storage.clone();
            Self {
                active_instruments,
                storage_settings,
                gui_state,
            }
        })
    }

    /// Applies the session state to the application.
    pub fn apply_to_app(&self, app: &DaqApp) {
        app.with_inner(|inner| {
            // Stop all current instruments
            let current_instruments: Vec<String> = inner.instruments.keys().cloned().collect();
            for id in current_instruments {
                inner.stop_instrument(&id);
            }

            // Start instruments from the session
            for id in &self.active_instruments {
                if let Err(e) = inner.spawn_instrument(id) {
                    log::error!("Failed to start instrument from session '{}': {}", id, e);
                }
            }

            // Apply storage settings
            let settings = Arc::make_mut(&mut inner.settings);
            settings.storage = self.storage_settings.clone();
        });
    }
}

/// Saves the current session to a file.
pub fn save_session(session: &Session, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(session)?;
    fs::write(path, json)?;
    Ok(())
}

/// Loads a session from a file.
pub fn load_session(path: &Path) -> Result<Session> {
    let json = fs::read_to_string(path)?;
    let session = serde_json::from_str(&json)?;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_save_and_load_session() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_session.json");

        let mut active_instruments = HashSet::new();
        active_instruments.insert("mock_instrument".to_string());
        active_instruments.insert("another_instrument".to_string());

        let storage_settings = StorageSettings {
            default_path: "test/path".to_string(),
            default_format: "json".to_string(),
        };

        let gui_state = GuiState {
            plot_data: vec![[0.0, 1.0], [1.0, 2.0]].into(),
        };

        let session_to_save = Session {
            active_instruments: active_instruments.clone(),
            storage_settings: storage_settings.clone(),
            gui_state: gui_state.clone(),
        };

        // Save the session
        let save_result = save_session(&session_to_save, &file_path);
        assert!(save_result.is_ok());

        // Load the session
        let loaded_session = load_session(&file_path).unwrap();

        // Verify the contents
        assert_eq!(
            session_to_save.active_instruments,
            loaded_session.active_instruments
        );
        assert_eq!(
            session_to_save.storage_settings.default_path,
            loaded_session.storage_settings.default_path
        );
        assert_eq!(
            session_to_save.storage_settings.default_format,
            loaded_session.storage_settings.default_format
        );
        assert_eq!(
            session_to_save.gui_state.plot_data,
            loaded_session.gui_state.plot_data
        );
    }
}
