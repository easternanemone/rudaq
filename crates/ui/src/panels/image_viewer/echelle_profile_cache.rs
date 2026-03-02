//! Echelle calibration profile cache for ImageViewerPanel.
//!
//! Keeps the last-good profile loaded while capturing reload errors, enabling
//! hot-reload-safe behavior during iterative calibration editing.

use common::echelle::EchelleCalibrationProfile;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EchelleProfileCacheEvent {
    Unchanged,
    Loaded(PathBuf),
    Error(String),
    Cleared,
}

#[derive(Debug, Clone, Default)]
pub struct EchelleProfileCache {
    path: Option<PathBuf>,
    last_modified: Option<SystemTime>,
    profile: Option<Arc<EchelleCalibrationProfile>>,
    last_error: Option<String>,
}

impl EchelleProfileCache {
    pub fn set_path(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if self.path.as_ref() != Some(&path) {
            self.path = Some(path);
            self.last_modified = None; // force reload on next poll
            self.last_error = None;
        }
    }

    pub fn clear(&mut self) -> EchelleProfileCacheEvent {
        self.path = None;
        self.last_modified = None;
        self.profile = None;
        self.last_error = None;
        EchelleProfileCacheEvent::Cleared
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn profile(&self) -> Option<&Arc<EchelleCalibrationProfile>> {
        self.profile.as_ref()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn poll_reload_if_changed(&mut self) -> EchelleProfileCacheEvent {
        let Some(path) = self.path.clone() else {
            return EchelleProfileCacheEvent::Unchanged;
        };

        let metadata = match fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                let msg = format!("Failed to stat echelle profile {}: {}", path.display(), e);
                if self.last_error.as_deref() != Some(&msg) {
                    self.last_error = Some(msg.clone());
                    return EchelleProfileCacheEvent::Error(msg);
                }
                return EchelleProfileCacheEvent::Unchanged;
            }
        };
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        if self.last_modified == Some(modified) {
            return EchelleProfileCacheEvent::Unchanged;
        }

        match EchelleCalibrationProfile::load_from_path(&path) {
            Ok(profile) => {
                self.profile = Some(Arc::new(profile));
                self.last_modified = Some(modified);
                self.last_error = None;
                EchelleProfileCacheEvent::Loaded(path)
            }
            Err(e) => {
                // Preserve last-good profile for hot-reload-safe behavior.
                let msg = format!("Failed to load echelle profile {}: {}", path.display(), e);
                if self.last_error.as_deref() != Some(&msg) {
                    self.last_error = Some(msg.clone());
                    EchelleProfileCacheEvent::Error(msg)
                } else {
                    EchelleProfileCacheEvent::Unchanged
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fixture_text() -> &'static str {
        include_str!("../../../../common/tests/fixtures/echelle_profile_v1.toml")
    }

    #[test]
    fn cache_loads_profile_and_reports_loaded() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile.toml");
        std::fs::write(&path, fixture_text()).unwrap();

        let mut cache = EchelleProfileCache::default();
        cache.set_path(&path);
        let event = cache.poll_reload_if_changed();
        assert!(matches!(event, EchelleProfileCacheEvent::Loaded(_)));
        assert!(cache.profile().is_some());
        assert!(cache.last_error().is_none());
    }

    #[test]
    fn cache_returns_unchanged_when_file_not_modified() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile.toml");
        std::fs::write(&path, fixture_text()).unwrap();

        let mut cache = EchelleProfileCache::default();
        cache.set_path(&path);
        let _ = cache.poll_reload_if_changed();
        let event = cache.poll_reload_if_changed();
        assert_eq!(event, EchelleProfileCacheEvent::Unchanged);
    }

    #[test]
    fn cache_preserves_last_good_profile_on_reload_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("profile.toml");
        std::fs::write(&path, fixture_text()).unwrap();

        let mut cache = EchelleProfileCache::default();
        cache.set_path(&path);
        let _ = cache.poll_reload_if_changed();
        let before = cache.profile().cloned();

        // Rewrite invalid content and bump mtime.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(&path, "not valid = [").unwrap();

        let event = cache.poll_reload_if_changed();
        assert!(matches!(event, EchelleProfileCacheEvent::Error(_)));
        assert!(cache.profile().is_some());
        assert_eq!(
            cache.profile().map(|p| p.display_name.clone()),
            before.map(|p| p.display_name.clone())
        );
    }
}
