//! HDF5 Data Recovery Tool
//!
//! Recovers data from partially-written or corrupt HDF5 files after power loss.
//!
//! After an unexpected power loss during acquisition, HDF5 files may be left
//! in an inconsistent state:
//! - Incomplete chunks (partially written datasets)
//! - Inconsistent metadata (B-tree corruption)
//! - Partially written groups
//!
//! This module attempts to open the corrupt file, enumerate all readable
//! datasets and groups, and copy the recoverable data into a new clean file.
//!
//! # Usage
//!
//! ```no_run
//! use std::path::Path;
//! use storage::hdf5_recovery::recover_hdf5;
//!
//! let report = recover_hdf5(
//!     Path::new("corrupt_experiment.h5"),
//!     Path::new("recovered_experiment.h5"),
//! ).expect("recovery failed");
//!
//! println!("Recovered {}/{} datasets", report.recovered_datasets, report.total_datasets);
//! ```

use std::path::{Path, PathBuf};

#[cfg(feature = "storage_hdf5")]
use anyhow::Context as _;
#[cfg(feature = "storage_hdf5")]
use hdf5::File;
#[cfg(feature = "storage_hdf5")]
use tracing::{info, warn};

/// Summary report produced by the recovery process.
#[derive(Debug, Clone)]
pub struct RecoveryReport {
    /// Path to the input (corrupt) file.
    pub file_path: PathBuf,
    /// Total number of datasets found in the corrupt file.
    pub total_datasets: usize,
    /// Number of datasets successfully recovered.
    pub recovered_datasets: usize,
    /// Number of groups found in the corrupt file.
    pub total_groups: usize,
    /// Number of groups successfully recovered.
    pub recovered_groups: usize,
    /// Number of attributes recovered.
    pub recovered_attributes: usize,
    /// Number of datasets that could not be read (lost frames).
    pub lost_datasets: usize,
    /// Per-dataset details for datasets that failed recovery.
    pub errors: Vec<RecoveryError>,
    /// Path to the output (recovered) file.
    pub output_path: PathBuf,
}

/// Details about a single dataset or group that failed recovery.
#[derive(Debug, Clone)]
pub struct RecoveryError {
    /// HDF5 path of the object that failed (e.g. "/measurements/batch_000001/timestamps").
    pub path: String,
    /// Human-readable description of the failure.
    pub reason: String,
}

impl std::fmt::Display for RecoveryReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "HDF5 Recovery Report")?;
        writeln!(f, "  Input:  {}", self.file_path.display())?;
        writeln!(f, "  Output: {}", self.output_path.display())?;
        writeln!(
            f,
            "  Groups:     {}/{}",
            self.recovered_groups, self.total_groups
        )?;
        writeln!(
            f,
            "  Datasets:   {}/{}",
            self.recovered_datasets, self.total_datasets
        )?;
        writeln!(f, "  Attributes: {}", self.recovered_attributes)?;
        writeln!(f, "  Lost:       {}", self.lost_datasets)?;
        if !self.errors.is_empty() {
            writeln!(f, "  Errors:")?;
            for err in &self.errors {
                writeln!(f, "    - {}: {}", err.path, err.reason)?;
            }
        }
        Ok(())
    }
}

/// Attempt to recover data from a corrupt HDF5 file.
///
/// Opens the input file in read-only mode, enumerates all groups and datasets,
/// and copies every readable dataset into a new clean output file. Groups and
/// attributes are recreated to preserve the file hierarchy.
///
/// # Arguments
///
/// * `input` - Path to the corrupt HDF5 file
/// * `output` - Path where the recovered file will be written (must not exist)
///
/// # Errors
///
/// Returns an error if:
/// - The input file cannot be opened at all (completely unreadable)
/// - The output path already exists
/// - The output file cannot be created
#[cfg(feature = "storage_hdf5")]
pub fn recover_hdf5(input: &Path, output: &Path) -> anyhow::Result<RecoveryReport> {
    if output.exists() {
        anyhow::bail!(
            "Output file already exists: {}. Remove it first or choose a different path.",
            output.display()
        );
    }

    info!(input = %input.display(), output = %output.display(), "Starting HDF5 recovery");

    // Try to open the corrupt file read-only
    let src = File::open(input).context(format!(
        "Cannot open corrupt HDF5 file: {}. The file may be too damaged for recovery.",
        input.display()
    ))?;

    // Create the output file
    let dst = File::create(output).context(format!(
        "Cannot create output HDF5 file: {}",
        output.display()
    ))?;

    let mut report = RecoveryReport {
        file_path: input.to_path_buf(),
        total_datasets: 0,
        recovered_datasets: 0,
        total_groups: 0,
        recovered_groups: 0,
        recovered_attributes: 0,
        lost_datasets: 0,
        errors: Vec::new(),
        output_path: output.to_path_buf(),
    };

    // Recover root-level attributes first
    recover_attributes(&src, &dst, "/", &mut report);

    // Recursively walk the file hierarchy
    recover_group_recursive(&src, &dst, "/", &mut report);

    report.lost_datasets = report
        .total_datasets
        .saturating_sub(report.recovered_datasets);

    info!(
        recovered_datasets = report.recovered_datasets,
        total_datasets = report.total_datasets,
        lost = report.lost_datasets,
        "HDF5 recovery complete"
    );

    Ok(report)
}

