//! Zarr-backed [`DocumentSink`] that maps dimensional scan indices to chunk
//! coordinates.
//!
//! When the RunEngine emits [`EventDoc`]s with populated `scan_indices`
//! (e.g., `[("wavelength", 3), ("position", 12)]`), the sink uses those
//! indices directly as Zarr chunk coordinates and writes the
//! `_ARRAY_DIMENSIONS` attribute so that downstream tools (Xarray, napari)
//! understand the dimensional structure.
//!
//! If `scan_indices` is `None` the sink falls back to a sequential counter
//! so that 1-D scans still produce valid output without requiring explicit
//! index bookkeeping.
//!
//! # Example
//!
//! ```ignore
//! use storage::zarr_sink::ZarrSink;
//!
//! let mut sink = ZarrSink::new("/tmp/zarr_data");
//! sink.on_document(&Document::Event(event)).await?;
//! ```

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use common::experiment::document::{DataKey, Document, EventDoc};
use serde_json::json;
use tracing::{debug, trace, warn};

use crate::document_sink::DocumentSink;
use crate::zarr_writer::ZarrWriter;

/// Maximum size per scan dimension when the total extent is unknown.
///
/// Zarr V3 uses sparse chunk storage, so only chunks that are actually
/// written consume disk space. This upper bound is therefore safe for
/// any scan that emits fewer than 10 000 points per dimension.
const DEFAULT_SCAN_DIM_SIZE: u64 = 10_000;

/// Cached descriptor info used for deferred array creation.
struct CachedDescriptor {
    stream_name: String,
    data_keys: HashMap<String, DataKey>,
}

/// Tracks per-array state after creation.
struct ArrayState {
    /// Zarr data type string used for runtime dispatch on writes.
    dtype: String,
    /// Spatial shape from `DataKey.shape` (empty for scalars).
    spatial_shape: Vec<i32>,
}

/// A [`DocumentSink`] that writes event data to a Zarr V3 store.
///
/// Consumes `scan_indices` from [`EventDoc`] to determine chunk
/// coordinates, falling back to a sequential counter when indices are
/// absent.
pub struct ZarrSink {
    /// Base output directory for the Zarr store.
    base_path: PathBuf,
    /// Sequential event counter (fallback when no `scan_indices`).
    event_counter: usize,
    /// Current run UID (set on `StartDoc`, cleared on `StopDoc`).
    current_run: Option<String>,
    /// The underlying Zarr writer (created lazily on `StartDoc`).
    writer: Option<ZarrWriter>,
    /// Descriptor info cached for deferred array creation.
    /// Key: descriptor UID.
    descriptors: HashMap<String, CachedDescriptor>,
    /// Per-array state keyed by `"{stream}/{key}"`.
    arrays: HashMap<String, ArrayState>,
    /// Dimension names resolved from the first event (if any).
    scan_dimensions: Option<Vec<String>>,
}

