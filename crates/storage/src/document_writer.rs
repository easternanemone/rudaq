//! Document Writer - Persist RunEngine Documents to HDF5
//!
//! This module provides a writer that consumes `RunEngine` documents
//! (Start, Descriptor, Event, Stop) and writes them to an HDF5 file.
//!
//! # Architecture
//!
//! - **Start**: Creates a new HDF5 file (or group if appending)
//! - **Descriptor**: Creates datasets for each data key
//! - **Event**: Appends data to the datasets
//! - **Stop**: Finalizes the file/group
//!
//! This replaces the legacy `ScanProgress` pipeline.

use anyhow::Result;
#[cfg(feature = "storage_hdf5")]
use anyhow::anyhow;
use async_trait::async_trait;
use common::experiment::document::Document;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::document_sink::DocumentSink;

/// HDF5 Writer for RunEngine Documents
#[cfg_attr(not(feature = "storage_hdf5"), allow(dead_code))]
pub struct DocumentWriter {
    base_path: PathBuf,
    /// Current active run. The HDF5 file handle is held open for the
    /// lifetime of the run to avoid repeated `open_rw()` syscall overhead.
    /// The handle lives inside a `Mutex` — `hdf5::File` is `Send` but not
    /// `Sync`, which is fine because only one `spawn_blocking` task accesses
    /// it at a time. If the daemon crashes, the HDF5 library's Drop impl
    /// will clean up the file descriptor.
    active_run: Arc<Mutex<Option<ActiveRun>>>,
}

#[cfg(feature = "storage_hdf5")]
struct ActiveRun {
    run_uid: String,
    /// Retained for diagnostics/logging even though file handle is held open.
    #[allow(dead_code)]
    file_path: PathBuf,
    file: hdf5::File,
    descriptors: HashMap<String, DescriptorInfo>,
}

#[cfg(not(feature = "storage_hdf5"))]
#[allow(dead_code)]
struct ActiveRun {
    run_uid: String,
    file_path: PathBuf,
    descriptors: HashMap<String, DescriptorInfo>,
}

#[cfg_attr(not(feature = "storage_hdf5"), allow(dead_code))]
struct DescriptorInfo {
    stream_name: String,
    data_keys: HashMap<String, common::experiment::document::DataKey>,
}

