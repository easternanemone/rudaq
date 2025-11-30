//! Document Model for structured experiment data (bd-73yh.3)
//!
//! Implements the Bluesky-style document model for decoupling data acquisition
//! from storage and visualization. Documents provide:
//!
//! - **StartDoc**: Experiment intent and metadata
//! - **DescriptorDoc**: Schema for data streams
//! - **EventDoc**: Actual measurements at each point
//! - **StopDoc**: Completion status and summary
//!
//! # Document Flow
//!
//! ```text
//! StartDoc (1)
//!    │
//!    ├── DescriptorDoc (1+, one per data stream)
//!    │       │
//!    │       └── EventDoc (N, measurements)
//!    │
//! StopDoc (1)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Generate a new unique document ID
pub fn new_uid() -> String {
    Uuid::new_v4().to_string()
}

/// Current timestamp in nanoseconds since Unix epoch
pub fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

/// Document types for experiment data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Document {
    Start(StartDoc),
    Descriptor(DescriptorDoc),
    Event(EventDoc),
    Stop(StopDoc),
}

impl Document {
    /// Get the document UID
    pub fn uid(&self) -> &str {
        match self {
            Document::Start(d) => &d.uid,
            Document::Descriptor(d) => &d.uid,
            Document::Event(d) => &d.uid,
            Document::Stop(d) => &d.uid,
        }
    }

    /// Get the run UID this document belongs to
    pub fn run_uid(&self) -> &str {
        match self {
            Document::Start(d) => &d.uid, // Start doc UID is the run UID
            Document::Descriptor(d) => &d.run_uid,
            Document::Event(d) => &d.run_uid,
            Document::Stop(d) => &d.run_uid,
        }
    }

    /// Get the timestamp in nanoseconds
    pub fn timestamp_ns(&self) -> u64 {
        match self {
            Document::Start(d) => d.time_ns,
            Document::Descriptor(d) => d.time_ns,
            Document::Event(d) => d.time_ns,
            Document::Stop(d) => d.time_ns,
        }
    }
}

/// Start document - emitted at the beginning of a run
///
/// Contains experiment intent, plan configuration, and user-provided metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartDoc {
    /// Unique run identifier (this IS the run_uid)
    pub uid: String,
    /// Plan type that generated this run
    pub plan_type: String,
    /// User-friendly plan name
    pub plan_name: String,
    /// Plan arguments/configuration
    pub plan_args: HashMap<String, String>,
    /// User-provided metadata
    pub metadata: HashMap<String, String>,
    /// Visualization hints (e.g., preferred plot axes)
    pub hints: Vec<String>,
    /// Timestamp when run started
    pub time_ns: u64,
}

impl StartDoc {
    pub fn new(plan_type: &str, plan_name: &str) -> Self {
        Self {
            uid: new_uid(),
            plan_type: plan_type.to_string(),
            plan_name: plan_name.to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: Vec::new(),
            time_ns: now_ns(),
        }
    }

    pub fn with_arg(mut self, key: &str, value: &str) -> Self {
        self.plan_args.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    pub fn with_hint(mut self, hint: &str) -> Self {
        self.hints.push(hint.to_string());
        self
    }
}

/// Descriptor document - defines schema for event data
///
/// Each descriptor defines a "data stream" with named fields, their types,
/// shapes, and units. A run can have multiple descriptors (e.g., "primary"
/// for main data, "baseline" for background readings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptorDoc {
    /// Unique descriptor ID
    pub uid: String,
    /// Links to StartDoc
    pub run_uid: String,
    /// Stream name (e.g., "primary", "baseline", "monitor")
    pub name: String,
    /// Schema for data fields
    pub data_keys: HashMap<String, DataKey>,
    /// Device configuration at descriptor creation time
    pub configuration: HashMap<String, String>,
    /// Timestamp
    pub time_ns: u64,
}

impl DescriptorDoc {
    pub fn new(run_uid: &str, name: &str) -> Self {
        Self {
            uid: new_uid(),
            run_uid: run_uid.to_string(),
            name: name.to_string(),
            data_keys: HashMap::new(),
            configuration: HashMap::new(),
            time_ns: now_ns(),
        }
    }

    pub fn with_data_key(mut self, name: &str, key: DataKey) -> Self {
        self.data_keys.insert(name.to_string(), key);
        self
    }

    pub fn with_config(mut self, key: &str, value: &str) -> Self {
        self.configuration.insert(key.to_string(), value.to_string());
        self
    }
}

/// Schema for a data field within events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataKey {
    /// Data type: "number", "integer", "string", "array"
    pub dtype: String,
    /// Shape for arrays (empty for scalars)
    pub shape: Vec<i32>,
    /// Source device ID
    pub source: String,
    /// Physical units
    pub units: String,
    /// Measurement precision (optional)
    pub precision: Option<f64>,
    /// Lower limit (for validation/plotting)
    pub lower_limit: Option<f64>,
    /// Upper limit (for validation/plotting)
    pub upper_limit: Option<f64>,
}

impl DataKey {
    /// Create a scalar number data key
    pub fn scalar(source: &str, units: &str) -> Self {
        Self {
            dtype: "number".to_string(),
            shape: vec![],
            source: source.to_string(),
            units: units.to_string(),
            precision: None,
            lower_limit: None,
            upper_limit: None,
        }
    }