/// Recursively recover groups and their contents.
#[cfg(feature = "storage_hdf5")]
fn recover_group_recursive(
    src_container: &hdf5::Group,
    dst_container: &hdf5::Group,
    parent_path: &str,
    report: &mut RecoveryReport,
) {
    // Try to list child groups
    let groups = match src_container.groups() {
        Ok(groups) => groups,
        Err(e) => {
            warn!(path = parent_path, error = %e, "Cannot enumerate groups");
            report.errors.push(RecoveryError {
                path: parent_path.to_string(),
                reason: format!("Cannot enumerate child groups: {e}"),
            });
            return;
        }
    };

    for src_group in &groups {
        let name = match src_group.name() {
            name if name.starts_with(parent_path) => {
                // Extract the last component from the full HDF5 path
                let relative = name.trim_start_matches(parent_path).trim_start_matches('/');
                // Only take the immediate child name (not nested)
                if let Some(slash_pos) = relative.find('/') {
                    relative[..slash_pos].to_string()
                } else {
                    relative.to_string()
                }
            }
            name => name.clone(),
        };

        if name.is_empty() {
            continue;
        }

        let full_path = if parent_path == "/" {
            format!("/{name}")
        } else {
            format!("{parent_path}/{name}")
        };

        report.total_groups += 1;

        // Create the group in the destination
        let dst_group = match dst_container.create_group(&name) {
            Ok(g) => g,
            Err(e) => {
                warn!(path = %full_path, error = %e, "Cannot create group in output");
                report.errors.push(RecoveryError {
                    path: full_path.clone(),
                    reason: format!("Cannot create group in output: {e}"),
                });
                continue;
            }
        };

        report.recovered_groups += 1;

        // Recover attributes on this group
        recover_attributes(src_group, &dst_group, &full_path, report);

        // Recover datasets in this group
        recover_datasets(src_group, &dst_group, &full_path, report);

        // Recurse into child groups
        recover_group_recursive(src_group, &dst_group, &full_path, report);
    }

    // Also recover datasets at this level (important for root-level datasets)
    if parent_path == "/" {
        recover_datasets(src_container, dst_container, parent_path, report);
    }
}

/// Recover all datasets within a single group.
#[cfg(feature = "storage_hdf5")]
fn recover_datasets(
    src_group: &hdf5::Group,
    dst_group: &hdf5::Group,
    parent_path: &str,
    report: &mut RecoveryReport,
) {
    let datasets = match src_group.datasets() {
        Ok(ds) => ds,
        Err(e) => {
            warn!(path = parent_path, error = %e, "Cannot enumerate datasets");
            report.errors.push(RecoveryError {
                path: parent_path.to_string(),
                reason: format!("Cannot enumerate datasets: {e}"),
            });
            return;
        }
    };

    for src_ds in &datasets {
        let full_name = src_ds.name();
        let ds_name = full_name.rsplit('/').next().unwrap_or(&full_name);

        let ds_path = if parent_path == "/" {
            format!("/{ds_name}")
        } else {
            format!("{parent_path}/{ds_name}")
        };

        report.total_datasets += 1;

        if let Err(reason) = recover_single_dataset(src_ds, dst_group, ds_name) {
            warn!(path = %ds_path, error = %reason, "Dataset recovery failed");
            report.errors.push(RecoveryError {
                path: ds_path,
                reason,
            });
        } else {
            report.recovered_datasets += 1;
            info!(path = %ds_path, "Dataset recovered");
        }
    }
}

