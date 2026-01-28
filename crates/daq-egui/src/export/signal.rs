//! Signal trace CSV export
//!
//! Exports signal traces from SignalPlotter to CSV format.
//! Supports multiple traces with aligned time points.

use super::{write_metadata_comments, CsvDelimiter, CsvExportOptions};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

/// Options specific to signal trace export
#[derive(Debug, Clone)]
pub struct SignalExportOptions {
    /// Common CSV options
    pub csv_options: CsvExportOptions,
    /// Include trace labels in filename
    pub include_labels_in_filename: bool,
}

impl Default for SignalExportOptions {
    fn default() -> Self {
        Self {
            csv_options: CsvExportOptions::default(),
            include_labels_in_filename: false,
        }
    }
}

/// A single signal trace to export
#[derive(Debug, Clone)]
pub struct SignalTraceData {
    pub label: String,
    pub device_id: String,
    pub observable_name: String,
    pub points: Vec<(f64, f64)>, // (time_offset, value)
}

impl SignalTraceData {
    /// Create from SignalPlotter's VecDeque format
    pub fn from_deque(
        label: String,
        device_id: String,
        observable_name: String,
        points: &VecDeque<(f64, f64)>,
    ) -> Self {
        Self {
            label,
            device_id,
            observable_name,
            points: points.iter().copied().collect(),
        }
    }
}

/// Export multiple signal traces to a CSV file
///
/// Traces are aligned by time and exported as columns:
/// Time, Trace1, Trace2, Trace3, ...
///
/// If traces have different time points, nearest-neighbor interpolation is used.
pub fn export_signal_traces<P: AsRef<Path>>(
    path: P,
    traces: &[SignalTraceData],
    options: &SignalExportOptions,
) -> io::Result<()> {
    if traces.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No traces to export",
        ));
    }

    let mut file = File::create(path.as_ref())?;
    let delimiter = options.csv_options.delimiter.as_str();

    // Write metadata
    if options.csv_options.include_metadata {
        let timestamp = chrono::Local::now().to_rfc3339();
        let mut metadata = vec![
            ("Exported by", "rust-daq GUI"),
            ("Export type", "Signal traces"),
            ("Timestamp", &timestamp),
            ("Trace count", &traces.len().to_string()),
        ];

        // Add device IDs
        for trace in traces {
            metadata.push((&format!("Device ({})", trace.label), &trace.device_id));
        }

        write_metadata_comments(&mut file, &metadata)?;
    }

    // Write header
    if options.csv_options.include_header {
        write!(file, "Time (s)")?;
        for trace in traces {
            write!(file, "{}{}", delimiter, trace.label)?;
        }
        writeln!(file)?;
    }

    // Collect all unique time points and sort
    let mut all_times = std::collections::BTreeSet::new();
    for trace in traces {
        for &(time, _) in &trace.points {
            // Round to 6 decimal places to avoid floating-point precision issues
            let rounded = (time * 1_000_000.0).round() / 1_000_000.0;
            all_times.insert(ordered_float::OrderedFloat(rounded));
        }
    }

    // Export data rows
    for time in all_times {
        let time = time.0;
        write!(file, "{:.6}", time)?;

        for trace in traces {
            // Find value at this time (nearest neighbor)
            let value = find_nearest_value(&trace.points, time);
            if let Some(v) = value {
                write!(file, "{}{:.6}", delimiter, v)?;
            } else {
                write!(file, "{}", delimiter)?; // Empty cell
            }
        }
        writeln!(file)?;
    }

    Ok(())
}

