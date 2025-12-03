//! Document Model for structured experiment data (bd-73yh.3)
//!
//! Implements the Bluesky-style document model for decoupling data acquisition
//! from storage and visualization. Documents provide:
//!
//! - **StartDoc**: Experiment intent and metadata
//! - **DescriptorDoc**: Schema for data streams
//! - **EventDoc**: Actual measurements at each point
//! - **StopDoc**: Completion status and summary
//! - **ExperimentManifest**: Hardware parameter snapshot for reproducibility (bd-ej44)
//!
//! # Document Flow
//!
//! ```text
//! StartDoc (1)
//!    │
//!    ├── ExperimentManifest (1, hardware parameter snapshot)
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
    /// Experiment start document - intent and metadata
    Start(StartDoc),
    /// Data stream descriptor - schema definition
    Descriptor(DescriptorDoc),
    /// Event document - actual measurement data
    Event(EventDoc),
    /// Experiment stop document - completion status
    Stop(StopDoc),
    /// Experiment manifest - hardware parameter snapshot (bd-ib06)
    Manifest(ExperimentManifest),
}

impl Document {
    /// Get the document UID
    pub fn uid(&self) -> &str {
        match self {
            Document::Start(d) => &d.uid,
            Document::Descriptor(d) => &d.uid,
            Document::Event(d) => &d.uid,
            Document::Stop(d) => &d.uid,
            Document::Manifest(d) => &d.run_uid,
        }
    }

    /// Get the run UID this document belongs to
    pub fn run_uid(&self) -> &str {
        match self {
            Document::Start(d) => &d.uid, // Start doc UID is the run UID
            Document::Descriptor(d) => &d.run_uid,
            Document::Event(d) => &d.run_uid,
            Document::Stop(d) => &d.run_uid,
            Document::Manifest(d) => &d.run_uid,
        }
    }

    /// Get the timestamp in nanoseconds
    pub fn timestamp_ns(&self) -> u64 {
        match self {
            Document::Start(d) => d.time_ns,
            Document::Descriptor(d) => d.time_ns,
            Document::Event(d) => d.time_ns,
            Document::Stop(d) => d.time_ns,
            Document::Manifest(d) => d.timestamp_ns,
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
        self.configuration
            .insert(key.to_string(), value.to_string());
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

// =============================================================================
// Experiment Manifest (bd-ej44)
// =============================================================================

/// Experiment manifest - complete hardware state snapshot for reproducibility
///
/// Captures all device parameters at experiment start to ensure experiments
/// can be reproduced with identical hardware configuration.
///
/// # Structure
///
/// ```json
/// {
///   "timestamp_ns": 1234567890000000000,
///   "run_uid": "abc-123-def-456",
///   "plan_type": "grid_scan",
///   "plan_name": "Polarization Map",
///   "parameters": {
///     "mock_camera": {
///       "exposure_ms": 100.0,
///       "gain": 1.5,
///       "binning": 2
///     },
///     "mock_stage": {
///       "position": 0.0,
///       "velocity": 1.0
///     }
///   },
///   "system_info": {
///     "software_version": "0.1.0",
///     "hostname": "lab-daq-01"
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentManifest {
    /// When this manifest was captured
    pub timestamp_ns: u64,
    /// Run UID this manifest belongs to
    pub run_uid: String,
    /// Plan type being executed
    pub plan_type: String,
    /// User-friendly plan name
    pub plan_name: String,
    /// Complete parameter snapshot: device_id -> parameter_name -> value
    pub parameters: HashMap<String, HashMap<String, serde_json::Value>>,
    /// System information (software version, hostname, etc.)
    pub system_info: HashMap<String, String>,
    /// User-provided metadata from StartDoc
    pub metadata: HashMap<String, String>,
}

impl ExperimentManifest {
    /// Create a new experiment manifest
    ///
    /// # Arguments
    ///
    /// * `run_uid` - Run identifier from StartDoc
    /// * `plan_type` - Plan type being executed
    /// * `plan_name` - User-friendly plan name
    /// * `parameters` - Device parameter snapshot from DeviceRegistry
    pub fn new(
        run_uid: &str,
        plan_type: &str,
        plan_name: &str,
        parameters: HashMap<String, HashMap<String, serde_json::Value>>,
    ) -> Self {
        let mut system_info = HashMap::new();
        system_info.insert(
            "software_version".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        );

        // Capture hostname if available
        if let Ok(hostname) = hostname::get() {
            if let Ok(hostname_str) = hostname.into_string() {
                system_info.insert("hostname".to_string(), hostname_str);
            }
        }

        Self {
            timestamp_ns: now_ns(),
            run_uid: run_uid.to_string(),
            plan_type: plan_type.to_string(),
            plan_name: plan_name.to_string(),
            parameters,
            system_info,
            metadata: HashMap::new(),
        }
    }

    /// Add user metadata to the manifest
    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Add a single metadata entry
    pub fn add_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    /// Add system information
    pub fn add_system_info(mut self, key: &str, value: &str) -> Self {
        self.system_info.insert(key.to_string(), value.to_string());
        self
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Serialize to JSON value (for HDF5 attributes)
    pub fn to_json_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
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

    #[test]
    fn test_experiment_manifest() {
        use std::collections::HashMap;

        // Create mock parameter snapshot
        let mut parameters = HashMap::new();
        let mut camera_params = HashMap::new();
        camera_params.insert("exposure_ms".to_string(), serde_json::json!(100.0));
        camera_params.insert("gain".to_string(), serde_json::json!(1.5));
        parameters.insert("mock_camera".to_string(), camera_params);

        let mut stage_params = HashMap::new();
        stage_params.insert("position".to_string(), serde_json::json!(0.0));
        parameters.insert("mock_stage".to_string(), stage_params);

        // Create manifest
        let manifest =
            ExperimentManifest::new("test-run-123", "grid_scan", "Test Grid Scan", parameters);

        assert_eq!(manifest.run_uid, "test-run-123");
        assert_eq!(manifest.plan_type, "grid_scan");
        assert_eq!(manifest.plan_name, "Test Grid Scan");
        assert_eq!(manifest.parameters.len(), 2);
        assert!(manifest.parameters.contains_key("mock_camera"));
        assert!(manifest.parameters.contains_key("mock_stage"));

        // Check camera parameters
        let camera = manifest.parameters.get("mock_camera").unwrap();
        assert_eq!(camera.get("exposure_ms"), Some(&serde_json::json!(100.0)));
        assert_eq!(camera.get("gain"), Some(&serde_json::json!(1.5)));

        // Check system info is populated
        assert!(manifest.system_info.contains_key("software_version"));
    }

    #[test]
    fn test_manifest_serialization() {
        use std::collections::HashMap;

        let parameters = HashMap::new();
        let manifest = ExperimentManifest::new("run-456", "count", "Simple Count", parameters)
            .add_metadata("operator", "Alice")
            .add_system_info("test_key", "test_value");

        // Test JSON serialization
        let json = manifest.to_json().unwrap();
        assert!(json.contains("run-456"));
        assert!(json.contains("count"));
        assert!(json.contains("Simple Count"));
        assert!(json.contains("Alice"));

        // Test JSON value conversion
        let json_value = manifest.to_json_value().unwrap();
        assert!(json_value.is_object());
    }
}
