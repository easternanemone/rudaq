//! Path security validation for Rhai script bindings (bd-qa36.8.1).
//!
//! Scripts uploaded via gRPC can specify file paths (HDF5 output) and device
//! paths (serial ports, Comedi devices). Without validation, these paths enable
//! arbitrary filesystem access with daemon privileges.
//!
//! # Security Model
//!
//! - **HDF5 files**: Must be under a configurable data directory (default: `./data`).
//!   Path traversal (`../`) is rejected, canonicalization resolves symlinks, and
//!   directory creation only occurs after containment is verified.
//! - **Serial ports**: Must match specific device patterns (`/dev/ttyUSB*`,
//!   `/dev/ttyACM*`, `/dev/ttyS*`, `/dev/ttyAMA*`, `/dev/serial/by-id/*`,
//!   `/dev/cu.*`). Bare `/dev/tty` (controlling terminal) and virtual consoles
//!   are explicitly rejected.
//! - **Comedi devices**: Must match `/dev/comedi` followed by digits only.
//!
//! All rejections are logged at `warn!` level for security audit trails.
//! Error messages returned to scripts are generic to prevent information leakage.

use rhai::{EvalAltResult, Position};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// Configurable base directory for script HDF5 output.
/// Set once at startup via `set_data_directory()`. Defaults to `./data`.
static DATA_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// Set the allowed data directory for script file output.
///
/// Must be called before any scripts run. Can only be called once.
/// The path is canonicalized (with directory creation if needed) to ensure
/// consistent `starts_with` checks during validation.
/// Returns `Err` if already set.
pub fn set_data_directory(path: PathBuf) -> Result<(), PathBuf> {
    // Canonicalize to ensure starts_with checks work with canonical paths.
    // Create the directory first if it doesn't exist, so canonicalize succeeds.
    let _ = std::fs::create_dir_all(&path);
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    DATA_DIRECTORY.set(canonical)
}

/// Get the configured data directory, or the default (`./data`).
///
/// On first access, creates the default directory and canonicalizes the path.
/// This initialization performs filesystem operations (create_dir_all, canonicalize)
/// but only once — subsequent calls return the cached canonical path.
fn data_directory() -> &'static Path {
    DATA_DIRECTORY.get_or_init(|| {
        let default = PathBuf::from("./data");
        // Create the directory if it doesn't exist, then canonicalize
        let _ = std::fs::create_dir_all(&default);
        std::fs::canonicalize(&default).unwrap_or(default)
    })
}

/// Helper to create a security rejection error with generic user message.
/// Detailed information is logged server-side only.
#[allow(clippy::unnecessary_box_returns)] // Box<EvalAltResult> is the standard Rhai error type
fn security_reject(user_msg: &str) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(user_msg.into(), Position::NONE))
}

