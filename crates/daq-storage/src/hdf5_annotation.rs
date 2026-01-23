//! HDF5 Annotation - Post-acquisition metadata editing
//!
//! Provides utilities for adding user notes and tags to completed runs
//! by modifying HDF5 attributes on the start group.

use anyhow::{Context, Result};
use std::path::Path;

#[cfg(feature = "storage_hdf5")]
use hdf5::{types::VarLenUnicode, File};

/// Annotation data for a run
#[derive(Debug, Clone)]
pub struct RunAnnotation {
    pub notes: String,
    pub tags: Vec<String>,
}

/// Add or update user annotation on an existing HDF5 file
///
/// This writes `user_notes` and `tags` attributes to the `/start` group.
/// If attributes already exist, they are overwritten.
#[cfg(feature = "storage_hdf5")]
pub fn add_run_annotation(file_path: &Path, annotation: &RunAnnotation) -> Result<()> {
    let file = File::open_rw(file_path).context("Failed to open HDF5 file for annotation")?;

    let start_group = file
        .group("start")
        .context("HDF5 file missing /start group")?;

    // Delete existing attributes if present (HDF5 doesn't support overwrite)
    let _ = start_group.delete_attr("user_notes");
    let _ = start_group.delete_attr("tags");
    let _ = start_group.delete_attr("annotated_at_ns");

    // Write user notes
    if !annotation.notes.is_empty() {
        start_group
            .new_attr::<VarLenUnicode>()
            .create("user_notes")?
            .write_scalar(&annotation.notes.parse::<VarLenUnicode>()?)?;
    }

    // Write tags as JSON array
    if !annotation.tags.is_empty() {
        let tags_json = serde_json::to_string(&annotation.tags)?;
        start_group
            .new_attr::<VarLenUnicode>()
            .create("tags")?
            .write_scalar(&tags_json.parse::<VarLenUnicode>()?)?;
    }

    // Timestamp the annotation
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    start_group
        .new_attr::<u64>()
        .create("annotated_at_ns")?
        .write_scalar(&now_ns)?;

    Ok(())
}

/// Read existing annotations from an HDF5 file
///
/// Returns None if no annotations exist, Some(RunAnnotation) if found.
#[cfg(feature = "storage_hdf5")]
pub fn read_run_annotations(file_path: &Path) -> Result<Option<RunAnnotation>> {
    let file = File::open(file_path).context("Failed to open HDF5 file")?;

    let start_group = file
        .group("start")
        .context("HDF5 file missing /start group")?;

    let notes = start_group
        .attr("user_notes")
        .ok()
        .and_then(|attr| attr.read_scalar::<VarLenUnicode>().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();

    let tags = start_group
        .attr("tags")
        .ok()
        .and_then(|attr| attr.read_scalar::<VarLenUnicode>().ok())
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default();

    if notes.is_empty() && tags.is_empty() {
        Ok(None)
    } else {
        Ok(Some(RunAnnotation { notes, tags }))
    }
}

// Mock implementations for non-HDF5 builds
#[cfg(not(feature = "storage_hdf5"))]
pub fn add_run_annotation(_file_path: &Path, _annotation: &RunAnnotation) -> Result<()> {
    anyhow::bail!("HDF5 storage feature not enabled")
}

#[cfg(not(feature = "storage_hdf5"))]
pub fn read_run_annotations(_file_path: &Path) -> Result<Option<RunAnnotation>> {
    Ok(None)
}
