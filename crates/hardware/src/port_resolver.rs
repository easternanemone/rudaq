//! Serial port resolution based on hardware identifiers.
//!
//! This module provides stable serial port mapping using hardware identifiers
//! (vendor, model, serial number) instead of fragile paths like `/dev/ttyUSB0`
//! that can change between reboots.
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_hardware::port_resolver::{PortSpec, resolve_port};
//!
//! // Direct path (used as-is)
//! let port = resolve_port("/dev/ttyUSB0")?;
//!
//! // By-ID symlink (stable, recommended)
//! let port = resolve_port("/dev/serial/by-id/usb-FTDI_FT230X_DJ00XXXX-if00-port0")?;
//!
//! // Hardware spec (auto-resolved)
//! let spec = PortSpec::new().vendor("FTDI").model("FT230X");
//! let port = spec.resolve()?;
//! ```
//!
//! # Linux `/dev/serial/by-id/` Format
//!
//! Linux udev creates stable symlinks in `/dev/serial/by-id/` with the format:
//! ```text
//! usb-{VENDOR}_{MODEL}_{SERIAL}-if{INTERFACE}-port{PORT}
//! ```
//!
//! Examples:
//! - `usb-FTDI_FT230X_Basic_UART_DJ00XXXX-if00-port0` (FTDI chip)
//! - `usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_0001-if00-port0` (CP2102)

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur during port resolution.
#[derive(Debug, Error)]
pub enum PortResolveError {
    /// The specified port path does not exist.
    #[error("Port not found: {0}")]
    PortNotFound(String),

    /// No port matches the given hardware specification.
    #[error("No port found matching spec: vendor={vendor:?}, model={model:?}, serial={serial:?}")]
    NoMatch {
        vendor: Option<String>,
        model: Option<String>,
        serial: Option<String>,
    },

    /// Multiple ports match the specification (ambiguous).
    #[error("Multiple ports match spec: {0:?}")]
    AmbiguousMatch(Vec<String>),

    /// The `/dev/serial/by-id/` directory is not available (not Linux or no serial devices).
    #[error("Serial by-id directory not available: {0}")]
    ByIdNotAvailable(String),

    /// IO error during port resolution.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Hardware specification for finding a serial port.
///
/// At least one of `vendor`, `model`, or `serial` must be specified.
/// If multiple fields are specified, all must match.
#[derive(Debug, Clone, Default)]
pub struct PortSpec {
    /// USB vendor name (e.g., "FTDI", "Silicon_Labs")
    pub vendor: Option<String>,
    /// USB model/product name (e.g., "FT230X_Basic_UART", "CP2102")
    pub model: Option<String>,
    /// USB serial number (e.g., "DJ00XXXX", "0001")
    pub serial: Option<String>,
}

impl PortSpec {
    /// Create an empty port specification.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the vendor name filter.
    pub fn vendor(mut self, vendor: impl Into<String>) -> Self {
        self.vendor = Some(vendor.into());
        self
    }

    /// Set the model name filter.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the serial number filter.
    pub fn serial(mut self, serial: impl Into<String>) -> Self {
        self.serial = Some(serial.into());
        self
    }

    /// Check if this spec has at least one filter criterion.
    pub fn is_valid(&self) -> bool {
        self.vendor.is_some() || self.model.is_some() || self.serial.is_some()
    }

    /// Resolve this specification to a port path.
    ///
    /// Searches `/dev/serial/by-id/` for matching devices.
    pub fn resolve(&self) -> Result<String, PortResolveError> {
        if !self.is_valid() {
            return Err(PortResolveError::NoMatch {
                vendor: None,
                model: None,
                serial: None,
            });
        }

        let by_id_dir = Path::new("/dev/serial/by-id");
        if !by_id_dir.exists() {
            return Err(PortResolveError::ByIdNotAvailable(
                "Directory /dev/serial/by-id does not exist".to_string(),
            ));
        }

        let mut matches = Vec::new();

        for entry in std::fs::read_dir(by_id_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if self.matches_name(&name_str) {
                matches.push(entry.path().to_string_lossy().into_owned());
            }
        }

        match matches.len() {
            0 => Err(PortResolveError::NoMatch {
                vendor: self.vendor.clone(),
                model: self.model.clone(),
                serial: self.serial.clone(),
            }),
            1 => Ok(matches.into_iter().next().expect("checked len == 1")),
            _ => Err(PortResolveError::AmbiguousMatch(matches)),
        }
    }