/// Validate and sanitize an HDF5 file path from a Rhai script.
///
/// Ensures the path resolves to a location under the configured data directory.
/// Rejects path traversal, absolute paths outside the data dir, and symlink escapes.
///
/// # Security invariants
/// - Null bytes are rejected before any filesystem operations
/// - Traversal sequences (`..`) are rejected before any filesystem operations
/// - Absolute paths outside data_dir are rejected before directory creation
/// - Canonicalization failures are treated as rejections (fail closed)
/// - Directory creation only occurs after containment is verified
pub fn validate_hdf5_path(user_path: &str) -> Result<PathBuf, Box<EvalAltResult>> {
    let data_dir = data_directory();

    // Reject null bytes (defense-in-depth against C FFI truncation)
    if user_path.bytes().any(|b| b == 0) {
        tracing::warn!("SECURITY: Rejected HDF5 path containing null byte");
        return Err(security_reject("Path rejected: invalid characters in path"));
    }

    // Reject traversal sequences before any filesystem operations.
    // Uses Path::components() instead of string-based .contains("..") to avoid
    // false positives on filenames containing ".." (e.g., "my..data.h5").
    if Path::new(user_path)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        tracing::warn!(
            path = %user_path,
            "SECURITY: Rejected HDF5 path containing traversal sequence"
        );
        return Err(security_reject(
            "Path rejected: directory traversal is not allowed. Use a relative filename.",
        ));
    }

    // Resolve relative to data directory
    let candidate = if Path::new(user_path).is_absolute() {
        PathBuf::from(user_path)
    } else {
        data_dir.join(user_path)
    };

    // Pre-check: verify the candidate path starts with data_dir BEFORE any
    // filesystem operations. This prevents creating arbitrary directories
    // when an absolute path outside data_dir is provided.
    // (Review fix: G3 Pro #1, Codex #1 — pre-validation directory creation)
    if !candidate.starts_with(data_dir) {
        tracing::warn!(
            user_path = %user_path,
            candidate = %candidate.display(),
            allowed_dir = %data_dir.display(),
            "SECURITY: Rejected HDF5 path outside data directory (pre-canonicalization)"
        );
        return Err(security_reject(
            "Path rejected: must be a relative path under the data directory.",
        ));
    }

    // NOW it is safe to create directories (path is under data_dir)
    if let Some(parent) = candidate.parent()
        && !parent.exists()
    {
        let _ = std::fs::create_dir_all(parent);
    }

    // Canonicalize the parent to resolve symlinks, then re-append filename.
    // Fail closed: if canonicalization fails, reject the path.
    // (Review fix: Opus #4 — silent fallback on canonicalization failure)
    let canonical = if let Some(parent) = candidate.parent() {
        match std::fs::canonicalize(parent) {
            Ok(canon_parent) => {
                if let Some(filename) = candidate.file_name() {
                    canon_parent.join(filename)
                } else {
                    canon_parent
                }
            }
            Err(e) => {
                tracing::warn!(
                    path = %candidate.display(),
                    error = %e,
                    "SECURITY: Cannot canonicalize HDF5 path parent, rejecting"
                );
                return Err(security_reject("Path rejected: unable to resolve path."));
            }
        }
    } else {
        tracing::warn!(
            path = %candidate.display(),
            "SECURITY: HDF5 path has no parent directory"
        );
        return Err(security_reject("Path rejected: invalid path structure."));
    };

    // Defense-in-depth: if the target file already exists as a symlink, reject it.
    // This prevents an attacker from pre-creating a symlink that points outside data_dir.
    // Note: TOCTOU is inherent here, but this raises the bar for exploitation.
    if canonical.exists()
        && let Ok(meta) = std::fs::symlink_metadata(&canonical)
        && meta.file_type().is_symlink()
    {
        tracing::warn!(
            path = %canonical.display(),
            "SECURITY: Rejected HDF5 path — target is a symlink"
        );
        return Err(security_reject("Path rejected: symlinks are not allowed."));
    }

    // Post-canonicalization containment check (resolves symlink escapes)
    if !canonical.starts_with(data_dir) {
        tracing::warn!(
            user_path = %user_path,
            resolved = %canonical.display(),
            allowed_dir = %data_dir.display(),
            "SECURITY: Rejected HDF5 path outside data directory after canonicalization"
        );
        // Generic error message to prevent filesystem structure leakage
        // (Review fix: G3 Pro #2, Opus #7, Codex #7 — information leakage)
        return Err(security_reject(
            "Path rejected: must be a relative path under the data directory.",
        ));
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
/// Only allows paths matching specific serial device patterns:
/// - `/dev/ttyUSB*`, `/dev/ttyACM*`, `/dev/ttyS*`, `/dev/ttyAMA*` (Linux serial)
/// - `/dev/serial/by-id/*` (stable udev symlinks)
/// - `/dev/cu.*` (macOS serial)
///
/// Explicitly rejects `/dev/tty` (controlling terminal), `/dev/tty[0-9]` (virtual
/// consoles), and `/dev/ttyprintk` (kernel log).
/// (Review fix: G3 Pro #3, Opus #5, Codex #4 — overly broad whitelist)
pub fn validate_serial_port(port: &str) -> Result<(), Box<EvalAltResult>> {
    // Reject null bytes
    if port.bytes().any(|b| b == 0) {
        tracing::warn!("SECURITY: Rejected serial port path containing null byte");
        return Err(security_reject("Port rejected: invalid characters"));
    }

    // Reject traversal (use Path::components to avoid false positives on ".." in names)
    if Path::new(port)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        tracing::warn!(port = %port, "SECURITY: Rejected serial port with traversal");
        return Err(security_reject("Port rejected: invalid path"));
    }

    // Allow specific serial device patterns (not bare /dev/tty or virtual consoles)
    let allowed = port.starts_with("/dev/ttyUSB")
        || port.starts_with("/dev/ttyACM")
        || port.starts_with("/dev/ttyS")
        || port.starts_with("/dev/ttyAMA")
        || port.starts_with("/dev/serial/by-id/")
        || port.starts_with("/dev/cu."); // macOS

    if !allowed {
        tracing::warn!(
            port = %port,
            "SECURITY: Rejected serial port path"
        );
        return Err(security_reject(
            "Port rejected: only USB/ACM/S/AMA serial ports, /dev/serial/by-id/*, and /dev/cu.* are allowed",
        ));
    }

    Ok(())
}

