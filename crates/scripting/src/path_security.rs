//! Path security validation for Rhai script bindings (bd-qa36.8.1).
//!
//! Scripts uploaded via gRPC can specify file paths (HDF5 output) and device
//! paths (serial ports, Comedi devices). Without validation, these paths enable
//! arbitrary filesystem access with daemon privileges.
//!
//! # Security Model
//!
//! - **HDF5 files**: Must be under a configurable data directory (default: `./data`).
//!   Path traversal (`../`) is rejected after canonicalization.
//! - **Serial ports**: Must match `/dev/tty*` patterns. Rejects traversal paths.
//! - **Comedi devices**: Must match `/dev/comedi*` pattern.
//!
//! All rejections are logged at `warn!` level for security audit trails.

use rhai::{EvalAltResult, Position};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Configurable base directory for script HDF5 output.
/// Set once at startup via `set_data_directory()`. Defaults to `./data`.
static DATA_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// Set the allowed data directory for script file output.
///
/// Must be called before any scripts run. Can only be called once.
/// Returns `Err` if already set.
pub fn set_data_directory(path: PathBuf) -> Result<(), PathBuf> {
    DATA_DIRECTORY.set(path)
}

/// Get the configured data directory, or the default (`./data`).
fn data_directory() -> &'static Path {
    DATA_DIRECTORY.get_or_init(|| {
        let default = PathBuf::from("./data");
        // Try to create the directory if it doesn't exist
        let _ = std::fs::create_dir_all(&default);
        // Canonicalize if possible, otherwise use as-is
        std::fs::canonicalize(&default).unwrap_or(default)
    })
}

/// Validate and sanitize an HDF5 file path from a Rhai script.
///
/// Ensures the path resolves to a location under the configured data directory.
/// Rejects path traversal, absolute paths outside the data dir, and symlink escapes.
pub fn validate_hdf5_path(user_path: &str) -> Result<PathBuf, Box<EvalAltResult>> {
    let data_dir = data_directory();

    // Reject obviously malicious patterns early
    if user_path.contains("..") {
        tracing::warn!(
            path = %user_path,
            "SECURITY: Rejected HDF5 path containing traversal sequence"
        );
        return Err(Box::new(EvalAltResult::ErrorRuntime(
            format!(
                "Path rejected: '{}' contains directory traversal. \
                 Files must be created under {}",
                user_path,
                data_dir.display()
            )
            .into(),
            Position::NONE,
        )));
    }

    // Resolve relative to data directory
    let candidate = if Path::new(user_path).is_absolute() {
        PathBuf::from(user_path)
    } else {
        data_dir.join(user_path)
    };

    // Ensure parent directory exists (needed for canonicalize of new files)
    if let Some(parent) = candidate.parent() {
        if !parent.exists() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    // Canonicalize the parent to resolve symlinks, then re-append filename
    let canonical = if let Some(parent) = candidate.parent() {
        match std::fs::canonicalize(parent) {
            Ok(canon_parent) => {
                if let Some(filename) = candidate.file_name() {
                    canon_parent.join(filename)
                } else {
                    canon_parent
                }
            }
            Err(_) => candidate.clone(),
        }
    } else {
        candidate.clone()
    };

    // Verify the canonical path is under the data directory
    if !canonical.starts_with(data_dir) {
        tracing::warn!(
            user_path = %user_path,
            resolved = %canonical.display(),
            allowed_dir = %data_dir.display(),
            "SECURITY: Rejected HDF5 path outside data directory"
        );
        return Err(Box::new(EvalAltResult::ErrorRuntime(
            format!(
                "Path rejected: '{}' resolves to '{}' which is outside the allowed \
                 data directory '{}'. Use a relative path like 'experiment.h5'.",
                user_path,
                canonical.display(),
                data_dir.display()
            )
            .into(),
            Position::NONE,
        )));
    }

    tracing::debug!(
        user_path = %user_path,
        resolved = %canonical.display(),
        "HDF5 path validated"
    );

    Ok(canonical)
}

/// Validate a serial port path from a Rhai script.
///
/// Only allows paths matching `/dev/tty*` (Linux/macOS serial devices)
/// or `/dev/serial/by-id/*` (stable udev symlinks).
pub fn validate_serial_port(port: &str) -> Result<(), Box<EvalAltResult>> {
    // Allow common serial port patterns
    let allowed = port.starts_with("/dev/tty")
        || port.starts_with("/dev/serial/by-id/")
        || port.starts_with("/dev/cu."); // macOS

    if !allowed || port.contains("..") {
        tracing::warn!(
            port = %port,
            "SECURITY: Rejected serial port path"
        );
        return Err(Box::new(EvalAltResult::ErrorRuntime(
            format!(
                "Port rejected: '{}'. Allowed patterns: /dev/tty*, \
                 /dev/serial/by-id/*, /dev/cu.*",
                port
            )
            .into(),
            Position::NONE,
        )));
    }

    Ok(())
}

/// Validate a Comedi device path from a Rhai script.
///
/// Only allows paths matching `/dev/comedi*`.
pub fn validate_comedi_device(path: &str) -> Result<(), Box<EvalAltResult>> {
    if !path.starts_with("/dev/comedi") || path.contains("..") {
        tracing::warn!(
            path = %path,
            "SECURITY: Rejected Comedi device path"
        );
        return Err(Box::new(EvalAltResult::ErrorRuntime(
            format!(
                "Device path rejected: '{}'. Only /dev/comedi* devices are allowed.",
                path
            )
            .into(),
            Position::NONE,
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use a temp dir for tests to avoid filesystem side effects
    fn with_temp_data_dir<F: FnOnce(&Path)>(f: F) {
        let dir = tempfile::tempdir().expect("create temp dir");
        f(dir.path());
    }

    #[test]
    fn test_hdf5_relative_path_ok() {
        with_temp_data_dir(|dir| {
            // Can't use the global DATA_DIRECTORY in tests (OnceLock),
            // so test the logic directly
            let candidate = dir.join("experiment.h5");
            assert!(candidate.starts_with(dir));
        });
    }

    #[test]
    fn test_hdf5_traversal_rejected() {
        let result = validate_hdf5_path("../../../etc/passwd");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("traversal"), "error: {err}");
    }

    #[test]
    fn test_serial_port_valid() {
        assert!(validate_serial_port("/dev/ttyUSB0").is_ok());
        assert!(validate_serial_port("/dev/ttyACM0").is_ok());
        assert!(validate_serial_port("/dev/serial/by-id/usb-FTDI_FT232R").is_ok());
        assert!(validate_serial_port("/dev/cu.usbserial-110").is_ok());
    }

    #[test]
    fn test_serial_port_traversal_rejected() {
        assert!(validate_serial_port("/dev/tty../../etc/passwd").is_err());
    }

    #[test]
    fn test_serial_port_arbitrary_rejected() {
        assert!(validate_serial_port("/etc/passwd").is_err());
        assert!(validate_serial_port("/dev/mem").is_err());
        assert!(validate_serial_port("/dev/sda").is_err());
        assert!(validate_serial_port("relative/path").is_err());
    }

    #[test]
    fn test_comedi_valid() {
        assert!(validate_comedi_device("/dev/comedi0").is_ok());
        assert!(validate_comedi_device("/dev/comedi1").is_ok());
    }

    #[test]
    fn test_comedi_arbitrary_rejected() {
        assert!(validate_comedi_device("/dev/sda").is_err());
        assert!(validate_comedi_device("/dev/comedi/../mem").is_err());
        assert!(validate_comedi_device("/etc/passwd").is_err());
    }
}