    /// Check if a by-id filename matches this specification.
    ///
    /// The by-id format is: `usb-{VENDOR}_{MODEL}_{SERIAL}-if{N}-port{N}`
    fn matches_name(&self, name: &str) -> bool {
        // Must start with "usb-"
        let Some(name) = name.strip_prefix("usb-") else {
            return false;
        };

        // Check vendor if specified
        if let Some(ref vendor) = self.vendor {
            if !name.starts_with(vendor) {
                return false;
            }
        }

        // Check model if specified (appears after vendor_)
        if let Some(ref model) = self.model {
            if !name.contains(model) {
                return false;
            }
        }

        // Check serial if specified
        if let Some(ref serial) = self.serial {
            if !name.contains(serial) {
                return false;
            }
        }

        true
    }
}

/// Resolve a port path, handling direct paths, by-id paths, and relative specs.
///
/// # Arguments
///
/// * `port_or_spec` - Either:
///   - A direct device path (e.g., `/dev/ttyUSB0`)
///   - A by-id symlink path (e.g., `/dev/serial/by-id/usb-FTDI_...`)
///   - A short by-id name (e.g., `usb-FTDI_FT230X_DJ00XXXX-if00-port0`)
///
/// # Returns
///
/// The resolved port path that can be passed to serial port libraries.
pub fn resolve_port(port_or_spec: &str) -> Result<String, PortResolveError> {
    // If it's already a full path that exists, use it
    if port_or_spec.starts_with("/dev/") {
        let path = Path::new(port_or_spec);
        if path.exists() {
            // Canonicalize to resolve symlinks
            return Ok(std::fs::canonicalize(path)?.to_string_lossy().into_owned());
        } else {
            return Err(PortResolveError::PortNotFound(port_or_spec.to_string()));
        }
    }

    // If it looks like a by-id name (starts with "usb-"), prepend the directory
    if port_or_spec.starts_with("usb-") {
        let full_path = PathBuf::from("/dev/serial/by-id").join(port_or_spec);
        if full_path.exists() {
            return Ok(std::fs::canonicalize(&full_path)?
                .to_string_lossy()
                .into_owned());
        } else {
            return Err(PortResolveError::PortNotFound(
                full_path.to_string_lossy().into_owned(),
            ));
        }
    }

    // Otherwise, treat as a direct path and check existence
    let path = Path::new(port_or_spec);
    if path.exists() {
        Ok(std::fs::canonicalize(path)?.to_string_lossy().into_owned())
    } else {
        Err(PortResolveError::PortNotFound(port_or_spec.to_string()))
    }
}

/// List all available serial ports with their hardware identifiers.
///
/// Returns a list of (device_path, by_id_name) tuples.
pub fn list_ports() -> Result<Vec<PortInfo>, PortResolveError> {
    let by_id_dir = Path::new("/dev/serial/by-id");
    if !by_id_dir.exists() {
        return Err(PortResolveError::ByIdNotAvailable(
            "Directory /dev/serial/by-id does not exist".to_string(),
        ));
    }

    let mut ports = Vec::new();

    for entry in std::fs::read_dir(by_id_dir)? {
        let entry = entry?;
        let by_id_path = entry.path();
        let by_id_name = entry.file_name().to_string_lossy().into_owned();

        // Resolve the symlink to get the actual device path
        let device_path = std::fs::canonicalize(&by_id_path)?
            .to_string_lossy()
            .into_owned();

        // Parse vendor/model/serial from the by-id name
        let parsed = parse_by_id_name(&by_id_name);

        ports.push(PortInfo {
            device_path,
            by_id_path: by_id_path.to_string_lossy().into_owned(),
            by_id_name,
            vendor: parsed.0,
            model: parsed.1,
            serial: parsed.2,
        });
    }

    // Sort by device path for consistent ordering
    ports.sort_by(|a, b| a.device_path.cmp(&b.device_path));

    Ok(ports)
}

/// Information about an available serial port.
#[derive(Debug, Clone)]
pub struct PortInfo {
    /// The actual device path (e.g., `/dev/ttyUSB0`)
    pub device_path: String,
    /// The full by-id symlink path
    pub by_id_path: String,
    /// The by-id symlink name only
    pub by_id_name: String,
    /// Parsed vendor name (if available)
    pub vendor: Option<String>,
    /// Parsed model name (if available)
    pub model: Option<String>,
    /// Parsed serial number (if available)
    pub serial: Option<String>,
}

/// Parse vendor, model, and serial from a by-id name.
///
/// Format: `usb-{VENDOR}_{MODEL}_{SERIAL}-if{N}-port{N}`
fn parse_by_id_name(name: &str) -> (Option<String>, Option<String>, Option<String>) {
    // Strip "usb-" prefix
    let Some(name) = name.strip_prefix("usb-") else {
        return (None, None, None);
    };

    // Strip "-ifN-portN" suffix
    let name = if let Some(idx) = name.find("-if") {
        &name[..idx]
    } else {
        name
    };

    // Split by underscore - first is vendor, last is usually serial, middle is model
    let parts: Vec<&str> = name.split('_').collect();

    match parts.len() {
        0 => (None, None, None),
        1 => (Some(parts[0].to_string()), None, None),
        2 => (Some(parts[0].to_string()), None, Some(parts[1].to_string())),
        _ => {
            // First is vendor, last is serial, middle parts are model
            let vendor = parts[0].to_string();
            let serial = parts.last().expect("checked len >= 3").to_string();
            let model = parts[1..parts.len() - 1].join("_");
            (Some(vendor), Some(model), Some(serial))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_spec_matching() {
        let spec = PortSpec::new().vendor("FTDI").model("FT230X");

        assert!(spec.matches_name("usb-FTDI_FT230X_Basic_UART_DJ00XXXX-if00-port0"));
        assert!(!spec.matches_name("usb-Silicon_Labs_CP2102_0001-if00-port0"));
    }

    #[test]
    fn test_parse_by_id_name() {
        let (vendor, model, serial) =
            parse_by_id_name("usb-FTDI_FT230X_Basic_UART_DJ00XXXX-if00-port0");
        assert_eq!(vendor, Some("FTDI".to_string()));
        assert_eq!(model, Some("FT230X_Basic_UART".to_string()));
        assert_eq!(serial, Some("DJ00XXXX".to_string()));

        let (vendor, model, serial) =
            parse_by_id_name("usb-Silicon_Labs_CP2102_USB_to_UART_Bridge_0001-if00-port0");
        assert_eq!(vendor, Some("Silicon".to_string())); // Note: underscore in vendor name
        assert_eq!(model, Some("Labs_CP2102_USB_to_UART_Bridge".to_string()));
        assert_eq!(serial, Some("0001".to_string()));
    }

    #[test]
    fn test_port_spec_validation() {
        let empty = PortSpec::new();
        assert!(!empty.is_valid());

        let with_vendor = PortSpec::new().vendor("FTDI");
        assert!(with_vendor.is_valid());
    }
}
