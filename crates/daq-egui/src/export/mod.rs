//! CSV export functionality for signal and image data
//!
//! This module provides CSV export capabilities for:
//! - Signal traces from SignalPlotter
//! - Line profiles from ImageViewer (future)
//! - ROI statistics over time (future)
//!
//! Features:
//! - Configurable delimiter (comma, tab, semicolon)
//! - Optional header row with column labels
//! - Metadata comments (prefixed with #)
//! - Large dataset support via streaming writes

mod signal;

pub use signal::{export_signal_traces, SignalExportOptions};

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

/// CSV delimiter options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvDelimiter {
    Comma,
    Tab,
    Semicolon,
}

impl CsvDelimiter {
    pub fn as_byte(&self) -> u8 {
        match self {
            CsvDelimiter::Comma => b',',
            CsvDelimiter::Tab => b'\t',
            CsvDelimiter::Semicolon => b';',
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CsvDelimiter::Comma => ",",
            CsvDelimiter::Tab => "\t",
            CsvDelimiter::Semicolon => ";",
        }
    }
}

impl Default for CsvDelimiter {
    fn default() -> Self {
        CsvDelimiter::Comma
    }
}

/// Common CSV export options
#[derive(Debug, Clone)]
pub struct CsvExportOptions {
    /// Delimiter character
    pub delimiter: CsvDelimiter,
    /// Include header row with column labels
    pub include_header: bool,
    /// Include metadata comments (prefixed with #)
    pub include_metadata: bool,
}

impl Default for CsvExportOptions {
    fn default() -> Self {
        Self {
            delimiter: CsvDelimiter::Comma,
            include_header: true,
            include_metadata: true,
        }
    }
}

/// Write metadata comment lines to a CSV file
///
/// Each metadata line is prefixed with # to mark it as a comment.
pub fn write_metadata_comments<W: Write>(
    writer: &mut W,
    metadata: &[(&str, &str)],
) -> io::Result<()> {
    for (key, value) in metadata {
        writeln!(writer, "# {}: {}", key, value)?;
    }
    if !metadata.is_empty() {
        writeln!(writer)?; // Blank line after metadata
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_delimiter_conversions() {
        assert_eq!(CsvDelimiter::Comma.as_byte(), b',');
        assert_eq!(CsvDelimiter::Tab.as_byte(), b'\t');
        assert_eq!(CsvDelimiter::Semicolon.as_byte(), b';');

        assert_eq!(CsvDelimiter::Comma.as_str(), ",");
        assert_eq!(CsvDelimiter::Tab.as_str(), "\t");
        assert_eq!(CsvDelimiter::Semicolon.as_str(), ";");
    }

    #[test]
    fn test_write_metadata_comments() {
        let mut buffer = Cursor::new(Vec::new());
        let metadata = vec![
            ("Exported by", "rust-daq GUI"),
            ("Timestamp", "2026-01-27T12:00:00Z"),
        ];

        write_metadata_comments(&mut buffer, &metadata).unwrap();

        let output = String::from_utf8(buffer.into_inner()).unwrap();
        assert!(output.contains("# Exported by: rust-daq GUI"));
        assert!(output.contains("# Timestamp: 2026-01-27T12:00:00Z"));
        assert!(output.ends_with("\n\n")); // Blank line after metadata
    }
}