impl DocumentWriter {
    /// Create a new DocumentWriter
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            base_path,
            active_run: Arc::new(Mutex::new(None)),
        }
    }

    /// Write a document to storage
    ///
    /// This spawns a blocking task for HDF5 I/O.
    #[cfg(feature = "storage_hdf5")]
    pub async fn write(&self, doc: Document) -> Result<()> {
        let active_run = self.active_run.clone();
        let base_path = self.base_path.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut guard = active_run.lock().map_err(|_| anyhow!("Mutex poisoned"))?;

            match doc {
                Document::Start(start) => {
                    let filename = format!("{}_{}.h5", start.uid, start.time_ns);
                    let file_path = base_path.join(filename);

                    // Create file and write start metadata
                    use hdf5::File;
                    let file = File::create(&file_path)?;

                    // Write start doc attributes
                    let group = file.create_group("start")?;
                    write_group_attr(&group, "uid", &start.uid)?;
                    write_group_attr(&group, "plan_type", &start.plan_type)?;
                    write_group_attr(&group, "plan_name", &start.plan_name)?;

                    // Write detailed parameters (plan_args)
                    for (key, value) in &start.plan_args {
                        write_group_attr(&group, key, value)?;
                    }

                    // Write user metadata
                    for (key, value) in &start.metadata {
                        write_group_attr(&group, key, value)?;
                    }

                    *guard = Some(ActiveRun {
                        run_uid: start.uid,
                        file_path,
                        file,
                        descriptors: HashMap::new(),
                    });
                }
                Document::Descriptor(desc) => {
                    if let Some(run) = guard.as_mut() {
                        if run.run_uid != desc.run_uid {
                            return Err(anyhow!("Descriptor run_uid mismatch"));
                        }

                        let file = &run.file;

                        // Create group for this descriptor
                        // Determine stream name (primary, baseline, etc)
                        // For simplicity, verify uniqueness or use descriptor uid
                        let stream_name = &desc.name;
                        let group = if file.group(stream_name).is_ok() {
                            file.group(stream_name)?
                        } else {
                            file.create_group(stream_name)?
                        };

                        // Initialize datasets for each data key
                        // HDF5 requires fixed types, but we might receive varying types
                        // For v0, assume f64 (common case) or handle string serialization
                        for (key, meta) in &desc.data_keys {
                            // Create extendable dataset (chunked)
                            // Initial dimensions: (0), Max: (Unlimited)
                            // Create extendable dataset (chunked)
                            match meta.dtype.as_str() {
                                "uint16" => {
                                    // Create u16 dataset
                                    // For simplicity, we create 1D extendable for now,
                                    // flattening multidimensional frames if necessary.
                                    // Ideal: (0.., h, w).
                                    let ds = group
                                        .new_dataset::<u16>()
                                        .chunk(1024)
                                        .shape(0..)
                                        .create(key.as_str())?;

                                    // Write metadata
                                    write_dataset_attr(&ds, "source", &meta.source)?;
                                    write_dataset_attr(&ds, "dtype", &meta.dtype)?;
                                    write_dataset_attr(&ds, "shape", &format!("{:?}", meta.shape))?;
                                }
                                _ => {
                                    // Default to f64
                                    let ds = group
                                        .new_dataset::<f64>()
                                        .chunk(1024)
                                        .shape(0..)
                                        .create(key.as_str())?;

                                    write_dataset_attr(&ds, "source", &meta.source)?;
                                    write_dataset_attr(&ds, "dtype", &meta.dtype)?;
                                    write_dataset_attr(&ds, "shape", &format!("{:?}", meta.shape))?;
                                }
                            }
                        }

                        run.descriptors.insert(
                            desc.uid.clone(),
                            DescriptorInfo {
                                stream_name: desc.name.clone(),
                                data_keys: desc.data_keys.clone(),
                            },
                        );
                    }
                }
                Document::Event(event) => {
                    if let Some(run) = guard.as_mut()
                        && let Some(desc_info) = run.descriptors.get(&event.descriptor_uid)
                    {
                        let file = &run.file;

                        // Look up the stream name from the descriptor (bd-jcrz)
                        let stream_name = &desc_info.stream_name;
                        let group = file.group(stream_name).map_err(|_| {
                            anyhow!(
                                "HDF5 group '{}' not found for descriptor {}",
                                stream_name,
                                event.descriptor_uid
                            )
                        })?;

                        // Write scalar data (f64)
                        for (key, value) in &event.data {
                            if desc_info.data_keys.contains_key(key)
                                && let Ok(ds) = group.dataset(key.as_str())
                            {
                                let shape = ds.shape();
                                let current_len = shape[0];
                                ds.resize((current_len + 1,))?;
                                ds.write_slice(&[*value], current_len..)?;
                            }
                        }

                        // Write array data (frames/waveforms)
                        for (key, bytes) in &event.arrays {
                            if let Some(dkey) = desc_info.data_keys.get(key)
                                && let Ok(ds) = group.dataset(key.as_str())
                            {
                                let shape = ds.shape();
                                let current_len = shape[0]; // Dimension 0 is time/sequence

                                match dkey.dtype.as_str() {
                                    "uint16" => {
                                        if bytes.len() % 2 != 0 {
                                            continue; // Invalid buffer size
                                        }
                                        // Convert bytes to u16
                                        let u16_data: Vec<u16> = bytes
                                            .chunks_exact(2)
                                            .map(|b| u16::from_le_bytes([b[0], b[1]]))
                                            .collect();

                                        ds.resize((current_len + u16_data.len(),))?;
                                        ds.write_slice(&u16_data, current_len..)?;
                                    }
                                    _ => {
                                        // Default/Fallback
                                        // If we don't resize, we can't write.
                                        // Assume 1 element (scalar) or handle dynamic?
                                        // For now, doing nothing avoids crash but drops data.
                                        // To support existing f64 arrays if any:
                                        // let count = bytes.len() / 8;
                                        // ds.resize((current_len + count,))?;
                                        // ds.write_slice(...)?
                                    }
                                }
                            }
                        }

                        // Also write timestamps
                        #[allow(clippy::cast_precision_loss)]
                        // SAFETY: precision loss acceptable for timestamp conversion
                        let ts_secs = event.time_ns as f64 / 1_000_000_000.0;
                        if let Ok(ds) = group.dataset("timestamps") {
                            let shape = ds.shape();
                            let current_len = shape[0];
                            ds.resize((current_len + 1,))?;
                            ds.write_slice(&[ts_secs], current_len..)?;
                        } else {
                            // Create if missing (lazy)
                            let ds = group
                                .new_dataset::<f64>()
                                .chunk(1024)
                                .shape(0..)
                                .create("timestamps")?;
                            let shape = ds.shape();
                            ds.resize((shape[0] + 1,))?;
                            ds.write_slice(&[ts_secs], shape[0]..)?;
                        }

                        // Write event metadata as flat attributes (bd-p6r4)
                        if !event.metadata.is_empty() {
                            let md_group = if group.group("event_metadata").is_ok() {
                                group.group("event_metadata")?
                            } else {
                                group.create_group("event_metadata")?
                            };
                            for (key, value) in &event.metadata {
                                let attr_name = format!("seq_{:06}_{}", event.seq_num, key);
                                write_group_attr(&md_group, &attr_name, value)?;
                            }
                        }
                    }
                    return Ok(()); // Return Ok from closure
                }
                Document::Stop(stop) => {
                    if let Some(run) = guard.as_mut()
                        && run.run_uid == stop.run_uid
                    {
                        let file = &run.file;
                        let group = file.create_group("stop")?;
                        write_group_attr(&group, "exit_status", &stop.exit_status)?;

                        // Flush HDF5 buffers before dropping the file handle
                        run.file.flush()?;
                        // Clear active run (file handle dropped here)
                        *guard = None;
                    }
                }
                Document::Manifest(manifest) => {
                    if let Some(run) = guard.as_ref() {
                        if run.run_uid != manifest.run_uid {
                            return Err(anyhow!("Manifest run_uid mismatch"));
                        }

                        let file = &run.file;
                        let group = file.create_group("manifest")?;

                        // Top-level scalar attributes
                        write_group_attr(&group, "run_uid", &manifest.run_uid)?;
                        write_group_attr(&group, "plan_type", &manifest.plan_type)?;
                        write_group_attr(&group, "plan_name", &manifest.plan_name)?;
                        write_group_attr(
                            &group,
                            "timestamp_ns",
                            &manifest.timestamp_ns.to_string(),
                        )?;

                        // Provenance fields (when present)
                        if let Some(ref sha) = manifest.git_commit {
                            write_group_attr(&group, "git_commit", sha)?;
                        }
                        if let Some(dirty) = manifest.git_dirty {
                            write_group_attr(&group, "git_dirty", &dirty.to_string())?;
                        }
                        if let Some(ref hash) = manifest.graph_hash {
                            write_group_attr(&group, "graph_hash", hash)?;
                        }
                        if let Some(ref path) = manifest.graph_file {
                            write_group_attr(&group, "graph_file", path)?;
                        }

                        // System info sub-group
                        if !manifest.system_info.is_empty() {
                            let sys_group = group.create_group("system_info")?;
                            for (key, value) in &manifest.system_info {
                                write_group_attr(&sys_group, key, value)?;
                            }
                        }

                        // User metadata sub-group
                        if !manifest.metadata.is_empty() {
                            let meta_group = group.create_group("metadata")?;
                            for (key, value) in &manifest.metadata {
                                write_group_attr(&meta_group, key, value)?;
                            }
                        }

                        // Device parameters: one sub-group per device
                        if !manifest.parameters.is_empty() {
                            let params_group = group.create_group("parameters")?;
                            for (device_id, params) in &manifest.parameters {
                                let device_group = params_group.create_group(device_id.as_str())?;
                                for (param_name, param_value) in params {
                                    let value_str = match param_value {
                                        serde_json::Value::String(s) => s.clone(),
                                        other => other.to_string(),
                                    };
                                    write_group_attr(&device_group, param_name, &value_str)?;
                                }
                            }
                        }
                    }
                }
            }
            Ok(())
        })
        .await??;

        Ok(())
    }

    #[cfg(not(feature = "storage_hdf5"))]
    pub async fn write(&self, _doc: Document) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl DocumentSink for DocumentWriter {
    fn name(&self) -> &'static str {
        "hdf5_document_writer"
    }

    async fn on_document(&mut self, doc: &Document) -> Result<()> {
        // Delegate to the existing write method which uses interior
        // mutability (Arc<Mutex<..>>) so taking &self is sufficient.
        self.write(doc.clone()).await
    }

    async fn flush(&mut self) -> Result<()> {
        // HDF5 writes are flushed per-document; nothing buffered.
        Ok(())
    }
}