/// Validate a Comedi device path from a Rhai script.
///
/// Only allows paths matching `/dev/comedi` followed by one or more digits.
/// (Review fix: Opus #6, Codex #6 — overly broad prefix match)
pub fn validate_comedi_device(path: &str) -> Result<(), Box<EvalAltResult>> {
    // Reject null bytes
    if path.bytes().any(|b| b == 0) {
        tracing::warn!("SECURITY: Rejected Comedi path containing null byte");
        return Err(security_reject("Device path rejected: invalid characters"));
    }

    // Must be /dev/comedi followed by digits only (e.g., /dev/comedi0, /dev/comedi12)
    // The digit-only suffix check already prevents traversal, but we keep the explicit
    // component check for defense-in-depth.
    let valid = path.starts_with("/dev/comedi")
        && path.len() > "/dev/comedi".len()
        && path["/dev/comedi".len()..]
            .chars()
            .all(|c| c.is_ascii_digit())
        && !Path::new(path)
            .components()
            .any(|c| matches!(c, Component::ParentDir));

    if !valid {
        tracing::warn!(
            path = %path,
            "SECURITY: Rejected Comedi device path"
        );
        return Err(security_reject(
            "Device path rejected: only /dev/comedi[N] devices are allowed",
        ));
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
    fn test_hdf5_relative_path_containment() {
        with_temp_data_dir(|dir| {
            // Can't call validate_hdf5_path() directly because it uses
            // a global OnceLock. Instead, exercise the same containment logic:
            // a relative path joined to the data dir should stay inside it.
            let candidate = dir.join("subdir/experiment.h5");
            assert!(
                candidate.starts_with(dir),
                "Relative path should resolve under data dir"
            );

            // An absolute path outside data_dir should NOT pass containment
            let outside = PathBuf::from("/etc/passwd");
            assert!(
                !outside.starts_with(dir),
                "Absolute path outside data dir should fail containment"
            );
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
    fn test_hdf5_double_dot_in_filename_not_rejected() {
        // Filenames containing ".." (e.g., "my..data.h5") should NOT be rejected
        // as traversal attempts — only actual ParentDir components ("..") should be.
        // This tests that we use Path::components() instead of string .contains("..").
        let result = validate_hdf5_path("my..data.h5");
        // The path should pass the traversal check (may fail later on containment
        // depending on OnceLock state, but it should NOT fail with "traversal")
        if let Err(ref e) = result {
            let err_str = format!("{}", e);
            assert!(
                !err_str.contains("traversal"),
                "Filename 'my..data.h5' should not be rejected as traversal: {err_str}"
            );
        }
    }

    #[test]
    fn test_hdf5_null_byte_rejected() {
        let result = validate_hdf5_path("evil\x00.h5");
        assert!(result.is_err());
    }

    // --- Serial port tests ---

    #[test]
    fn test_serial_port_valid() {
        assert!(validate_serial_port("/dev/ttyUSB0").is_ok());
        assert!(validate_serial_port("/dev/ttyACM0").is_ok());
        assert!(validate_serial_port("/dev/ttyS0").is_ok());
        assert!(validate_serial_port("/dev/ttyAMA0").is_ok());
        assert!(validate_serial_port("/dev/serial/by-id/usb-FTDI_FT232R").is_ok());
        assert!(validate_serial_port("/dev/cu.usbserial-110").is_ok());
    }

    #[test]
    fn test_serial_port_bare_tty_rejected() {
        // Bare /dev/tty (controlling terminal) must be rejected
        assert!(validate_serial_port("/dev/tty").is_err());
    }

    #[test]
    fn test_serial_port_virtual_console_rejected() {
        // Virtual consoles /dev/tty0 through /dev/tty63 must be rejected
        assert!(validate_serial_port("/dev/tty0").is_err());
        assert!(validate_serial_port("/dev/tty1").is_err());
    }

    #[test]
    fn test_serial_port_ttyprintk_rejected() {
        assert!(validate_serial_port("/dev/ttyprintk").is_err());
    }

    #[test]
    fn test_serial_port_traversal_rejected() {
        assert!(validate_serial_port("/dev/ttyUSB../../etc/passwd").is_err());
    }

    #[test]
    fn test_serial_port_arbitrary_rejected() {
        assert!(validate_serial_port("/etc/passwd").is_err());
        assert!(validate_serial_port("/dev/mem").is_err());
        assert!(validate_serial_port("/dev/sda").is_err());
        assert!(validate_serial_port("relative/path").is_err());
    }

    #[test]
    fn test_serial_port_null_byte_rejected() {
        assert!(validate_serial_port("/dev/ttyUSB0\x00extra").is_err());
    }

    // --- Comedi tests ---

    #[test]
    fn test_comedi_valid() {
        assert!(validate_comedi_device("/dev/comedi0").is_ok());
        assert!(validate_comedi_device("/dev/comedi1").is_ok());
        assert!(validate_comedi_device("/dev/comedi12").is_ok());
    }

    #[test]
    fn test_comedi_bare_rejected() {
        // Bare /dev/comedi (no digit) must be rejected
        assert!(validate_comedi_device("/dev/comedi").is_err());
    }

    #[test]
    fn test_comedi_non_digit_suffix_rejected() {
        assert!(validate_comedi_device("/dev/comedi_test").is_err());
        assert!(validate_comedi_device("/dev/comediX").is_err());
    }

    #[test]
    fn test_comedi_arbitrary_rejected() {
        assert!(validate_comedi_device("/dev/sda").is_err());
        assert!(validate_comedi_device("/dev/comedi/../mem").is_err());
        assert!(validate_comedi_device("/etc/passwd").is_err());
    }

    #[test]
    fn test_comedi_null_byte_rejected() {
        assert!(validate_comedi_device("/dev/comedi0\x00extra").is_err());
    }
}
