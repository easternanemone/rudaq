// Allow expected warnings in this FFI/mock driver crate:
// - unsafe_code: FFI requires unsafe blocks
// - dead_code: Hardware-only structs have fields unused in mock mode
// - unused_variables: Mock implementations may not use all parameters
// - unused_imports: Some imports are only used in hardware mode
#![allow(unsafe_code, dead_code, unused_variables, unused_imports)]
// Clippy allows for FFI/mock patterns:
#![allow(
    clippy::missing_safety_doc,
    clippy::new_without_default,
    clippy::unused_async,
    clippy::collapsible_if
)]

//! Andor iStar Camera and Shamrock Spectrograph Driver
//!
//! Safe Rust wrapper for Andor SDK3, providing drivers for:
//! - **Andor iStar**: Intensified CCD camera with MCP gain and DDG timing
//! - **Shamrock Spectrograph**: Grating-based spectrograph with wavelength tuning
//!
//! # Architecture
//!
//! This crate follows the componentized driver pattern:
//! - `camera/`: iStar camera driver (FrameProducer + Triggerable + Parameterized)
//! - `spectrograph/`: Shamrock spectrograph driver (WavelengthTunable + ShutterControl + Parameterized)
//! - `mock/`: Full mock implementations for cross-platform development
//! - `factory/`: DriverFactory implementations for plugin registration
//!
//! # Features
//!
//! - `camera`: Enable iStar camera driver
//! - `spectrograph`: Enable Shamrock spectrograph driver
//! - `hardware`: Enable real SDK3 hardware (Windows only)
//! - Default: Mock implementations only
//!
//! # Safety
//!
//! The SDK3 API uses wide strings (UTF-16) and manual memory management.
//! This crate provides safe abstractions over the unsafe FFI layer.
//!
//! # Example
//!
//! ```rust,no_run
//! use driver_andor_sdk3::camera::AndorCamera;
//! use common::capabilities::{FrameProducer, Triggerable, ExposureControl};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create camera (validates device identity)
//! let camera = AndorCamera::new_async(0).await?;
//!
//! // Configure acquisition
//! camera.set_trigger_mode("External").await?;
//! camera.set_exposure(0.001).await?;  // 1ms exposure
//! camera.set_mcp_gain(3600).await?;   // MCP gain
//!
//! // Start streaming
//! camera.start_stream().await?;
//! # Ok(())
//! # }
//! ```

pub mod buffer;
pub mod camera;
pub mod error;
pub mod factory;
pub(crate) mod ffi_timeout;
pub mod introspection;
pub mod mock;
pub mod spectrograph;
pub mod types;

// Re-export main types
pub use camera::AndorCamera;
pub use error::{AndorError, AndorResult};
pub use factory::{AndorCameraFactory, AndorSpectrographFactory};
pub use spectrograph::AndorSpectrograph;
pub use types::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RuntimeCleanupReport {
    removed: Vec<String>,
    failed: Vec<String>,
}

impl RuntimeCleanupReport {
    pub(crate) fn removed_count(&self) -> usize {
        self.removed.len()
    }

    pub(crate) fn failed_count(&self) -> usize {
        self.failed.len()
    }

    pub(crate) fn summary(&self) -> String {
        format!(
            "{} removed, {} failed",
            self.removed_count(),
            self.failed_count()
        )
    }
}

pub(crate) fn cleanup_runtime_artifacts(roots: &[&str], prefixes: &[&str]) -> RuntimeCleanupReport {
    let normalized_prefixes: Vec<String> = prefixes
        .iter()
        .map(|prefix| prefix.to_ascii_lowercase())
        .collect();
    let mut report = RuntimeCleanupReport::default();

    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy().to_ascii_lowercase();
            if !normalized_prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }

            let path = entry.path();
            let removal_result = match entry.file_type() {
                Ok(file_type) if file_type.is_dir() => std::fs::remove_dir_all(&path),
                Ok(_) => std::fs::remove_file(&path),
                Err(err) => Err(err),
            };

            match removal_result {
                Ok(()) => report.removed.push(path.display().to_string()),
                Err(err) => report.failed.push(format!("{}: {}", path.display(), err)),
            }
        }
    }

    report
}

/// Linker reference function to ensure this crate is not stripped.
///
/// Called by `drivers::link_drivers()` when the `andor_sdk3` feature is enabled.
#[inline(never)]
pub fn link() {
    std::hint::black_box(std::any::TypeId::of::<AndorCamera>());
    std::hint::black_box(std::any::TypeId::of::<AndorSpectrograph>());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn cleanup_runtime_artifacts_removes_prefixed_entries() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rust-daq-andor-cleanup-{}-{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&root).unwrap();

        let removable = root.join("andor-stale.lock");
        let keep = root.join("unrelated.lock");
        std::fs::write(&removable, b"stale").unwrap();
        std::fs::write(&keep, b"keep").unwrap();

        let report = cleanup_runtime_artifacts(&[root.to_str().unwrap()], &["andor", "sem.atcore"]);

        assert_eq!(report.removed_count(), 1);
        assert_eq!(report.failed_count(), 0);
        assert!(!removable.exists());
        assert!(keep.exists());

        std::fs::remove_file(keep).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
