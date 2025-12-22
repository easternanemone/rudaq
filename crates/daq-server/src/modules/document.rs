//! Structured Document Stream
//!
//! Bluesky-inspired document model for self-describing scientific data.
//! Documents provide metadata context that travels with the data through
//! the processing pipeline.
//!
//! # Document Types
//!
//! - `RunStart` - Marks the beginning of an acquisition run
//! - `Descriptor` - Describes the schema of upcoming Event documents
//! - `Event` - Contains actual measurement data
//! - `RunStop` - Marks the end of an acquisition run
//!
//! # Example
//!
//! ```rust,ignore
//! // Start a new run
//! let run_uid = Uuid::new_v4();
//! ctx.emit_document(Document::run_start(run_uid, "power_scan")).await?;
//!
//! // Describe the data schema
//! ctx.emit_document(Document::descriptor(run_uid, "primary", vec![
//!     DataKey::new("power", "number").with_units("mW"),
//!     DataKey::new("position", "number").with_units("mm"),
//! ])).await?;
//!
//! // Emit events
//! for i in 0..100 {
//!     ctx.emit_document(Document::event(descriptor_uid, hashmap!{
//!         "power" => 42.5,
//!         "position" => i as f64 * 0.1,
//!     })).await?;
//! }
//!
//! // End the run
//! ctx.emit_document(Document::run_stop(run_uid, StopReason::Success)).await?;
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// =============================================================================
// Document Types
// =============================================================================

/// A document in the data stream.
///
/// Documents form a self-describing data pipeline where:
/// - RunStart provides run-level metadata
/// - Descriptor defines the schema for Events
/// - Event contains actual measurements
/// - RunStop finalizes the run
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Document {
    /// Marks the start of an acquisition run
    RunStart(RunStartDoc),
    /// Describes the schema of Event documents
    Descriptor(DescriptorDoc),
    /// Contains measurement data
    Event(EventDoc),
    /// Marks the end of an acquisition run
    RunStop(RunStopDoc),
}

impl Document {
    /// Create a new RunStart document.
    pub fn run_start(run_uid: Uuid, scan_type: impl Into<String>) -> Self {
        Document::RunStart(RunStartDoc {
            uid: run_uid,
            time_ns: now_ns(),
            scan_type: scan_type.into(),
            metadata: HashMap::new(),
        })
    }

    /// Create a new Descriptor document.
    pub fn descriptor(run_start: Uuid, name: impl Into<String>, data_keys: Vec<DataKey>) -> Self {
        Document::Descriptor(DescriptorDoc {
            uid: Uuid::new_v4(),
            run_start,
            time_ns: now_ns(),
            name: name.into(),
            data_keys,
        })
    }

    /// Create a new Event document.
    pub fn event(descriptor: Uuid, data: HashMap<String, serde_json::Value>) -> Self {
        Document::Event(EventDoc {
            uid: Uuid::new_v4(),
            descriptor,
            time_ns: now_ns(),
            seq_num: 0, // Set by emitter
            data,
            timestamps: HashMap::new(),
        })
    }

    /// Create a new RunStop document.
    pub fn run_stop(run_start: Uuid, reason: StopReason) -> Self {
        let exit_status = match &reason {
            StopReason::Success => "success".to_string(),
            StopReason::Abort => "abort".to_string(),
            StopReason::Fail(_) => "fail".to_string(),
        };
        Document::RunStop(RunStopDoc {
            uid: Uuid::new_v4(),
            run_start,
            time_ns: now_ns(),
            reason,
            num_events: 0, // Set by engine
            exit_status,
        })
    }

    /// Get the document UID.
    pub fn uid(&self) -> Uuid {
        match self {
            Document::RunStart(d) => d.uid,
            Document::Descriptor(d) => d.uid,
            Document::Event(d) => d.uid,
            Document::RunStop(d) => d.uid,
        }
    }

    /// Get the timestamp in nanoseconds.
    pub fn time_ns(&self) -> u64 {
        match self {
            Document::RunStart(d) => d.time_ns,
            Document::Descriptor(d) => d.time_ns,
            Document::Event(d) => d.time_ns,
            Document::RunStop(d) => d.time_ns,
        }
    }
}

// =============================================================================
// Document Structures
// =============================================================================

/// Run start document - marks beginning of acquisition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunStartDoc {
    /// Unique identifier for this run
    pub uid: Uuid,
    /// Timestamp (nanoseconds since epoch)
    pub time_ns: u64,
    /// Type of scan/acquisition
    pub scan_type: String,
    /// Additional metadata (sample info, operator, etc.)
    pub metadata: HashMap<String, serde_json::Value>,
}

impl RunStartDoc {
    /// Add metadata to this run start.
    pub fn with_metadata(
        mut self,
        key: impl Into<String>,
        value: impl Serialize,
    ) -> Result<Self, serde_json::Error> {
        self.metadata
            .insert(key.into(), serde_json::to_value(value)?);
        Ok(self)
    }
}

