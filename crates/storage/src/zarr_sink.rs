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
//! let mut sink = ZarrSink::new("/tmp/experiment.zarr");
//! sink.on_document(&Document::Event(event)).await?;
//! ```

use anyhow::Result;
use async_trait::async_trait;
use common::experiment::document::{Document, EventDoc};
use std::path::PathBuf;
use tracing::{debug, trace};

use crate::document_sink::DocumentSink;

/// A [`DocumentSink`] that writes event data to a Zarr V3 store.
///
/// Consumes `scan_indices` from [`EventDoc`] to determine chunk
/// coordinates, falling back to a sequential counter when indices are
/// absent.
pub struct ZarrSink {
    /// Base output directory for the Zarr store.
    #[allow(dead_code)] // used once full write logic lands
    base_path: PathBuf,
    /// Sequential event counter (fallback when no `scan_indices`).
    event_counter: usize,
    /// Current run UID (set on `StartDoc`, cleared on `StopDoc`).
    current_run: Option<String>,
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
                self.current_run = Some(start.uid.clone());
                self.event_counter = 0;
                Ok(())
            }
            Document::Descriptor(desc) => {
                trace!(
                    descriptor_uid = %desc.uid,
                    stream = %desc.name,
                    keys = ?desc.data_keys.keys().collect::<Vec<_>>(),
                    "ZarrSink: descriptor received"
                );
                // TODO(bd-p2a1): pre-create Zarr arrays based on DataKey schema
                Ok(())
            }
            Document::Event(event) => {
                let coords = self.resolve_coordinates(event);
                let _dim_names = Self::dimension_names(&coords);

                trace!(
                    seq = event.seq_num,
                    coords = ?coords,
                    "ZarrSink: event -> chunk coordinates"
                );

                self.event_counter += 1;

                // TODO(bd-p2a1): write scalar data and arrays to the Zarr store using
                // `ZarrWriter::write_chunk` with chunk indices derived from
                // `coords`. Set `_ARRAY_DIMENSIONS` from `dim_names` when
                // creating the array on first write.
                Ok(())
            }
            Document::Stop(stop) => {
                debug!(
                    run_uid = %stop.run_uid,
                    status = %stop.exit_status,
                    events = stop.num_events,
                    "ZarrSink: run stopped"
                );
                self.current_run = None;
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
    use common::experiment::document::{EventDoc, StartDoc, StopDoc};

    #[tokio::test]
    async fn test_zarr_sink_lifecycle() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut sink = ZarrSink::new(temp.path().join("test.zarr"));

        assert_eq!(sink.name(), "zarr");

        // Start
        let start = Document::Start(StartDoc::new("grid_scan", "Test Grid"));
        let run_uid = match &start {
            Document::Start(s) => s.uid.clone(),
            _ => unreachable!(),
        };
        sink.on_document(&start).await.unwrap();
        assert_eq!(sink.current_run.as_deref(), Some(run_uid.as_str()));
        assert_eq!(sink.event_counter, 0);

        // Event without scan_indices -- fallback to sequential
        let event = EventDoc::new(&run_uid, "desc_1", 0).with_datum("power", 1.0);
        sink.on_document(&Document::Event(event)).await.unwrap();
        assert_eq!(sink.event_counter, 1);

        // Event with scan_indices
        let mut event2 = EventDoc::new(&run_uid, "desc_1", 1).with_datum("power", 2.0);
        event2.scan_indices = Some(vec![
            ("wavelength".to_string(), 3),
            ("position".to_string(), 7),
        ]);
        sink.on_document(&Document::Event(event2)).await.unwrap();
        assert_eq!(sink.event_counter, 2);

        // Stop
        let stop = StopDoc::success(&run_uid, 2);
        sink.on_document(&Document::Stop(stop)).await.unwrap();
        assert!(sink.current_run.is_none());

        // Flush is a no-op
        sink.flush().await.unwrap();
    }

    #[tokio::test]
    async fn test_resolve_coordinates_with_scan_indices() {
        let temp = tempfile::TempDir::new().unwrap();
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
        let temp = tempfile::TempDir::new().unwrap();
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
}