    /// Create an array data key
    pub fn array(source: &str, shape: Vec<i32>) -> Self {
        Self {
            dtype: "array".to_string(),
            shape,
            source: source.to_string(),
            units: String::new(),
            precision: None,
            lower_limit: None,
            upper_limit: None,
        }
    }

    pub fn with_precision(mut self, precision: f64) -> Self {
        self.precision = Some(precision);
        self
    }

    pub fn with_limits(mut self, lower: f64, upper: f64) -> Self {
        self.lower_limit = Some(lower);
        self.upper_limit = Some(upper);
        self
    }
}

/// Event document - actual measurement data
///
/// Contains scalar data inline and references bulk data via external storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDoc {
    /// Unique event ID
    pub uid: String,
    /// Links to StartDoc (for quick run lookup)
    pub run_uid: String,
    /// Links to DescriptorDoc that defines schema
    pub descriptor_uid: String,
    /// Event sequence number within this descriptor stream
    pub seq_num: u32,
    /// Timestamp
    pub time_ns: u64,
    /// Scalar data values (field name -> value)
    pub data: HashMap<String, f64>,
    /// Per-field timestamps (field name -> timestamp_ns)
    pub timestamps: HashMap<String, u64>,
    /// Position data (axis name -> position)
    pub positions: HashMap<String, f64>,
}

impl EventDoc {
    pub fn new(run_uid: &str, descriptor_uid: &str, seq_num: u32) -> Self {
        Self {
            uid: new_uid(),
            run_uid: run_uid.to_string(),
            descriptor_uid: descriptor_uid.to_string(),
            seq_num,
            time_ns: now_ns(),
            data: HashMap::new(),
            timestamps: HashMap::new(),
            positions: HashMap::new(),
        }
    }

    pub fn with_datum(mut self, field: &str, value: f64) -> Self {
        let ts = now_ns();
        self.data.insert(field.to_string(), value);
        self.timestamps.insert(field.to_string(), ts);
        self
    }

    pub fn with_position(mut self, axis: &str, position: f64) -> Self {
        self.positions.insert(axis.to_string(), position);
        self
    }
}

/// Stop document - emitted at the end of a run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopDoc {
    /// Unique stop doc ID
    pub uid: String,
    /// Links to StartDoc
    pub run_uid: String,
    /// Exit status: "success", "abort", "fail"
    pub exit_status: String,
    /// Reason for abort/failure
    pub reason: String,
    /// Timestamp when run ended
    pub time_ns: u64,
    /// Total events emitted
    pub num_events: u32,
}

impl StopDoc {
    pub fn success(run_uid: &str, num_events: u32) -> Self {
        Self {
            uid: new_uid(),
            run_uid: run_uid.to_string(),
            exit_status: "success".to_string(),
            reason: String::new(),
            time_ns: now_ns(),
            num_events,
        }
    }

    pub fn abort(run_uid: &str, reason: &str, num_events: u32) -> Self {
        Self {
            uid: new_uid(),
            run_uid: run_uid.to_string(),
            exit_status: "abort".to_string(),
            reason: reason.to_string(),
            time_ns: now_ns(),
            num_events,
        }
    }

    pub fn fail(run_uid: &str, reason: &str, num_events: u32) -> Self {
        Self {
            uid: new_uid(),
            run_uid: run_uid.to_string(),
            exit_status: "fail".to_string(),
            reason: reason.to_string(),
            time_ns: now_ns(),
            num_events,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_doc_builder() {
        let doc = StartDoc::new("grid_scan", "My Grid Scan")
            .with_arg("x_start", "0.0")
            .with_arg("x_end", "10.0")
            .with_metadata("operator", "Alice")
            .with_hint("x_motor");

        assert_eq!(doc.plan_type, "grid_scan");
        assert_eq!(doc.plan_name, "My Grid Scan");
        assert_eq!(doc.plan_args.get("x_start"), Some(&"0.0".to_string()));
        assert_eq!(doc.metadata.get("operator"), Some(&"Alice".to_string()));
        assert!(doc.hints.contains(&"x_motor".to_string()));
    }

    #[test]
    fn test_descriptor_doc() {
        let run_uid = new_uid();
        let desc = DescriptorDoc::new(&run_uid, "primary")
            .with_data_key("power", DataKey::scalar("power_meter", "W"))
            .with_data_key("position", DataKey::scalar("stage_x", "mm"));

        assert_eq!(desc.name, "primary");
        assert!(desc.data_keys.contains_key("power"));
        assert!(desc.data_keys.contains_key("position"));
    }

    #[test]
    fn test_event_doc() {
        let run_uid = new_uid();
        let desc_uid = new_uid();
        let event = EventDoc::new(&run_uid, &desc_uid, 0)
            .with_datum("power", 0.042)
            .with_position("x", 5.0);

        assert_eq!(event.seq_num, 0);
        assert_eq!(event.data.get("power"), Some(&0.042));
        assert_eq!(event.positions.get("x"), Some(&5.0));
    }

    #[test]
    fn test_document_enum() {
        let start = StartDoc::new("test", "Test Run");
        let run_uid = start.uid.clone();
        let doc = Document::Start(start);

        assert_eq!(doc.run_uid(), run_uid);
    }
}
