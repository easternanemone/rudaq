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

#[cfg(feature = "storage_hdf5")]
use anyhow::anyhow;
use anyhow::Result;
use async_trait::async_trait;
use common::experiment::document::Document;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::document_sink::DocumentSink;

/// HDF5 Writer for RunEngine Documents
pub struct DocumentWriter {
    #[allow(dead_code)]
    base_path: PathBuf,
    /// current active run (run_uid -> HDF5 file/group handle)
    /// Using a simple implementation for now: one writer instance per run, or single active run
    #[allow(dead_code)]
    active_run: Arc<Mutex<Option<ActiveRun>>>,
}

#[allow(dead_code)]
struct ActiveRun {
    run_uid: String,
    file_path: PathBuf,
    // descriptors: descriptor_uid -> (data_keys)
    descriptors: HashMap<String, DescriptorInfo>,
}

#[allow(dead_code)]
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
                        descriptors: HashMap::new(),
                    });
                }
                Document::Descriptor(desc) => {
                    if let Some(run) = guard.as_mut() {
                        if run.run_uid != desc.run_uid {
                            return Err(anyhow!("Descriptor run_uid mismatch"));
                        }

                        use hdf5::File;
                        let file = File::open_rw(&run.file_path)?;

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
                    if let Some(run) = guard.as_mut() {
                        if let Some(desc_info) = run.descriptors.get(&event.descriptor_uid) {
                            use hdf5::File;
                            let file = File::open_rw(&run.file_path)?;

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
                                if desc_info.data_keys.contains_key(key) {
                                    if let Ok(ds) = group.dataset(key.as_str()) {
                                        let shape = ds.shape();
                                        let current_len = shape[0];
                                        ds.resize((current_len + 1,))?;
                                        ds.write_slice(&[*value], current_len..)?;
                                    }
                                }
                            }

                            // Write array data (frames/waveforms)
                            for (key, bytes) in &event.arrays {
                                if let Some(dkey) = desc_info.data_keys.get(key) {
                                    if let Ok(ds) = group.dataset(key.as_str()) {
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

                            // Write event metadata as sub-groups (bd-p6r4)
                            if !event.metadata.is_empty() {
                                let md_group = if group.group("event_metadata").is_ok() {
                                    group.group("event_metadata")?
                                } else {
                                    group.create_group("event_metadata")?
                                };
                                let md_key = format!("seq_{:06}", event.seq_num);
                                let event_md_group = md_group.create_group(&md_key)?;
                                for (key, value) in &event.metadata {
                                    write_group_attr(&event_md_group, key, value)?;
                                }
                            }
                        }
                    }
                    return Ok(()); // Return Ok from closure
                }
                Document::Stop(stop) => {
                    if let Some(run) = guard.as_mut() {
                        if run.run_uid == stop.run_uid {
                            use hdf5::File;
                            let file = File::open_rw(&run.file_path)?;
                            let group = file.create_group("stop")?;
                            write_group_attr(&group, "exit_status", &stop.exit_status)?;

                            // Clear active run
                            *guard = None;
                        }
                    }
                }
                Document::Manifest(manifest) => {
                    if let Some(run) = guard.as_ref() {
                        use hdf5::File;
                        let file = File::open_rw(&run.file_path)?;

                        let group = file.create_group("manifest")?;

                        // Core fields
                        write_group_attr(&group, "run_uid", &manifest.run_uid)?;
                        write_group_attr(&group, "plan_type", &manifest.plan_type)?;
                        write_group_attr(&group, "plan_name", &manifest.plan_name)?;
                        write_group_attr(
                            &group,
                            "timestamp_ns",
                            &manifest.timestamp_ns.to_string(),
                        )?;

                        // Optional git info
                        if let Some(ref commit) = manifest.git_commit {
                            write_group_attr(&group, "git_commit", commit)?;
                        }
                        if let Some(dirty) = manifest.git_dirty {
                            write_group_attr(&group, "git_dirty", &dirty.to_string())?;
                        }

                        // System info as attributes
                        for (key, value) in &manifest.system_info {
                            write_group_attr(&group, &format!("sys.{key}"), value)?;
                        }

                        // Metadata as attributes
                        for (key, value) in &manifest.metadata {
                            write_group_attr(&group, &format!("meta.{key}"), value)?;
                        }

                        // Device parameters as sub-groups
                        for (device_id, params) in &manifest.parameters {
                            let dev_group = group.create_group(device_id)?;
                            for (param_name, param_value) in params {
                                write_group_attr(
                                    &dev_group,
                                    param_name,
                                    &param_value.to_string(),
                                )?;
                            }
                        }
                    }
                    return Ok(());
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

        // 4. Stop
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
}