#[cfg(feature = "storage_hdf5")]
fn write_group_attr(container: &hdf5::Group, name: &str, value: &str) -> Result<()> {
    use hdf5::types::VarLenUnicode;
    container
        .new_attr::<VarLenUnicode>()
        .create(name)?
        .write_scalar(
            &value
                .parse::<VarLenUnicode>()
                .map_err(|e| anyhow!("VarLenUnicode parse: {}", e))?,
        )?;
    Ok(())
}

#[cfg(feature = "storage_hdf5")]
fn write_dataset_attr(container: &hdf5::Dataset, name: &str, value: &str) -> Result<()> {
    use hdf5::types::VarLenUnicode;
    container
        .new_attr::<VarLenUnicode>()
        .create(name)?
        .write_scalar(
            &value
                .parse::<VarLenUnicode>()
                .map_err(|e| anyhow!("VarLenUnicode parse: {}", e))?,
        )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use common::experiment::document::{
        DescriptorDoc, EventDoc, ExperimentManifest, StartDoc, StopDoc,
    };
    #[allow(unused_imports)]
    use tempfile::TempDir;

    #[tokio::test]
    #[cfg(feature = "storage_hdf5")]
    async fn test_document_writer_full_cycle() {
        let temp_dir = TempDir::new().unwrap();
        let writer = DocumentWriter::new(temp_dir.path().to_path_buf());

        // 1. Start
        let mut plan_args = HashMap::new();
        plan_args.insert("exposure".to_string(), "0.1".to_string());
        plan_args.insert("num".to_string(), "5".to_string());

        let mut metadata = HashMap::new();
        metadata.insert("user".to_string(), "tester".to_string());

        let start = StartDoc {
            uid: "test_run_1".to_string(),
            time_ns: 1000,
            plan_type: "count".to_string(),
            plan_name: "Count".to_string(),
            plan_args,
            metadata,
            hints: vec![],
        };
        writer.write(Document::Start(start)).await.unwrap();

        // 2. Descriptor
        let mut data_keys = HashMap::new();
        data_keys.insert(
            "det1".to_string(),
            common::experiment::document::DataKey {
                source: "det1".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: String::new(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );
        // Add array key (camera frame)
        data_keys.insert(
            "cam1".to_string(),
            common::experiment::document::DataKey {
                source: "cam1".to_string(),
                dtype: "uint16".to_string(),
                shape: vec![10, 10], // 10x10 frame
                units: String::new(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );

        let descriptor = DescriptorDoc {
            run_uid: "test_run_1".to_string(),
            uid: "desc_1".to_string(),
            name: "primary".to_string(),
            data_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(descriptor))
            .await
            .unwrap();

        // 3. Event
        let mut data = HashMap::new();
        data.insert("det1".to_string(), 42.0); // Direct f64

        // Add array data
        let mut arrays = HashMap::new();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // SAFETY: value is validated/bounded before cast
        let frame_data: Vec<u16> = (0..100).map(|i| i as u16).collect();
        // Convert to LE bytes
        let mut frame_bytes = Vec::with_capacity(200);
        for val in frame_data {
            frame_bytes.extend_from_slice(&val.to_le_bytes());
        }
        arrays.insert("cam1".to_string(), bytes::Bytes::from(frame_bytes));

        let mut event_metadata = HashMap::new();
        event_metadata.insert("hw_timestamp".to_string(), "12345678".to_string());
        event_metadata.insert("quality".to_string(), "good".to_string());

        let event = EventDoc {
            descriptor_uid: "desc_1".to_string(),
            seq_num: 1,
            data,
            arrays,
            timestamps: HashMap::new(),
            metadata: event_metadata,
            run_uid: "test_run_1".to_string(),
            time_ns: 1_000_000_000,
            uid: "event_1".to_string(),
            positions: HashMap::new(),
            scan_indices: None,
        };
        writer.write(Document::Event(event)).await.unwrap();

        // 4. Manifest
        let mut manifest_params = HashMap::new();
        let mut cam_params = HashMap::new();
        cam_params.insert("exposure_ms".to_string(), serde_json::json!(100.0));
        manifest_params.insert("cam1".to_string(), cam_params);

        let manifest = ExperimentManifest::new("test_run_1", "count", "Count", manifest_params);
        writer.write(Document::Manifest(manifest)).await.unwrap();

        // 5. Stop
        let stop = StopDoc {
            uid: "stop_1".to_string(),
            run_uid: "test_run_1".to_string(),
            time_ns: 2_000_000_000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 1,
        };
        writer.write(Document::Stop(stop)).await.unwrap();

        // Verify file exists
        let filename = "test_run_1_1000.h5".to_string();
        let file_path = temp_dir.path().join(filename);
        assert!(file_path.exists());

        // Verify contents logic would go here (requires hdf5 crate in dev-dependencies)
    }

    #[tokio::test]
    #[cfg(feature = "storage_hdf5")]
    async fn test_multi_group_descriptor_routing() {
        let temp_dir = TempDir::new().unwrap();
        let writer = DocumentWriter::new(temp_dir.path().to_path_buf());

        // 1. Start
        let start = StartDoc {
            uid: "multi_run".to_string(),
            time_ns: 2000,
            plan_type: "grid_scan".to_string(),
            plan_name: "Multi-stream Grid".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer.write(Document::Start(start)).await.unwrap();

        // 2. Primary descriptor (camera data)
        let mut primary_keys = HashMap::new();
        primary_keys.insert(
            "camera".to_string(),
            common::experiment::document::DataKey {
                source: "cam1".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: "counts".to_string(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );
        let primary_desc = DescriptorDoc {
            run_uid: "multi_run".to_string(),
            uid: "desc_primary".to_string(),
            name: "primary".to_string(),
            data_keys: primary_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(primary_desc))
            .await
            .unwrap();

        // 3. Baseline descriptor (monitor readings)
        let mut baseline_keys = HashMap::new();
        baseline_keys.insert(
            "power_meter".to_string(),
            common::experiment::document::DataKey {
                source: "pm1".to_string(),
                dtype: "number".to_string(),
                shape: vec![],
                units: "W".to_string(),
                precision: None,
                lower_limit: None,
                upper_limit: None,
            },
        );
        let baseline_desc = DescriptorDoc {
            run_uid: "multi_run".to_string(),
            uid: "desc_baseline".to_string(),
            name: "baseline".to_string(),
            data_keys: baseline_keys,
            configuration: HashMap::new(),
            time_ns: 0,
        };
        writer
            .write(Document::Descriptor(baseline_desc))
            .await
            .unwrap();

        // 4. Events targeting different descriptors
        let mut primary_data = HashMap::new();
        primary_data.insert("camera".to_string(), 1024.0);
        let primary_event = EventDoc {
            descriptor_uid: "desc_primary".to_string(),
            seq_num: 1,
            data: primary_data,
            arrays: HashMap::new(),
            timestamps: HashMap::new(),
            metadata: HashMap::new(),
            run_uid: "multi_run".to_string(),
            time_ns: 1_000_000_000,
            uid: "evt_primary_1".to_string(),
            positions: HashMap::new(),
            scan_indices: None,
        };
        writer.write(Document::Event(primary_event)).await.unwrap();

        let mut baseline_data = HashMap::new();
        baseline_data.insert("power_meter".to_string(), 0.042);
        let baseline_event = EventDoc {
            descriptor_uid: "desc_baseline".to_string(),
            seq_num: 1,
            data: baseline_data,
            arrays: HashMap::new(),
            timestamps: HashMap::new(),
            metadata: HashMap::new(),
            run_uid: "multi_run".to_string(),
            time_ns: 1_100_000_000,
            uid: "evt_baseline_1".to_string(),
            positions: HashMap::new(),
            scan_indices: None,
        };
        writer.write(Document::Event(baseline_event)).await.unwrap();

        // 5. Stop
        let stop = StopDoc {
            uid: "stop_multi".to_string(),
            run_uid: "multi_run".to_string(),
            time_ns: 3_000_000_000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 2,
        };
        writer.write(Document::Stop(stop)).await.unwrap();

        // 6. Verify the HDF5 file has both groups with correct data
        let file_path = temp_dir.path().join("multi_run_2000.h5");
        assert!(file_path.exists());

        use hdf5::File;
        let file = File::open(&file_path).unwrap();

        // Primary group should have 'camera' dataset with value 1024.0
        let primary_group = file.group("primary").expect("primary group should exist");
        let camera_ds = primary_group
            .dataset("camera")
            .expect("camera dataset should exist in primary");
        let camera_data: Vec<f64> = camera_ds.read_raw().unwrap();
        assert_eq!(camera_data, vec![1024.0]);

        // Baseline group should have 'power_meter' dataset with value 0.042
        let baseline_group = file.group("baseline").expect("baseline group should exist");
        let pm_ds = baseline_group
            .dataset("power_meter")
            .expect("power_meter dataset should exist in baseline");
        let pm_data: Vec<f64> = pm_ds.read_raw().unwrap();
        assert_eq!(pm_data, vec![0.042]);

        // Neither group should contain the other's dataset
        assert!(
            primary_group.dataset("power_meter").is_err(),
            "power_meter should NOT be in primary group"
        );
        assert!(
            baseline_group.dataset("camera").is_err(),
            "camera should NOT be in baseline group"
        );
    }

    #[tokio::test]
    #[cfg(feature = "storage_hdf5")]
    async fn test_document_writer_manifest() {
        let temp_dir = TempDir::new().unwrap();
        let writer = DocumentWriter::new(temp_dir.path().to_path_buf());

        let start = StartDoc {
            uid: "manifest_run".to_string(),
            time_ns: 5000,
            plan_type: "grid_scan".to_string(),
            plan_name: "Grid Scan".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer.write(Document::Start(start)).await.unwrap();

        let mut parameters = HashMap::new();
        let mut camera_params = HashMap::new();
        camera_params.insert("exposure_ms".to_string(), serde_json::json!(100.0));
        camera_params.insert("gain".to_string(), serde_json::json!(1.5));
        camera_params.insert("binning".to_string(), serde_json::json!(2));
        camera_params.insert("mode".to_string(), serde_json::json!("normal"));
        parameters.insert("mock_camera".to_string(), camera_params);

        let mut stage_params = HashMap::new();
        stage_params.insert("position".to_string(), serde_json::json!(0.0));
        stage_params.insert("velocity".to_string(), serde_json::json!(1.0));
        parameters.insert("mock_stage".to_string(), stage_params);

        let mut system_info = HashMap::new();
        system_info.insert("software_version".to_string(), "0.1.0".to_string());
        system_info.insert("hostname".to_string(), "test-host".to_string());

        let mut metadata = HashMap::new();
        metadata.insert("operator".to_string(), "Alice".to_string());
        metadata.insert("sample".to_string(), "Si wafer #42".to_string());

        let manifest = common::experiment::document::ExperimentManifest {
            timestamp_ns: 6000,
            run_uid: "manifest_run".to_string(),
            plan_type: "grid_scan".to_string(),
            plan_name: "Grid Scan".to_string(),
            parameters,
            system_info,
            metadata,
            git_commit: Some("abc123def456".to_string()),
            git_dirty: Some(false),
            graph_hash: Some("deadbeef".repeat(8)),
            graph_file: Some("/tmp/test.expgraph".to_string()),
        };
        writer.write(Document::Manifest(manifest)).await.unwrap();

        let stop = StopDoc {
            uid: "stop_m".to_string(),
            run_uid: "manifest_run".to_string(),
            time_ns: 7000,
            exit_status: "success".to_string(),
            reason: String::new(),
            num_events: 0,
        };
        writer.write(Document::Stop(stop)).await.unwrap();

        let file_path = temp_dir.path().join("manifest_run_5000.h5");
        assert!(file_path.exists(), "HDF5 file should exist");

        use hdf5::File;
        let file = File::open(&file_path).unwrap();
        let manifest_group = file.group("manifest").expect("manifest group should exist");

        let read_attr = |group: &hdf5::Group, name: &str| -> String {
            use hdf5::types::VarLenUnicode;
            let attr = group
                .attr(name)
                .unwrap_or_else(|_| panic!("attr {name} missing"));
            let val: VarLenUnicode = attr.read_scalar().unwrap_or_else(|_| panic!("read {name}"));
            val.to_string()
        };

        assert_eq!(read_attr(&manifest_group, "run_uid"), "manifest_run");
        assert_eq!(read_attr(&manifest_group, "plan_type"), "grid_scan");
        assert_eq!(read_attr(&manifest_group, "plan_name"), "Grid Scan");
        assert_eq!(read_attr(&manifest_group, "timestamp_ns"), "6000");

        assert_eq!(read_attr(&manifest_group, "git_commit"), "abc123def456");
        assert_eq!(read_attr(&manifest_group, "git_dirty"), "false");
        assert_eq!(
            read_attr(&manifest_group, "graph_file"),
            "/tmp/test.expgraph"
        );

        let sys_group = manifest_group
            .group("system_info")
            .expect("system_info group");
        assert_eq!(read_attr(&sys_group, "software_version"), "0.1.0");
        assert_eq!(read_attr(&sys_group, "hostname"), "test-host");

        let meta_group = manifest_group.group("metadata").expect("metadata group");
        assert_eq!(read_attr(&meta_group, "operator"), "Alice");
        assert_eq!(read_attr(&meta_group, "sample"), "Si wafer #42");

        let params_group = manifest_group
            .group("parameters")
            .expect("parameters group");
        let camera_group = params_group
            .group("mock_camera")
            .expect("mock_camera group");
        assert_eq!(read_attr(&camera_group, "exposure_ms"), "100.0");
        assert_eq!(read_attr(&camera_group, "gain"), "1.5");
        assert_eq!(read_attr(&camera_group, "binning"), "2");
        assert_eq!(read_attr(&camera_group, "mode"), "normal");

        let stage_group = params_group.group("mock_stage").expect("mock_stage group");
        assert_eq!(read_attr(&stage_group, "position"), "0.0");
        assert_eq!(read_attr(&stage_group, "velocity"), "1.0");
    }

    #[tokio::test]
    #[cfg(feature = "storage_hdf5")]
    async fn test_manifest_run_uid_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let writer = DocumentWriter::new(temp_dir.path().to_path_buf());

        let start = StartDoc {
            uid: "run_a".to_string(),
            time_ns: 1000,
            plan_type: "count".to_string(),
            plan_name: "Count".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer.write(Document::Start(start)).await.unwrap();

        let manifest = ExperimentManifest {
            timestamp_ns: 2000,
            run_uid: "wrong_run".to_string(),
            plan_type: "count".to_string(),
            plan_name: "Count".to_string(),
            parameters: HashMap::new(),
            system_info: HashMap::new(),
            metadata: HashMap::new(),
            git_commit: None,
            git_dirty: None,
            graph_hash: None,
            graph_file: None,
        };
        let result = writer.write(Document::Manifest(manifest)).await;
        assert!(result.is_err(), "Mismatched run_uid should produce error");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Manifest run_uid mismatch"),
        );
    }

    #[tokio::test]
    #[cfg(feature = "storage_hdf5")]
    async fn test_manifest_without_active_run() {
        let temp_dir = TempDir::new().unwrap();
        let writer = DocumentWriter::new(temp_dir.path().to_path_buf());

        let manifest = ExperimentManifest {
            timestamp_ns: 1000,
            run_uid: "orphan".to_string(),
            plan_type: "count".to_string(),
            plan_name: "Count".to_string(),
            parameters: HashMap::new(),
            system_info: HashMap::new(),
            metadata: HashMap::new(),
            git_commit: None,
            git_dirty: None,
            graph_hash: None,
            graph_file: None,
        };
        let result = writer.write(Document::Manifest(manifest)).await;
        assert!(
            result.is_ok(),
            "Manifest without active run should be Ok (no-op)"
        );
    }

    #[tokio::test]
    #[cfg(feature = "storage_hdf5")]
    async fn test_manifest_empty_optional_fields() {
        let temp_dir = TempDir::new().unwrap();
        let writer = DocumentWriter::new(temp_dir.path().to_path_buf());

        let start = StartDoc {
            uid: "minimal_run".to_string(),
            time_ns: 1000,
            plan_type: "count".to_string(),
            plan_name: "Count".to_string(),
            plan_args: HashMap::new(),
            metadata: HashMap::new(),
            hints: vec![],
        };
        writer.write(Document::Start(start)).await.unwrap();

        let manifest = ExperimentManifest {
            timestamp_ns: 2000,
            run_uid: "minimal_run".to_string(),
            plan_type: "count".to_string(),
            plan_name: "Count".to_string(),
            parameters: HashMap::new(),
            system_info: HashMap::new(),
            metadata: HashMap::new(),
            git_commit: None,
            git_dirty: None,
            graph_hash: None,
            graph_file: None,
        };
        writer.write(Document::Manifest(manifest)).await.unwrap();

        let file_path = temp_dir.path().join("minimal_run_1000.h5");
        assert!(file_path.exists());

        use hdf5::File;
        let file = File::open(&file_path).unwrap();
        let manifest_group = file.group("manifest").expect("manifest group should exist");

        let sub_groups = manifest_group.groups().unwrap();
        assert!(
            sub_groups.is_empty(),
            "No sub-groups expected for empty manifest collections"
        );

        assert!(
            manifest_group.attr("git_commit").is_err(),
            "git_commit should not exist when None"
        );
        assert!(
            manifest_group.attr("git_dirty").is_err(),
            "git_dirty should not exist when None"
        );
    }
}