/// Descriptor document - describes schema of Event documents.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DescriptorDoc {
    /// Unique identifier
    pub uid: Uuid,
    /// Reference to the run this belongs to
    pub run_start: Uuid,
    /// Timestamp
    pub time_ns: u64,
    /// Stream name (e.g., "primary", "baseline")
    pub name: String,
    /// Schema for data fields
    pub data_keys: Vec<DataKey>,
}

/// Description of a data field in Event documents.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DataKey {
    /// Field name
    pub name: String,
    /// Data type ("number", "array", "string", "boolean")
    pub dtype: String,
    /// Array shape (empty for scalars)
    #[serde(default)]
    pub shape: Vec<usize>,
    /// Data source (device/module producing this)
    #[serde(default)]
    pub source: String,
    /// Physical units
    #[serde(default)]
    pub units: Option<String>,
    /// Lower limit (for plotting)
    #[serde(default)]
    pub lower_ctrl_limit: Option<f64>,
    /// Upper limit (for plotting)
    #[serde(default)]
    pub upper_ctrl_limit: Option<f64>,
}

impl DataKey {
    /// Create a new DataKey.
    pub fn new(name: impl Into<String>, dtype: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dtype: dtype.into(),
            shape: vec![],
            source: String::new(),
            units: None,
            lower_ctrl_limit: None,
            upper_ctrl_limit: None,
        }
    }

    /// Set the units.
    pub fn with_units(mut self, units: impl Into<String>) -> Self {
        self.units = Some(units.into());
        self
    }

    /// Set the source.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Set the shape (for arrays).
    pub fn with_shape(mut self, shape: Vec<usize>) -> Self {
        self.shape = shape;
        self
    }

    /// Set control limits for plotting.
    pub fn with_limits(mut self, lower: f64, upper: f64) -> Self {
        self.lower_ctrl_limit = Some(lower);
        self.upper_ctrl_limit = Some(upper);
        self
    }
}

/// Event document - contains actual measurement data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventDoc {
    /// Unique identifier
    pub uid: Uuid,
    /// Reference to the descriptor
    pub descriptor: Uuid,
    /// Timestamp
    pub time_ns: u64,
    /// Sequence number within this descriptor stream
    pub seq_num: u64,
    /// Data values (keyed by DataKey names)
    pub data: HashMap<String, serde_json::Value>,
    /// Per-field timestamps (optional, for async data)
    #[serde(default)]
    pub timestamps: HashMap<String, u64>,
}

/// Run stop document - marks end of acquisition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunStopDoc {
    /// Unique identifier
    pub uid: Uuid,
    /// Reference to the run start
    pub run_start: Uuid,
    /// Timestamp
    pub time_ns: u64,
    /// Reason for stopping
    pub reason: StopReason,
    /// Total number of events emitted
    pub num_events: u64,
    /// Exit status string
    pub exit_status: String,
}

/// Reason for run termination.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Normal completion
    Success,
    /// User-requested abort
    Abort,
    /// Error occurred
    Fail(String),
}

// =============================================================================
// Helpers
// =============================================================================

/// Get current time in nanoseconds since epoch.
///
/// Returns 0 if system clock is before Unix epoch (bd-21yj).
fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_lifecycle() {
        let run_uid = Uuid::new_v4();

        let start = Document::run_start(run_uid, "power_scan");
        assert!(matches!(start, Document::RunStart(_)));

        let desc = Document::descriptor(
            run_uid,
            "primary",
            vec![
                DataKey::new("power", "number").with_units("mW"),
                DataKey::new("position", "number").with_units("mm"),
            ],
        );
        let desc_uid = desc.uid();
        assert!(matches!(desc, Document::Descriptor(_)));

        let mut data = HashMap::new();
        data.insert("power".to_string(), serde_json::json!(42.5));
        data.insert("position".to_string(), serde_json::json!(1.0));
        let event = Document::event(desc_uid, data);
        assert!(matches!(event, Document::Event(_)));

        let stop = Document::run_stop(run_uid, StopReason::Success);
        assert!(matches!(stop, Document::RunStop(_)));
    }

    #[test]
    fn test_data_key_builder() {
        let key = DataKey::new("image", "array")
            .with_shape(vec![2048, 2048])
            .with_source("pvcam")
            .with_units("counts");

        assert_eq!(key.name, "image");
        assert_eq!(key.dtype, "array");
        assert_eq!(key.shape, vec![2048, 2048]);
        assert_eq!(key.units.as_deref(), Some("counts"));
    }

    #[test]
    fn test_document_serialization() {
        let run_uid = Uuid::new_v4();
        let doc = Document::run_start(run_uid, "test_scan");

        let json = serde_json::to_string(&doc).unwrap();
        assert!(json.contains("run_start"));
        assert!(json.contains("test_scan"));

        let parsed: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.uid(), run_uid);
    }
}