/// Find the value at the nearest time point
fn find_nearest_value(points: &[(f64, f64)], target_time: f64) -> Option<f64> {
    if points.is_empty() {
        return None;
    }

    // Binary search for nearest point
    let idx = points
        .binary_search_by(|&(t, _)| {
            t.partial_cmp(&target_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or_else(|i| i);

    // Check if exact match
    if idx < points.len() && (points[idx].0 - target_time).abs() < 1e-9 {
        return Some(points[idx].1);
    }

    // Find nearest neighbor
    if idx == 0 {
        Some(points[0].1)
    } else if idx >= points.len() {
        Some(points[points.len() - 1].1)
    } else {
        // Compare distances to idx-1 and idx
        let dist_prev = (points[idx - 1].0 - target_time).abs();
        let dist_next = (points[idx].0 - target_time).abs();
        if dist_prev < dist_next {
            Some(points[idx - 1].1)
        } else {
            Some(points[idx].1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::NamedTempFile;

    #[test]
    fn test_find_nearest_value() {
        let points = vec![(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)];

        assert_eq!(find_nearest_value(&points, 0.0), Some(1.0));
        assert_eq!(find_nearest_value(&points, 0.4), Some(1.0)); // Closer to 0.0
        assert_eq!(find_nearest_value(&points, 0.6), Some(2.0)); // Closer to 1.0
        assert_eq!(find_nearest_value(&points, 2.5), Some(3.0)); // Beyond range
        assert_eq!(find_nearest_value(&[], 1.0), None); // Empty
    }

    #[test]
    fn test_export_single_trace() {
        let trace = SignalTraceData {
            label: "Test Signal".to_string(),
            device_id: "device1".to_string(),
            observable_name: "power".to_string(),
            points: vec![(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)],
        };

        let temp = NamedTempFile::new().unwrap();
        let options = SignalExportOptions::default();

        export_signal_traces(temp.path(), &[trace], &options).unwrap();

        let content = std::fs::read_to_string(temp.path()).unwrap();

        // Check metadata
        assert!(content.contains("# Exported by: rust-daq GUI"));
        assert!(content.contains("# Trace count: 1"));

        // Check header
        assert!(content.contains("Time (s),Test Signal"));

        // Check data
        assert!(content.contains("0.000000,1.000000"));
        assert!(content.contains("1.000000,2.000000"));
        assert!(content.contains("2.000000,3.000000"));
    }

    #[test]
    fn test_export_multiple_traces_aligned() {
        let trace1 = SignalTraceData {
            label: "Signal A".to_string(),
            device_id: "dev1".to_string(),
            observable_name: "obs1".to_string(),
            points: vec![(0.0, 1.0), (1.0, 2.0)],
        };

        let trace2 = SignalTraceData {
            label: "Signal B".to_string(),
            device_id: "dev2".to_string(),
            observable_name: "obs2".to_string(),
            points: vec![(0.0, 10.0), (1.0, 20.0)],
        };

        let temp = NamedTempFile::new().unwrap();
        let options = SignalExportOptions::default();

        export_signal_traces(temp.path(), &[trace1, trace2], &options).unwrap();

        let content = std::fs::read_to_string(temp.path()).unwrap();

        // Check header
        assert!(content.contains("Time (s),Signal A,Signal B"));

        // Check data - traces should be aligned
        assert!(content.contains("0.000000,1.000000,10.000000"));
        assert!(content.contains("1.000000,2.000000,20.000000"));
    }

    #[test]
    fn test_export_with_tab_delimiter() {
        let trace = SignalTraceData {
            label: "Test".to_string(),
            device_id: "dev1".to_string(),
            observable_name: "obs1".to_string(),
            points: vec![(0.0, 1.0)],
        };

        let temp = NamedTempFile::new().unwrap();
        let mut options = SignalExportOptions::default();
        options.csv_options.delimiter = CsvDelimiter::Tab;

        export_signal_traces(temp.path(), &[trace], &options).unwrap();

        let content = std::fs::read_to_string(temp.path()).unwrap();

        // Check tab delimiter in header
        assert!(content.contains("Time (s)\tTest"));
    }

    #[test]
    fn test_export_without_header() {
        let trace = SignalTraceData {
            label: "Test".to_string(),
            device_id: "dev1".to_string(),
            observable_name: "obs1".to_string(),
            points: vec![(0.0, 1.0)],
        };

        let temp = NamedTempFile::new().unwrap();
        let mut options = SignalExportOptions::default();
        options.csv_options.include_header = false;

        export_signal_traces(temp.path(), &[trace], &options).unwrap();

        let content = std::fs::read_to_string(temp.path()).unwrap();

        // Should not contain header
        assert!(!content.contains("Time (s)"));
        // Should contain data
        assert!(content.contains("0.000000,1.000000"));
    }
}