impl ZarrSink {
    /// Create a new `ZarrSink` targeting the given directory.
    ///
    /// The directory is created lazily when the first `StartDoc` arrives.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
            event_counter: 0,
            current_run: None,
            writer: None,
            descriptors: HashMap::new(),
            arrays: HashMap::new(),
            scan_dimensions: None,
        }
    }

    /// Resolve chunk coordinates from an [`EventDoc`].
    ///
    /// Uses `scan_indices` when available, otherwise returns a single
    /// `("index", counter)` pair for 1-D fallback.
    fn resolve_coordinates(&mut self, event: &EventDoc) -> Vec<(String, usize)> {
        event
            .scan_indices
            .as_ref()
            .cloned()
            .unwrap_or_else(|| vec![("index".to_string(), self.event_counter)])
    }

    /// Extract dimension names from coordinates.
    ///
    /// These become the `_ARRAY_DIMENSIONS` attribute on the Zarr array,
    /// enabling automatic recognition by Xarray / napari.
    fn dimension_names(coords: &[(String, usize)]) -> Vec<String> {
        coords.iter().map(|(name, _)| name.clone()).collect()
    }

    /// Map a `DataKey.dtype` string to the appropriate Zarr array builder
    /// configuration and create the array via `ZarrWriter`.
    async fn create_zarr_array(
        writer: &ZarrWriter,
        array_name: &str,
        data_key: &DataKey,
        scan_dim_names: &[String],
    ) -> Result<()> {
        // Build the full shape: scan dimensions + spatial dimensions.
        let mut shape: Vec<u64> = scan_dim_names
            .iter()
            .map(|_| DEFAULT_SCAN_DIM_SIZE)
            .collect();
        let mut chunks: Vec<u64> = vec![1; scan_dim_names.len()];

        // Append spatial dimensions from DataKey.shape (empty for scalars).
        #[allow(clippy::cast_sign_loss)]
        for &s in &data_key.shape {
            let extent = s.max(1) as u64;
            shape.push(extent);
            chunks.push(extent);
        }

        // Build dimension name list: scan dims + spatial dims.
        let mut dim_names: Vec<String> = scan_dim_names.to_vec();
        if data_key.shape.len() == 2 {
            dim_names.push("y".to_string());
            dim_names.push("x".to_string());
        } else if data_key.shape.len() == 1 {
            dim_names.push("i".to_string());
        } else {
            for idx in 0..data_key.shape.len() {
                dim_names.push(format!("dim_{idx}"));
            }
        }

        let mut builder = writer.create_array().name(array_name);
        builder = builder.shape(shape).chunks(chunks).dimensions(dim_names);

        // Add source/units attributes.
        if !data_key.source.is_empty() {
            builder = builder.attribute("source", json!(data_key.source));
        }
        if !data_key.units.is_empty() {
            builder = builder.attribute("units", json!(data_key.units));
        }

        // Select dtype based on DataKey.dtype string.
        builder = match data_key.dtype.as_str() {
            "uint8" | "u8" => builder.dtype_u8(),
            "uint16" | "u16" => builder.dtype_u16(),
            "uint32" | "u32" => builder.dtype_u32(),
            "uint64" | "u64" => builder.dtype_u64(),
            "int8" | "i8" => builder.dtype_i8(),
            "int16" | "i16" => builder.dtype_i16(),
            "int32" | "i32" => builder.dtype_i32(),
            "int64" | "i64" => builder.dtype_i64(),
            "float32" | "f32" => builder.dtype_f32(),
            "float64" | "f64" => builder.dtype_f64(),
            "array" => builder.dtype_u8(),
            // Default: "number" or anything else -> f64.
            _ => builder.dtype_f64(),
        };

        builder
            .build()
            .await
            .with_context(|| format!("Failed to create Zarr array '{array_name}'"))?;

        Ok(())
    }

    /// Ensure all arrays for a descriptor are created.
    ///
    /// Called on the first event that references a given descriptor because
    /// only at that point do we know the scan dimension names.
    async fn ensure_arrays_created(
        &mut self,
        descriptor_uid: &str,
        scan_dim_names: &[String],
    ) -> Result<()> {
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| anyhow!("ZarrWriter not initialized (missing StartDoc?)"))?;

        let desc = self
            .descriptors
            .get(descriptor_uid)
            .ok_or_else(|| anyhow!("Unknown descriptor UID: {descriptor_uid}"))?;

        // Collect keys that need array creation.
        let mut to_create: Vec<(String, DataKey)> = Vec::new();
        for (key, data_key) in &desc.data_keys {
            let full_name = format!("{}/{key}", desc.stream_name);
            if !self.arrays.contains_key(&full_name) {
                to_create.push((full_name, data_key.clone()));
            }
        }

        // Also create a timestamp array for this stream.
        let ts_name = format!("{}/timestamps", desc.stream_name);
        if !self.arrays.contains_key(&ts_name) {
            let ts_key = DataKey {
                dtype: "float64".to_string(),
                shape: vec![],
                source: "system".to_string(),
                units: "s".to_string(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            };
            to_create.push((ts_name, ts_key));
        }

        for (full_name, data_key) in &to_create {
            Self::create_zarr_array(writer, full_name, data_key, scan_dim_names).await?;
            self.arrays.insert(
                full_name.clone(),
                ArrayState {
                    dtype: data_key.dtype.clone(),
                    spatial_shape: data_key.shape.clone(),
                },
            );
        }

        Ok(())
    }

    /// Write scalar data for a single event field.
    async fn write_scalar(
        writer: &ZarrWriter,
        array_name: &str,
        chunk_indices: &[u64],
        value: f64,
        dtype: &str,
    ) -> Result<()> {
        match dtype {
            "uint8" | "u8" => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let v = value as u8;
                writer
                    .write_chunk::<u8>(array_name, chunk_indices, vec![v])
                    .await
            }
            "uint16" | "u16" => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let v = value as u16;
                writer
                    .write_chunk::<u16>(array_name, chunk_indices, vec![v])
                    .await
            }
            "uint32" | "u32" => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let v = value as u32;
                writer
                    .write_chunk::<u32>(array_name, chunk_indices, vec![v])
                    .await
            }
            "uint64" | "u64" => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let v = value as u64;
                writer
                    .write_chunk::<u64>(array_name, chunk_indices, vec![v])
                    .await
            }
            "int8" | "i8" => {
                #[allow(clippy::cast_possible_truncation)]
                let v = value as i8;
                writer
                    .write_chunk::<i8>(array_name, chunk_indices, vec![v])
                    .await
            }
            "int16" | "i16" => {
                #[allow(clippy::cast_possible_truncation)]
                let v = value as i16;
                writer
                    .write_chunk::<i16>(array_name, chunk_indices, vec![v])
                    .await
            }
            "int32" | "i32" => {
                #[allow(clippy::cast_possible_truncation)]
                let v = value as i32;
                writer
                    .write_chunk::<i32>(array_name, chunk_indices, vec![v])
                    .await
            }
            "int64" | "i64" => {
                #[allow(clippy::cast_possible_truncation)]
                let v = value as i64;
                writer
                    .write_chunk::<i64>(array_name, chunk_indices, vec![v])
                    .await
            }
            "float32" | "f32" => {
                #[allow(clippy::cast_possible_truncation)]
                let v = value as f32;
                writer
                    .write_chunk::<f32>(array_name, chunk_indices, vec![v])
                    .await
            }
            // Default: f64 (covers "number", "float64", "f64", etc.)
            _ => {
                writer
                    .write_chunk::<f64>(array_name, chunk_indices, vec![value])
                    .await
            }
        }
        .with_context(|| format!("Failed to write scalar to '{array_name}'"))
    }

    /// Write array (binary) data for a single event field.
    async fn write_array_data(
        writer: &ZarrWriter,
        array_name: &str,
        chunk_indices: &[u64],
        bytes: &[u8],
        dtype: &str,
    ) -> Result<()> {
        match dtype {
            "uint8" | "u8" => {
                writer
                    .write_chunk::<u8>(array_name, chunk_indices, bytes.to_vec())
                    .await
            }
            "uint16" | "u16" => {
                if bytes.len() % 2 != 0 {
                    return Err(anyhow!(
                        "u16 array data has odd byte count ({})",
                        bytes.len()
                    ));
                }
                let data: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                writer
                    .write_chunk::<u16>(array_name, chunk_indices, data)
                    .await
            }
            "uint32" | "u32" => {
                if bytes.len() % 4 != 0 {
                    return Err(anyhow!(
                        "u32 array data has invalid byte count ({})",
                        bytes.len()
                    ));
                }
                let data: Vec<u32> = bytes
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                writer
                    .write_chunk::<u32>(array_name, chunk_indices, data)
                    .await
            }
            "uint64" | "u64" => {
                if bytes.len() % 8 != 0 {
                    return Err(anyhow!(
                        "u64 array data has invalid byte count ({})",
                        bytes.len()
                    ));
                }
                let data: Vec<u64> = bytes
                    .chunks_exact(8)
                    .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
                    .collect();
                writer
                    .write_chunk::<u64>(array_name, chunk_indices, data)
                    .await
            }
            "int8" | "i8" => {
                let data: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
                writer
                    .write_chunk::<i8>(array_name, chunk_indices, data)
                    .await
            }
            "int16" | "i16" => {
                if bytes.len() % 2 != 0 {
                    return Err(anyhow!(
                        "i16 array data has odd byte count ({})",
                        bytes.len()
                    ));
                }
                let data: Vec<i16> = bytes
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                writer
                    .write_chunk::<i16>(array_name, chunk_indices, data)
                    .await
            }
            "int32" | "i32" => {
                if bytes.len() % 4 != 0 {
                    return Err(anyhow!(
                        "i32 array data has invalid byte count ({})",
                        bytes.len()
                    ));
                }
                let data: Vec<i32> = bytes
                    .chunks_exact(4)
                    .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                writer
                    .write_chunk::<i32>(array_name, chunk_indices, data)
                    .await
            }
            "int64" | "i64" => {
                if bytes.len() % 8 != 0 {
                    return Err(anyhow!(
                        "i64 array data has invalid byte count ({})",
                        bytes.len()
                    ));
                }
                let data: Vec<i64> = bytes
                    .chunks_exact(8)
                    .map(|b| i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
                    .collect();
                writer
                    .write_chunk::<i64>(array_name, chunk_indices, data)
                    .await
            }
            "float32" | "f32" => {
                if bytes.len() % 4 != 0 {
                    return Err(anyhow!(
                        "f32 array data has invalid byte count ({})",
                        bytes.len()
                    ));
                }
                let data: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                writer
                    .write_chunk::<f32>(array_name, chunk_indices, data)
                    .await
            }
            "float64" | "f64" | "number" => {
                if bytes.len() % 8 != 0 {
                    return Err(anyhow!(
                        "f64 array data has invalid byte count ({})",
                        bytes.len()
                    ));
                }
                let data: Vec<f64> = bytes
                    .chunks_exact(8)
                    .map(|b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
                    .collect();
                writer
                    .write_chunk::<f64>(array_name, chunk_indices, data)
                    .await
            }
            // Default fallback: treat as raw u8 bytes.
            _ => {
                writer
                    .write_chunk::<u8>(array_name, chunk_indices, bytes.to_vec())
                    .await
            }
        }
        .with_context(|| format!("Failed to write array data to '{array_name}'"))
    }
}

#[async_trait]
impl DocumentSink for ZarrSink {
    fn name(&self) -> &'static str {
        "zarr"
    }

    async fn on_document(&mut self, doc: &Document) -> Result<()> {
        match doc {
            Document::Start(start) => {
                debug!(
                    run_uid = %start.uid,
                    plan = %start.plan_name,
                    "ZarrSink: new run started"
                );

                // Create the store directory for this run.
                let store_path = self.base_path.join(format!("{}.zarr", start.uid));
                let writer = ZarrWriter::new(&store_path).await.with_context(|| {
                    format!("Failed to create Zarr store at {}", store_path.display())
                })?;

                // Write run metadata as group attributes.
                writer
                    .add_group_attribute("run_uid", json!(start.uid))
                    .await?;
                writer
                    .add_group_attribute("plan_type", json!(start.plan_type))
                    .await?;
                writer
                    .add_group_attribute("plan_name", json!(start.plan_name))
                    .await?;
                if !start.plan_args.is_empty() {
                    writer
                        .add_group_attribute("plan_args", json!(start.plan_args))
                        .await?;
                }
                if !start.metadata.is_empty() {
                    writer
                        .add_group_attribute("metadata", json!(start.metadata))
                        .await?;
                }

                self.writer = Some(writer);
                self.current_run = Some(start.uid.clone());
                self.event_counter = 0;
                self.descriptors.clear();
                self.arrays.clear();
                self.scan_dimensions = None;
                Ok(())
            }
            Document::Descriptor(desc) => {
                trace!(
                    descriptor_uid = %desc.uid,
                    stream = %desc.name,
                    keys = ?desc.data_keys.keys().collect::<Vec<_>>(),
                    "ZarrSink: descriptor received — arrays will be created on first event"
                );
                // Cache descriptor info; actual array creation is deferred
                // until the first event so we know the scan dimension names.
                self.descriptors.insert(
                    desc.uid.clone(),
                    CachedDescriptor {
                        stream_name: desc.name.clone(),
                        data_keys: desc.data_keys.clone(),
                    },
                );
                Ok(())
            }
            Document::Event(event) => {
                let coords = self.resolve_coordinates(event);
                let dim_names = Self::dimension_names(&coords);

                trace!(
                    seq = event.seq_num,
                    coords = ?coords,
                    "ZarrSink: event -> chunk coordinates"
                );

                // Lazily capture scan dimension names from the first event.
                if self.scan_dimensions.is_none() {
                    self.scan_dimensions = Some(dim_names.clone());
                } else if let Some(expected_dims) = &self.scan_dimensions {
                    if expected_dims != &dim_names {
                        return Err(anyhow!(
                            "Inconsistent scan dimensions: initial {:?} but event {} uses {:?}",
                            expected_dims,
                            event.seq_num,
                            dim_names
                        ));
                    }
                }

                // Ensure arrays exist for this descriptor's data keys.
                self.ensure_arrays_created(&event.descriptor_uid, &dim_names)
                    .await?;

                // Build chunk indices from scan coordinates.
                #[allow(clippy::cast_possible_truncation)]
                let scan_chunk_indices: Vec<u64> =
                    coords.iter().map(|(_, idx)| *idx as u64).collect();

                // Look up descriptor to find stream name and data keys.
                let desc = self.descriptors.get(&event.descriptor_uid).ok_or_else(|| {
                    anyhow!(
                        "Event references unknown descriptor: {}",
                        event.descriptor_uid
                    )
                })?;
                let stream_name = desc.stream_name.clone();

                // --- Write scalar data ---
                let writer = self
                    .writer
                    .as_ref()
                    .ok_or_else(|| anyhow!("ZarrWriter not initialized"))?;

                for (key, &value) in &event.data {
                    let full_name = format!("{stream_name}/{key}");
                    let dtype = self
                        .arrays
                        .get(&full_name)
                        .map(|s| s.dtype.as_str())
                        .unwrap_or("number");
                    // For scalars, chunk indices == scan indices.
                    if let Err(e) =
                        Self::write_scalar(writer, &full_name, &scan_chunk_indices, value, dtype)
                            .await
                    {
                        warn!(key = %key, error = %e, "ZarrSink: failed to write scalar");
                    }
                }

                // --- Write array data ---
                for (key, bytes) in &event.arrays {
                    let full_name = format!("{stream_name}/{key}");
                    if let Some(state) = self.arrays.get(&full_name) {
                        // For arrays, chunk indices are [scan..., 0, 0, ...]
                        // because spatial dimensions use full-extent chunks.
                        let mut chunk_indices = scan_chunk_indices.clone();
                        for _ in &state.spatial_shape {
                            chunk_indices.push(0);
                        }
                        if let Err(e) = Self::write_array_data(
                            writer,
                            &full_name,
                            &chunk_indices,
                            bytes,
                            &state.dtype,
                        )
                        .await
                        {
                            warn!(key = %key, error = %e, "ZarrSink: failed to write array");
                        }
                    } else {
                        warn!(
                            key = %key,
                            "ZarrSink: no array state for key — skipping"
                        );
                    }
                }

                // --- Write timestamp ---
                #[allow(clippy::cast_precision_loss)]
                let ts_secs = event.time_ns as f64 / 1_000_000_000.0;
                let ts_name = format!("{stream_name}/timestamps");
                if self.arrays.contains_key(&ts_name) {
                    if let Err(e) =
                        Self::write_scalar(writer, &ts_name, &scan_chunk_indices, ts_secs, "f64")
                            .await
                    {
                        warn!(error = %e, "ZarrSink: failed to write timestamp");
                    }
                }

                self.event_counter += 1;
                Ok(())
            }
            Document::Stop(stop) => {
                debug!(
                    run_uid = %stop.run_uid,
                    status = %stop.exit_status,
                    events = stop.num_events,
                    "ZarrSink: run stopped"
                );

                // Write stop metadata as group attributes.
                if let Some(writer) = &self.writer {
                    writer
                        .add_group_attribute("exit_status", json!(stop.exit_status))
                        .await?;
                    writer
                        .add_group_attribute("num_events", json!(stop.num_events))
                        .await?;
                    if !stop.reason.is_empty() {
                        writer
                            .add_group_attribute("stop_reason", json!(stop.reason))
                            .await?;
                    }
                }

                self.current_run = None;
                self.writer = None;
                Ok(())
            }
            Document::Manifest(_) => {
                // Manifests are informational; no Zarr action needed.
                Ok(())
            }
        }
    }

    async fn flush(&mut self) -> Result<()> {
        // Zarr V3 writes are immediately persisted per-chunk, so flush is
        // a no-op for now. This will change if we add write-batching.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::experiment::document::{DataKey, DescriptorDoc, EventDoc, StartDoc, StopDoc};

    #[tokio::test]
    async fn test_zarr_sink_lifecycle() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut sink = ZarrSink::new(temp.path());

        assert_eq!(sink.name(), "zarr");

        // Start
        let start = Document::Start(StartDoc::new("grid_scan", "Test Grid"));
        let run_uid = match &start {
            Document::Start(s) => s.uid.clone(),
            _ => unreachable!(),
        };
        sink.on_document(&start).await.expect("start doc");
        assert_eq!(sink.current_run.as_deref(), Some(run_uid.as_str()));
        assert_eq!(sink.event_counter, 0);

        // Descriptor
        let desc = DescriptorDoc::new(&run_uid, "primary")
            .with_data_key("power", DataKey::scalar("power_meter", "W"));
        let desc_uid = desc.uid.clone();
        sink.on_document(&Document::Descriptor(desc))
            .await
            .expect("descriptor doc");

        // Event without scan_indices -- fallback to sequential
        let event = EventDoc::new(&run_uid, &desc_uid, 0).with_datum("power", 1.0);
        sink.on_document(&Document::Event(event))
            .await
            .expect("event doc");
        assert_eq!(sink.event_counter, 1);

        // Event with scan_indices
        let mut event2 = EventDoc::new(&run_uid, &desc_uid, 1).with_datum("power", 2.0);
        event2.scan_indices = Some(vec![
            ("wavelength".to_string(), 3),
            ("position".to_string(), 7),
        ]);
        sink.on_document(&Document::Event(event2))
            .await
            .expect("event2 doc");
        assert_eq!(sink.event_counter, 2);

        // Stop
        let stop = StopDoc::success(&run_uid, 2);
        sink.on_document(&Document::Stop(stop))
            .await
            .expect("stop doc");
        assert!(sink.current_run.is_none());

        // Flush is a no-op
        sink.flush().await.expect("flush");

        // Verify Zarr store was created on disk
        let store_path = temp.path().join(format!("{run_uid}.zarr"));
        assert!(
            store_path.exists(),
            "Zarr store directory should exist at {}",
            store_path.display()
        );
    }

    #[tokio::test]
    async fn test_scalar_data_written_to_zarr() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut sink = ZarrSink::new(temp.path());

        let start = StartDoc::new("count", "Scalar Test");
        let run_uid = start.uid.clone();
        sink.on_document(&Document::Start(start))
            .await
            .expect("start");

        let desc = DescriptorDoc::new(&run_uid, "primary")
            .with_data_key("intensity", DataKey::scalar("det1", "counts"));
        let desc_uid = desc.uid.clone();
        sink.on_document(&Document::Descriptor(desc))
            .await
            .expect("descriptor");

        // Write 3 events sequentially (use non-zero values to avoid
        // sparse-chunk elision when fill_value == 0.0).
        for i in 0..3u32 {
            #[allow(clippy::cast_lossless)]
            let event = EventDoc::new(&run_uid, &desc_uid, i)
                .with_datum("intensity", (i + 1) as f64 * 10.0);
            sink.on_document(&Document::Event(event))
                .await
                .expect("event");
        }

        sink.on_document(&Document::Stop(StopDoc::success(&run_uid, 3)))
            .await
            .expect("stop");

        // Verify chunk files exist (1-D array, each event = 1 chunk)
        let store_path = temp.path().join(format!("{run_uid}.zarr"));
        let array_dir = store_path.join("primary").join("intensity");
        assert!(
            array_dir.exists(),
            "Array directory should exist at {}",
            array_dir.display()
        );

        // Zarr V3: chunk files at c/0, c/1, c/2
        let chunk_dir = array_dir.join("c");
        assert!(chunk_dir.join("0").exists(), "Chunk 0 should exist");
        assert!(chunk_dir.join("1").exists(), "Chunk 1 should exist");
        assert!(chunk_dir.join("2").exists(), "Chunk 2 should exist");
    }

    #[tokio::test]
    async fn test_array_data_written_to_zarr() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut sink = ZarrSink::new(temp.path());

        let start = StartDoc::new("count", "Array Test");
        let run_uid = start.uid.clone();
        sink.on_document(&Document::Start(start))
            .await
            .expect("start");

        // Descriptor with a 4x4 u16 camera frame
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "cam1".to_string(),
            DataKey {
                source: "camera".to_string(),
                dtype: "uint16".to_string(),
                shape: vec![4, 4],
                units: String::new(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );
        let desc = DescriptorDoc {
            uid: "desc_1".to_string(),
            run_uid: run_uid.clone(),
            name: "primary".to_string(),
            data_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        sink.on_document(&Document::Descriptor(desc))
            .await
            .expect("descriptor");

        // Write an event with array data (4x4 u16 = 32 bytes LE)
        let frame: Vec<u16> = (0..16).collect();
        let mut frame_bytes = Vec::with_capacity(32);
        for val in &frame {
            frame_bytes.extend_from_slice(&val.to_le_bytes());
        }

        let event = EventDoc::new(&run_uid, "desc_1", 0)
            .with_array("cam1", bytes::Bytes::from(frame_bytes));
        sink.on_document(&Document::Event(event))
            .await
            .expect("event with array");

        sink.on_document(&Document::Stop(StopDoc::success(&run_uid, 1)))
            .await
            .expect("stop");

        // Verify array directory and chunk exist
        let store_path = temp.path().join(format!("{run_uid}.zarr"));
        let array_dir = store_path.join("primary/cam1");
        assert!(array_dir.exists(), "Camera array should exist");

        // Chunk at [0, 0, 0] (scan_idx=0, spatial chunk=[0,0])
        let chunk_path = array_dir.join("c").join("0").join("0").join("0");
        assert!(
            chunk_path.exists(),
            "Frame chunk should exist at {}",
            chunk_path.display()
        );
    }

    #[tokio::test]
    async fn test_scan_indices_become_chunk_coords() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut sink = ZarrSink::new(temp.path());

        let start = StartDoc::new("grid_scan", "2D Grid");
        let run_uid = start.uid.clone();
        sink.on_document(&Document::Start(start))
            .await
            .expect("start");

        let desc = DescriptorDoc::new(&run_uid, "primary")
            .with_data_key("signal", DataKey::scalar("det1", "V"));
        let desc_uid = desc.uid.clone();
        sink.on_document(&Document::Descriptor(desc))
            .await
            .expect("descriptor");

        // Write event at (outer=2, inner=5)
        let mut event = EventDoc::new(&run_uid, &desc_uid, 0).with_datum("signal", 42.0);
        event.scan_indices = Some(vec![("outer".to_string(), 2), ("inner".to_string(), 5)]);
        sink.on_document(&Document::Event(event))
            .await
            .expect("event");

        sink.on_document(&Document::Stop(StopDoc::success(&run_uid, 1)))
            .await
            .expect("stop");

        // Verify chunk at [2, 5]
        let store_path = temp.path().join(format!("{run_uid}.zarr"));
        let chunk_path = store_path
            .join("primary/signal")
            .join("c")
            .join("2")
            .join("5");
        assert!(
            chunk_path.exists(),
            "Chunk at scan indices [2,5] should exist at {}",
            chunk_path.display()
        );
    }

    #[tokio::test]
    async fn test_zarr_metadata_attributes() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut sink = ZarrSink::new(temp.path());

        let start = StartDoc::new("count", "Metadata Test")
            .with_metadata("operator", "Alice")
            .with_arg("exposure", "100ms");
        let run_uid = start.uid.clone();
        sink.on_document(&Document::Start(start))
            .await
            .expect("start");

        let stop = StopDoc::success(&run_uid, 0);
        sink.on_document(&Document::Stop(stop)).await.expect("stop");

        // Verify root group metadata
        let store_path = temp.path().join(format!("{run_uid}.zarr"));
        let root_json_path = store_path.join("zarr.json");
        assert!(root_json_path.exists(), "Root zarr.json should exist");

        let content = std::fs::read_to_string(&root_json_path).expect("read zarr.json");
        let metadata: serde_json::Value = serde_json::from_str(&content).expect("parse zarr.json");
        let attrs = metadata
            .get("attributes")
            .expect("attributes should exist in root group");

        assert_eq!(
            attrs.get("run_uid"),
            Some(&json!(run_uid)),
            "run_uid attribute"
        );
        assert_eq!(
            attrs.get("plan_name"),
            Some(&json!("Metadata Test")),
            "plan_name attribute"
        );
        assert_eq!(
            attrs.get("exit_status"),
            Some(&json!("success")),
            "exit_status attribute"
        );
    }

    #[tokio::test]
    async fn test_resolve_coordinates_with_scan_indices() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut sink = ZarrSink::new(temp.path());

        let mut event = EventDoc::new("run1", "desc1", 0);
        event.scan_indices = Some(vec![("outer".to_string(), 2), ("inner".to_string(), 5)]);

        let coords = sink.resolve_coordinates(&event);
        assert_eq!(
            coords,
            vec![("outer".to_string(), 2), ("inner".to_string(), 5)]
        );
    }

    #[tokio::test]
    async fn test_resolve_coordinates_fallback() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut sink = ZarrSink::new(temp.path());
        sink.event_counter = 42;

        let event = EventDoc::new("run1", "desc1", 0);
        assert!(event.scan_indices.is_none());

        let coords = sink.resolve_coordinates(&event);
        assert_eq!(coords, vec![("index".to_string(), 42)]);
    }

    #[test]
    fn test_dimension_names() {
        let coords = vec![
            ("wavelength".to_string(), 0),
            ("position".to_string(), 1),
            ("y".to_string(), 0),
            ("x".to_string(), 0),
        ];
        let names = ZarrSink::dimension_names(&coords);
        assert_eq!(names, vec!["wavelength", "position", "y", "x"]);
    }

    #[tokio::test]
    async fn test_xarray_dimensions_attribute() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut sink = ZarrSink::new(temp.path());

        let start = StartDoc::new("count", "Dims Test");
        let run_uid = start.uid.clone();
        sink.on_document(&Document::Start(start))
            .await
            .expect("start");

        let desc = DescriptorDoc::new(&run_uid, "primary")
            .with_data_key("power", DataKey::scalar("pm", "W"));
        let desc_uid = desc.uid.clone();
        sink.on_document(&Document::Descriptor(desc))
            .await
            .expect("descriptor");

        let event = EventDoc::new(&run_uid, &desc_uid, 0).with_datum("power", 1.0);
        sink.on_document(&Document::Event(event))
            .await
            .expect("event");

        sink.on_document(&Document::Stop(StopDoc::success(&run_uid, 1)))
            .await
            .expect("stop");

        // Read the array zarr.json and check _ARRAY_DIMENSIONS
        let store_path = temp.path().join(format!("{run_uid}.zarr"));
        let array_json_path = store_path.join("primary/power/zarr.json");
        assert!(
            array_json_path.exists(),
            "Array zarr.json should exist at {}",
            array_json_path.display()
        );

        let content = std::fs::read_to_string(&array_json_path).expect("read");
        let metadata: serde_json::Value = serde_json::from_str(&content).expect("parse");
        let attrs = metadata.get("attributes").expect("attributes");
        let dims = attrs
            .get("_ARRAY_DIMENSIONS")
            .expect("_ARRAY_DIMENSIONS should be set");
        assert_eq!(
            dims,
            &json!(["index"]),
            "1-D fallback should produce [\"index\"]"
        );
    }
}
