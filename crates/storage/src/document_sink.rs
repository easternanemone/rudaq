//! Pluggable sink for consuming Bluesky-style documents.
//!
//! The [`DocumentSink`] trait decouples document production (RunEngine) from
//! consumption (storage backends, live visualisation, search indexing, etc.).
//!
//! Each implementation receives documents independently:
//!
//! ```text
//! RunEngine broadcast
//!   ├─> HDF5Writer  (impl DocumentSink)
//!   ├─> ArrowWriter (impl DocumentSink)
//!   ├─> ZarrSink    (impl DocumentSink)
//!   └─> LivePlot    (impl DocumentSink)
//! ```
//!
//! # Implementing a Sink
//!
//! ```ignore
//! use storage::document_sink::DocumentSink;
//!
//! struct MySink;
//!
//! #[async_trait::async_trait]
//! impl DocumentSink for MySink {
//!     fn name(&self) -> &'static str { "my_sink" }
//!
//!     async fn on_document(&mut self, doc: &Document) -> Result<()> {
//!         // process the document
//!         Ok(())
//!     }
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use common::experiment::document::Document;

/// Pluggable sink for consuming Bluesky-style documents.
///
/// Implementations receive documents from the RunEngine broadcast and
/// process them independently (write to HDF5, stream to Arrow, index for
/// search, etc.).
///
/// Sinks are expected to be driven sequentially per run -- the caller
/// delivers documents in order and may call [`flush`](DocumentSink::flush)
/// at run boundaries or on shutdown.
#[async_trait]
pub trait DocumentSink: Send + Sync {
    /// Process a single document.
    ///
    /// Called once per document in emission order. Implementations should
    /// avoid blocking the async runtime for extended periods; use
    /// `tokio::task::spawn_blocking` for heavy I/O (HDF5, filesystem, etc.).
    async fn on_document(&mut self, doc: &Document) -> Result<()>;

    /// Flush any buffered data to the underlying store.
    ///
    /// Called at run boundaries (after `StopDoc`) and on graceful shutdown.
    /// The default implementation is a no-op.
    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    /// Human-readable name for logging and diagnostics.
    fn name(&self) -> &'static str;
}