/// Recover a single dataset by reading its raw bytes and writing to the destination.
///
/// Uses type-erased reads: we read the dataset as raw bytes and write it to a
/// new dataset with the same shape and type. This handles f64, u64, u32, u16,
/// u8, i64, i32, and i16 — the types used by the DAQ writers.
#[cfg(feature = "storage_hdf5")]
fn recover_single_dataset(
    src_ds: &hdf5::Dataset,
    dst_group: &hdf5::Group,
    name: &str,
) -> std::result::Result<(), String> {
    // Get shape and type info
    let shape = src_ds.shape();
    let dtype = src_ds
        .dtype()
        .map_err(|e| format!("Cannot read dtype: {e}"))?;

    // Attempt to read and write based on the underlying type
    // The hdf5-metno crate uses TypeDescriptor for type introspection
    let type_desc = dtype
        .to_descriptor()
        .map_err(|e| format!("Cannot get type descriptor: {e}"))?;

    match type_desc {
        hdf5::types::TypeDescriptor::Float(hdf5::types::FloatSize::U8) => {
            copy_dataset::<f64>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Float(hdf5::types::FloatSize::U4) => {
            copy_dataset::<f32>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Unsigned(hdf5::types::IntSize::U8) => {
            copy_dataset::<u64>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Unsigned(hdf5::types::IntSize::U4) => {
            copy_dataset::<u32>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Unsigned(hdf5::types::IntSize::U2) => {
            copy_dataset::<u16>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Unsigned(hdf5::types::IntSize::U1) => {
            copy_dataset::<u8>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Integer(hdf5::types::IntSize::U8) => {
            copy_dataset::<i64>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Integer(hdf5::types::IntSize::U4) => {
            copy_dataset::<i32>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Integer(hdf5::types::IntSize::U2) => {
            copy_dataset::<i16>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::Integer(hdf5::types::IntSize::U1) => {
            copy_dataset::<i8>(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::VarLenUnicode => {
            copy_varlen_string_dataset(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::FixedUnicode(_) => {
            copy_varlen_string_dataset(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::VarLenAscii => {
            copy_varlen_string_dataset(src_ds, dst_group, name, &shape)
        }
        hdf5::types::TypeDescriptor::FixedAscii(_) => {
            copy_varlen_string_dataset(src_ds, dst_group, name, &shape)
        }
        other => {
            // Fallback: try reading as raw u8 bytes
            warn!(dtype = ?other, "Unsupported dtype, attempting raw byte copy");
            copy_dataset::<u8>(src_ds, dst_group, name, &shape)
        }
    }
}

/// Generic typed copy: read dataset as Vec<T>, write to destination.
#[cfg(feature = "storage_hdf5")]
fn copy_dataset<T>(
    src_ds: &hdf5::Dataset,
    dst_group: &hdf5::Group,
    name: &str,
    shape: &[usize],
) -> std::result::Result<(), String>
where
    T: hdf5::H5Type + Clone,
{
    let data: Vec<T> = src_ds
        .read_raw()
        .map_err(|e| format!("Cannot read data: {e}"))?;

    let dst_ds = dst_group
        .new_dataset::<T>()
        .shape(shape)
        .create(name)
        .map_err(|e| format!("Cannot create dataset: {e}"))?;

    dst_ds
        .write_raw(&data)
        .map_err(|e| format!("Cannot write data: {e}"))?;

    // Copy dataset attributes
    copy_object_attributes(src_ds, &dst_ds);

    Ok(())
}

/// Copy a VarLenUnicode string dataset.
#[cfg(feature = "storage_hdf5")]
fn copy_varlen_string_dataset(
    src_ds: &hdf5::Dataset,
    dst_group: &hdf5::Group,
    name: &str,
    shape: &[usize],
) -> std::result::Result<(), String> {
    use hdf5::types::VarLenUnicode;

    let data: Vec<VarLenUnicode> = src_ds
        .read_raw()
        .map_err(|e| format!("Cannot read string data: {e}"))?;

    let dst_ds = dst_group
        .new_dataset::<VarLenUnicode>()
        .shape(shape)
        .create(name)
        .map_err(|e| format!("Cannot create string dataset: {e}"))?;

    dst_ds
        .write_raw(&data)
        .map_err(|e| format!("Cannot write string data: {e}"))?;

    copy_object_attributes(src_ds, &dst_ds);

    Ok(())
}

/// Best-effort copy of all attributes from one HDF5 object to another.
///
/// Logs warnings on failure but does not propagate errors — attribute loss is
/// acceptable as long as the core data is preserved.
///
/// Both `Group` and `Dataset` deref to `Location`, so callers can pass either.
#[cfg(feature = "storage_hdf5")]
fn copy_object_attributes(src: &hdf5::Location, dst: &hdf5::Location) {
    let attr_names = match src.attr_names() {
        Ok(names) => names,
        Err(_) => return,
    };

    for attr_name in &attr_names {
        if let Ok(src_attr) = src.attr(attr_name) {
            // Try common attribute types
            if try_copy_attr_scalar::<f64>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_scalar::<u64>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_scalar::<i64>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_scalar::<u32>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_scalar::<i32>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_string(&src_attr, dst, attr_name).is_ok()
            {
                // Attribute copied successfully
            } else {
                warn!(
                    attr = attr_name,
                    "Could not copy attribute (unsupported type)"
                );
            }
        }
    }
}

/// Try copying a scalar attribute of a specific type.
#[cfg(feature = "storage_hdf5")]
fn try_copy_attr_scalar<T: hdf5::H5Type>(
    src_attr: &hdf5::Attribute,
    dst: &hdf5::Location,
    name: &str,
) -> std::result::Result<(), ()> {
    let value: T = src_attr.read_scalar().map_err(|_| ())?;
    dst.new_attr::<T>()
        .create(name)
        .map_err(|_| ())?
        .write_scalar(&value)
        .map_err(|_| ())?;
    Ok(())
}

/// Try copying a string attribute.
#[cfg(feature = "storage_hdf5")]
fn try_copy_attr_string(
    src_attr: &hdf5::Attribute,
    dst: &hdf5::Location,
    name: &str,
) -> std::result::Result<(), ()> {
    use hdf5::types::VarLenUnicode;

    let value: VarLenUnicode = src_attr.read_scalar().map_err(|_| ())?;
    dst.new_attr::<VarLenUnicode>()
        .create(name)
        .map_err(|_| ())?
        .write_scalar(&value)
        .map_err(|_| ())?;
    Ok(())
}

/// Recover attributes on a container (group or file root).
#[cfg(feature = "storage_hdf5")]
fn recover_attributes(
    src: &hdf5::Group,
    dst: &hdf5::Group,
    path: &str,
    report: &mut RecoveryReport,
) {
    let attr_names = match src.attr_names() {
        Ok(names) => names,
        Err(e) => {
            warn!(path = path, error = %e, "Cannot enumerate attributes");
            return;
        }
    };

    for attr_name in &attr_names {
        if let Ok(src_attr) = src.attr(attr_name) {
            if try_copy_attr_scalar::<f64>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_scalar::<u64>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_scalar::<i64>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_scalar::<u32>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_scalar::<i32>(&src_attr, dst, attr_name).is_ok()
                || try_copy_attr_string(&src_attr, dst, attr_name).is_ok()
            {
                report.recovered_attributes += 1;
            } else {
                warn!(path = path, attr = attr_name, "Could not copy attribute");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Non-HDF5 stub
// ---------------------------------------------------------------------------

/// Stub for builds without `storage_hdf5` feature.
#[cfg(not(feature = "storage_hdf5"))]
pub fn recover_hdf5(_input: &Path, output: &Path) -> anyhow::Result<RecoveryReport> {
    if output.exists() {
        anyhow::bail!(
            "Output file already exists: {}. Remove it first or choose a different path.",
            output.display()
        );
    }
    anyhow::bail!("HDF5 storage feature not enabled. Rebuild with --features storage_hdf5")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recovery_report_display() {
        let report = RecoveryReport {
            file_path: PathBuf::from("corrupt.h5"),
            total_datasets: 10,
            recovered_datasets: 8,
            total_groups: 3,
            recovered_groups: 3,
            recovered_attributes: 12,
            lost_datasets: 2,
            errors: vec![RecoveryError {
                path: "/measurements/batch_000009/timestamps".to_string(),
                reason: "Cannot read data: truncated chunk".to_string(),
            }],
            output_path: PathBuf::from("recovered.h5"),
        };

        let display = format!("{report}");
        assert!(display.contains("HDF5 Recovery Report"));
        assert!(display.contains("8/10"));
        assert!(display.contains("truncated chunk"));
    }

    #[test]
    fn test_output_exists_rejected() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let existing = dir.path().join("exists.h5");
        std::fs::write(&existing, b"dummy").expect("write");

        let result = recover_hdf5(&dir.path().join("nonexistent.h5"), &existing);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("already exists"), "got: {err_msg}");
    }

    #[cfg(feature = "storage_hdf5")]
    #[test]
    fn test_recover_valid_hdf5_file() {
        use hdf5::File;

        let dir = tempfile::TempDir::new().expect("tempdir");
        let src_path = dir.path().join("source.h5");
        let dst_path = dir.path().join("recovered.h5");

        // Create a valid HDF5 file with known content
        {
            let file = File::create(&src_path).expect("create src");
            let group = file.create_group("measurements").expect("create group");
            let batch = group.create_group("batch_000000").expect("create batch");

            // Write datasets matching our DAQ writer patterns
            let timestamps: Vec<u64> = vec![1_000_000, 2_000_000, 3_000_000];
            batch
                .new_dataset::<u64>()
                .shape(timestamps.len())
                .create("timestamps")
                .expect("create ds")
                .write(&timestamps)
                .expect("write");

            let voltages: Vec<f64> = vec![1.1, 2.2, 3.3];
            batch
                .new_dataset::<f64>()
                .shape(voltages.len())
                .create("voltages")
                .expect("create ds")
                .write(&voltages)
                .expect("write");

            // Add an attribute
            batch
                .new_attr::<u64>()
                .create("ring_tail")
                .expect("create attr")
                .write_scalar(&42u64)
                .expect("write attr");
        }

        // Recover
        let report = recover_hdf5(&src_path, &dst_path).expect("recovery");

        assert_eq!(report.total_datasets, 2);
        assert_eq!(report.recovered_datasets, 2);
        assert_eq!(report.lost_datasets, 0);
        assert_eq!(report.total_groups, 2); // measurements + batch_000000
        assert_eq!(report.recovered_groups, 2);
        assert!(report.errors.is_empty());

        // Verify recovered content
        let recovered = File::open(&dst_path).expect("open recovered");
        let batch = recovered
            .group("measurements")
            .expect("measurements")
            .group("batch_000000")
            .expect("batch");

        let timestamps: Vec<u64> = batch
            .dataset("timestamps")
            .expect("timestamps ds")
            .read_raw()
            .expect("read");
        assert_eq!(timestamps, vec![1_000_000, 2_000_000, 3_000_000]);

        let voltages: Vec<f64> = batch
            .dataset("voltages")
            .expect("voltages ds")
            .read_raw()
            .expect("read");
        assert_eq!(voltages, vec![1.1, 2.2, 3.3]);
    }

    #[cfg(feature = "storage_hdf5")]
    #[test]
    fn test_recover_file_with_parameters_group() {
        use hdf5::types::VarLenUnicode;
        use hdf5::File;

        let dir = tempfile::TempDir::new().expect("tempdir");
        let src_path = dir.path().join("with_params.h5");
        let dst_path = dir.path().join("recovered_params.h5");

        // Create a file mimicking inject_parameters output
        {
            let file = File::create(&src_path).expect("create");
            let params = file.create_group("parameters").expect("params");

            let json_str = r#"{"name":"exposure_time","value":0.1,"units":"s"}"#;
            params
                .new_attr::<VarLenUnicode>()
                .create("exposure_time")
                .expect("create attr")
                .write_scalar(&json_str.parse::<VarLenUnicode>().expect("parse"))
                .expect("write");
        }

        let report = recover_hdf5(&src_path, &dst_path).expect("recovery");
        assert_eq!(report.recovered_groups, 1);
        assert!(report.recovered_attributes > 0);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn test_recover_nonexistent_input() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let result = recover_hdf5(
            &dir.path().join("does_not_exist.h5"),
            &dir.path().join("output.h5"),
        );
        assert!(result.is_err());
    }
}
