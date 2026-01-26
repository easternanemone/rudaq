//! Zarr V3 Writer - Cloud-native N-dimensional array storage
//!
//! This module provides Zarr V3 storage for nested multi-dimensional scans,
//! replacing HDF5 as the primary format for cloud-native workflows.
//!
//! # Architecture
//!
//! ```text
//! Experiment Data -> ZarrWriter -> Zarr V3 Store -> Xarray (Python)
//!                                     |
//!                              experiment.zarr/
//!                              +-- zarr.json
//!                              +-- data/
//!                                  +-- zarr.json
//!                                  +-- c/0/0/0
//! ```
//!
//! # Xarray Compatibility
//!
//! Arrays include `_ARRAY_DIMENSIONS` attribute for seamless
//! Xarray interoperability:
//!
//! ```python
//! import xarray as xr
//! ds = xr.open_zarr("experiment.zarr")
//! # Dimensions automatically recognized from _ARRAY_DIMENSIONS
//! ```
//!
//! # Example
//!
//! ```no_run
//! use daq_storage::zarr_writer::{ZarrWriter, ZarrArrayBuilder};
//! use std::path::Path;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let writer = ZarrWriter::new(Path::new("experiment.zarr")).await?;
//!
//!     // Create 4D array for nested scan data
//!     writer.create_array()
//!         .name("camera_frames")
//!         .shape(vec![10, 5, 256, 256])
//!         .chunks(vec![10, 1, 256, 256])
//!         .dimensions(vec!["wavelength", "position", "y", "x"])
//!         .dtype_u16()
//!         .build()
//!         .await?;
//!
//!     // Write chunk at indices [wavelength=0, position=0]
//!     let frame_data = vec![0u16; 10 * 256 * 256];
//!     writer.write_chunk::<u16>("camera_frames", &[0, 0, 0, 0], frame_data).await?;
//!
//!     Ok(())
//! }
//! ```

use anyhow::{anyhow, Result};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use zarrs::array::{Array, ArrayBuilder, DataType, FillValue};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::GroupBuilder;
use zarrs::storage::ReadableWritableListableStorage;

/// Handle to a created Zarr array for writing chunks
struct ArrayHandle {
    path: String,
    #[allow(dead_code)]
    data_type: DataType,
}

/// Zarr V3 writer for N-dimensional scientific data
///
/// Provides a high-level API for creating Zarr stores with Xarray-compatible
/// encoding. All file I/O operations are performed via `tokio::task::spawn_blocking`
/// to prevent blocking the async runtime.
pub struct ZarrWriter {
    output_path: PathBuf,
    store: ReadableWritableListableStorage,
    arrays: RwLock<HashMap<String, ArrayHandle>>,
}

impl ZarrWriter {
    /// Create a new Zarr V3 store at the given path
    ///
    /// Creates the store directory and root group with Zarr V3 metadata.
    ///
    /// # Arguments
    ///
    /// * `path` - Path where Zarr store will be created (e.g., "experiment.zarr")
    ///
    /// # Returns
    ///
    /// A new `ZarrWriter` instance ready for array creation
    ///
    /// # Errors
    ///
    /// Returns error if store creation fails (permissions, disk space, etc.)
    pub async fn new(path: &Path) -> Result<Self> {
        let output_path = path.to_path_buf();
        let path_clone = output_path.clone();

        // Create store in blocking context
        let store: ReadableWritableListableStorage =
            tokio::task::spawn_blocking(move || -> Result<ReadableWritableListableStorage> {
                let store = FilesystemStore::new(&path_clone)
                    .map_err(|e| anyhow!("Failed to create Zarr store: {}", e))?;
                let store_arc: ReadableWritableListableStorage = Arc::new(store);

                // Create root group
                let group = GroupBuilder::new()
                    .build(store_arc.clone(), "/")
                    .map_err(|e| anyhow!("Failed to create root group: {}", e))?;
                group
                    .store_metadata()
                    .map_err(|e| anyhow!("Failed to store root group metadata: {}", e))?;

                Ok(store_arc)
            })
            .await??;

        Ok(Self {
            output_path,
            store,
            arrays: RwLock::new(HashMap::new()),
        })
    }

    /// Start building a new array
    ///
    /// Returns a fluent builder for configuring array shape, chunking,
    /// dimensions, and data type.
    pub fn create_array(&self) -> ZarrArrayBuilder<'_> {
        ZarrArrayBuilder {
            writer: self,
            name: None,
            shape: None,
            chunks: None,
            dimensions: None,
            data_type: None,
            fill_value: None,
            attributes: serde_json::Map::new(),
        }
    }

    /// Write a chunk of data to an array
    ///
    /// # Type Parameters
    ///
    /// * `T` - Element type (must match array's dtype: u8, u16, f32, f64, etc.)
    ///
    /// # Arguments
    ///
    /// * `array_name` - Name of the array (as passed to `ZarrArrayBuilder::name()`)
    /// * `chunk_indices` - Chunk coordinates (length must match array dimensions)
    /// * `data` - Flat array of elements (length must equal chunk size)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Array doesn't exist
    /// - Type mismatch with array dtype
    /// - Chunk indices out of bounds
    /// - Data length doesn't match chunk size
    pub async fn write_chunk<T: zarrs::array::Element + Send + Sync + 'static>(
        &self,
        array_name: &str,
        chunk_indices: &[u64],
        data: Vec<T>,
    ) -> Result<()> {
        let arrays = self.arrays.read().await;
        let handle = arrays
            .get(array_name)
            .ok_or_else(|| anyhow!("Array '{}' not found", array_name))?;

        // Clone what we need for the blocking task
        let array_path = handle.path.clone();
        let indices: Vec<u64> = chunk_indices.to_vec();
        let store = self.store.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let array = Array::open(store, &array_path)
                .map_err(|e| anyhow!("Failed to open array: {}", e))?;

            array
                .store_chunk_elements(&indices, &data)
                .map_err(|e| anyhow!("Failed to write chunk: {}", e))?;
            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Add an attribute to the root group
    ///
    /// Use for experiment-level metadata (scan ID, timestamps, etc.)
    ///
    /// # Arguments
    ///
    /// * `key` - Attribute name
    /// * `value` - JSON-serializable value
    pub async fn add_group_attribute(&self, key: &str, value: serde_json::Value) -> Result<()> {
        let store = self.store.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> Result<()> {
            // Read existing root group to get current attributes
            let existing_group = zarrs::group::Group::open(store.clone(), "/")
                .map_err(|e| anyhow!("Failed to open root group: {}", e))?;

            // Build new group with updated attributes
            let mut attrs = existing_group.attributes().clone();
            attrs.insert(key, value);

            let group = GroupBuilder::new()
                .attributes(attrs)
                .build(store, "/")
                .map_err(|e| anyhow!("Failed to update root group: {}", e))?;

            group
                .store_metadata()
                .map_err(|e| anyhow!("Failed to store group metadata: {}", e))?;

            Ok(())
        })
        .await??;

        Ok(())
    }

    /// Get the output path of this Zarr store
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Internal: Register a created array
    async fn register_array(&self, name: String, path: String, data_type: DataType) {
        let mut arrays = self.arrays.write().await;
        arrays.insert(name, ArrayHandle { path, data_type });
    }
}

/// Fluent builder for creating Zarr arrays
///
/// Allows chained configuration of array properties before creation.
pub struct ZarrArrayBuilder<'a> {
    writer: &'a ZarrWriter,
    name: Option<String>,
    shape: Option<Vec<u64>>,
    chunks: Option<Vec<u64>>,
    dimensions: Option<Vec<String>>,
    data_type: Option<DataType>,
    fill_value: Option<FillValueSpec>,
    attributes: serde_json::Map<String, serde_json::Value>,
}

/// Internal type for deferred fill value specification
enum FillValueSpec {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl<'a> ZarrArrayBuilder<'a> {
    /// Set the array name (path within store)
    ///
    /// The name becomes the array path, e.g., "camera_frames" creates
    /// an array at `/camera_frames` in the store.
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    /// Set the array shape
    ///
    /// # Arguments
    ///
    /// * `dims` - Array dimensions, e.g., `[10, 5, 256, 256]` for
    ///   10 wavelengths x 5 positions x 256x256 camera frames
    pub fn shape(mut self, dims: Vec<u64>) -> Self {
        self.shape = Some(dims);
        self
    }

    /// Set chunk sizes
    ///
    /// Chunks should target 10-100 MB for optimal performance.
    /// Match chunk boundaries to access patterns.
    ///
    /// # Arguments
    ///
    /// * `sizes` - Chunk dimensions (must match array dimensionality)
    ///
    /// # Example
    ///
    /// For time-series analysis across wavelengths:
    /// ```ignore
    /// // Array shape: [wavelengths=10, positions=5, y=256, x=256]
    /// // Access pattern: extract all wavelengths at each position
    /// builder.chunks(vec![10, 1, 256, 256])  // All wavelengths per chunk
    /// ```
    pub fn chunks(mut self, sizes: Vec<u64>) -> Self {
        self.chunks = Some(sizes);
        self
    }

    /// Set dimension names for Xarray compatibility
    ///
    /// This writes the `_ARRAY_DIMENSIONS` attribute that Xarray uses
    /// to recognize dimensional structure.
    ///
    /// # Arguments
    ///
    /// * `names` - Dimension names in order, e.g., `["wavelength", "position", "y", "x"]`
    pub fn dimensions<S: AsRef<str>>(mut self, names: Vec<S>) -> Self {
        self.dimensions = Some(names.iter().map(|s| s.as_ref().to_string()).collect());
        self
    }

    /// Set data type to unsigned 8-bit integer
    pub fn dtype_u8(mut self) -> Self {
        self.data_type = Some(DataType::UInt8);
        self.fill_value = Some(FillValueSpec::U8(0));
        self
    }

    /// Set data type to unsigned 16-bit integer (common for camera data)
    pub fn dtype_u16(mut self) -> Self {
        self.data_type = Some(DataType::UInt16);
        self.fill_value = Some(FillValueSpec::U16(0));
        self
    }

    /// Set data type to unsigned 32-bit integer
    pub fn dtype_u32(mut self) -> Self {
        self.data_type = Some(DataType::UInt32);
        self.fill_value = Some(FillValueSpec::U32(0));
        self
    }

    /// Set data type to unsigned 64-bit integer
    pub fn dtype_u64(mut self) -> Self {
        self.data_type = Some(DataType::UInt64);
        self.fill_value = Some(FillValueSpec::U64(0));
        self
    }

    /// Set data type to signed 8-bit integer
    pub fn dtype_i8(mut self) -> Self {
        self.data_type = Some(DataType::Int8);
        self.fill_value = Some(FillValueSpec::I8(0));
        self
    }

    /// Set data type to signed 16-bit integer
    pub fn dtype_i16(mut self) -> Self {
        self.data_type = Some(DataType::Int16);
        self.fill_value = Some(FillValueSpec::I16(0));
        self
    }

    /// Set data type to signed 32-bit integer
    pub fn dtype_i32(mut self) -> Self {
        self.data_type = Some(DataType::Int32);
        self.fill_value = Some(FillValueSpec::I32(0));
        self
    }

    /// Set data type to signed 64-bit integer
    pub fn dtype_i64(mut self) -> Self {
        self.data_type = Some(DataType::Int64);
        self.fill_value = Some(FillValueSpec::I64(0));
        self
    }

    /// Set data type to 32-bit floating point
    pub fn dtype_f32(mut self) -> Self {
        self.data_type = Some(DataType::Float32);
        self.fill_value = Some(FillValueSpec::F32(0.0));
        self
    }

    /// Set data type to 64-bit floating point
    pub fn dtype_f64(mut self) -> Self {
        self.data_type = Some(DataType::Float64);
        self.fill_value = Some(FillValueSpec::F64(0.0));
        self
    }

    /// Add a custom attribute to the array
    ///
    /// # Arguments
    ///
    /// * `key` - Attribute name
    /// * `value` - JSON-serializable value
    pub fn attribute(mut self, key: &str, value: serde_json::Value) -> Self {
        self.attributes.insert(key.to_string(), value);
        self
    }

    /// Build and create the array
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Required fields (name, shape, chunks, dtype) are missing
    /// - Shape and chunks dimensionality mismatch
    /// - Store write fails
    pub async fn build(self) -> Result<()> {
        let name = self.name.ok_or_else(|| anyhow!("Array name is required"))?;
        let shape = self
            .shape
            .ok_or_else(|| anyhow!("Array shape is required"))?;
        let chunks = self
            .chunks
            .ok_or_else(|| anyhow!("Chunk sizes are required"))?;
        let data_type = self
            .data_type
            .ok_or_else(|| anyhow!("Data type is required"))?;
        let fill_value_spec = self
            .fill_value
            .ok_or_else(|| anyhow!("Fill value is required"))?;

        // Validate dimensions match
        if shape.len() != chunks.len() {
            return Err(anyhow!(
                "Shape dimensions ({}) must match chunk dimensions ({})",
                shape.len(),
                chunks.len()
            ));
        }

        // Build attributes with _ARRAY_DIMENSIONS for Xarray compatibility
        let mut attributes = self.attributes;
        if let Some(ref dims) = self.dimensions {
            if dims.len() != shape.len() {
                return Err(anyhow!(
                    "Dimension names ({}) must match shape dimensions ({})",
                    dims.len(),
                    shape.len()
                ));
            }
            attributes.insert("_ARRAY_DIMENSIONS".to_string(), json!(dims));
        }

        // Prepare dimension names for zarrs (Option<Vec<Option<String>>>)
        let dimension_names: Option<Vec<Option<String>>> = self
            .dimensions
            .map(|dims| dims.into_iter().map(Some).collect());

        let store = self.writer.store.clone();
        let array_path = format!("/{}", name);
        let data_type_clone = data_type.clone();
        let array_path_clone = array_path.clone();

        // Create array in blocking context
        tokio::task::spawn_blocking(move || -> Result<()> {
            // Convert fill value to zarrs FillValue
            let fill_value = match fill_value_spec {
                FillValueSpec::U8(v) => FillValue::from(v),
                FillValueSpec::U16(v) => FillValue::from(v),
                FillValueSpec::U32(v) => FillValue::from(v),
                FillValueSpec::U64(v) => FillValue::from(v),
                FillValueSpec::I8(v) => FillValue::from(v),
                FillValueSpec::I16(v) => FillValue::from(v),
                FillValueSpec::I32(v) => FillValue::from(v),
                FillValueSpec::I64(v) => FillValue::from(v),
                FillValueSpec::F32(v) => FillValue::from(v),
                FillValueSpec::F64(v) => FillValue::from(v),
            };

            let mut builder = ArrayBuilder::new(shape, chunks, data_type, fill_value);

            builder.attributes(attributes);

            if let Some(dim_names) = dimension_names {
                builder.dimension_names(Some(dim_names));
            }

            let array = builder
                .build(store, &array_path)
                .map_err(|e| anyhow!("Failed to create array '{}': {}", array_path, e))?;

            // Store array metadata
            array
                .store_metadata()
                .map_err(|e| anyhow!("Failed to store array metadata: {}", e))?;

            Ok(())
        })
        .await??;

        // Register array for future writes
        self.writer
            .register_array(name, array_path_clone, data_type_clone)
            .await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_zarr_store() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("test.zarr");

        let writer = ZarrWriter::new(&store_path).await.unwrap();

        // Verify store directory was created
        assert!(store_path.exists(), "Zarr store directory should exist");

        // Verify zarr.json file at root (Zarr V3 group metadata)
        let zgroup_path = store_path.join("zarr.json");
        assert!(
            zgroup_path.exists(),
            "Root zarr.json (Zarr V3 group) should exist"
        );

        // Verify output path accessor
        assert_eq!(writer.output_path(), store_path);
    }

    #[tokio::test]
    async fn test_write_1d_array() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("1d_test.zarr");

        let writer = ZarrWriter::new(&store_path).await.unwrap();

        // Create 1D array
        writer
            .create_array()
            .name("signal")
            .shape(vec![100])
            .chunks(vec![50])
            .dimensions(vec!["time"])
            .dtype_f64()
            .build()
            .await
            .unwrap();

        // Write first chunk
        let chunk_0: Vec<f64> = (0..50).map(|i| i as f64 * 0.1).collect();
        writer
            .write_chunk::<f64>("signal", &[0], chunk_0)
            .await
            .unwrap();

        // Write second chunk
        let chunk_1: Vec<f64> = (50..100).map(|i| i as f64 * 0.1).collect();
        writer
            .write_chunk::<f64>("signal", &[1], chunk_1)
            .await
            .unwrap();

        // Verify array metadata file exists
        let array_path = store_path.join("signal").join("zarr.json");
        assert!(array_path.exists(), "Array zarr.json should exist");

        // Verify chunk files exist
        let chunk_0_path = store_path.join("signal").join("c").join("0");
        let chunk_1_path = store_path.join("signal").join("c").join("1");
        assert!(chunk_0_path.exists(), "Chunk 0 should exist");
        assert!(chunk_1_path.exists(), "Chunk 1 should exist");
    }

    #[tokio::test]
    async fn test_write_nd_array_with_dimensions() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("nd_test.zarr");

        let writer = ZarrWriter::new(&store_path).await.unwrap();

        // Create 4D array mimicking nested scan
        writer
            .create_array()
            .name("camera_frames")
            .shape(vec![10, 5, 256, 256])
            .chunks(vec![10, 1, 256, 256])
            .dimensions(vec!["wavelength", "position", "y", "x"])
            .dtype_u16()
            .attribute("units", json!("counts"))
            .build()
            .await
            .unwrap();

        // Write a single chunk (wavelength=all, position=0)
        let chunk_data: Vec<u16> = vec![42u16; 10 * 256 * 256];
        writer
            .write_chunk::<u16>("camera_frames", &[0, 0, 0, 0], chunk_data)
            .await
            .unwrap();

        // Verify array metadata exists
        let array_json = store_path.join("camera_frames").join("zarr.json");
        assert!(array_json.exists(), "Array zarr.json should exist");

        // Read and verify _ARRAY_DIMENSIONS is in metadata
        let metadata_content = std::fs::read_to_string(&array_json).unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&metadata_content).unwrap();

        // Check attributes contains _ARRAY_DIMENSIONS
        let attrs = metadata.get("attributes").expect("attributes should exist");
        let array_dims = attrs
            .get("_ARRAY_DIMENSIONS")
            .expect("_ARRAY_DIMENSIONS should exist");
        assert_eq!(
            array_dims,
            &json!(["wavelength", "position", "y", "x"]),
            "_ARRAY_DIMENSIONS should match specified dimensions"
        );

        // Check units attribute
        let units = attrs.get("units").expect("units attribute should exist");
        assert_eq!(units, &json!("counts"));
    }

    #[tokio::test]
    async fn test_chunking_strategy() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("chunk_test.zarr");

        let writer = ZarrWriter::new(&store_path).await.unwrap();

        // Create array with specific chunking
        // Shape: [4, 4] with chunks [2, 2] = 4 chunks total
        writer
            .create_array()
            .name("grid")
            .shape(vec![4, 4])
            .chunks(vec![2, 2])
            .dimensions(vec!["row", "col"])
            .dtype_i32()
            .build()
            .await
            .unwrap();

        // Write all 4 chunks
        // Chunk layout:
        // [0,0] [0,1]
        // [1,0] [1,1]
        let data_00: Vec<i32> = vec![1, 2, 3, 4];
        let data_01: Vec<i32> = vec![5, 6, 7, 8];
        let data_10: Vec<i32> = vec![9, 10, 11, 12];
        let data_11: Vec<i32> = vec![13, 14, 15, 16];

        writer
            .write_chunk::<i32>("grid", &[0, 0], data_00)
            .await
            .unwrap();
        writer
            .write_chunk::<i32>("grid", &[0, 1], data_01)
            .await
            .unwrap();
        writer
            .write_chunk::<i32>("grid", &[1, 0], data_10)
            .await
            .unwrap();
        writer
            .write_chunk::<i32>("grid", &[1, 1], data_11)
            .await
            .unwrap();

        // Verify all chunk files exist at expected paths
        let chunk_dir = store_path.join("grid").join("c");
        assert!(
            chunk_dir.join("0").join("0").exists(),
            "Chunk [0,0] should exist"
        );
        assert!(
            chunk_dir.join("0").join("1").exists(),
            "Chunk [0,1] should exist"
        );
        assert!(
            chunk_dir.join("1").join("0").exists(),
            "Chunk [1,0] should exist"
        );
        assert!(
            chunk_dir.join("1").join("1").exists(),
            "Chunk [1,1] should exist"
        );
    }

    #[tokio::test]
    async fn test_group_attribute() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("attr_test.zarr");

        let writer = ZarrWriter::new(&store_path).await.unwrap();

        // Add experiment metadata
        writer
            .add_group_attribute("experiment_id", json!("EXP-001"))
            .await
            .unwrap();
        writer
            .add_group_attribute("created_at", json!("2026-01-25T12:00:00Z"))
            .await
            .unwrap();

        // Verify root zarr.json contains attributes
        let root_json = store_path.join("zarr.json");
        let metadata_content = std::fs::read_to_string(&root_json).unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&metadata_content).unwrap();

        let attrs = metadata.get("attributes").expect("attributes should exist");
        assert_eq!(
            attrs.get("experiment_id"),
            Some(&json!("EXP-001")),
            "experiment_id attribute should be present"
        );
    }

    #[tokio::test]
    async fn test_validation_errors() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("validation_test.zarr");

        let writer = ZarrWriter::new(&store_path).await.unwrap();

        // Missing name
        let result = writer
            .create_array()
            .shape(vec![10])
            .chunks(vec![10])
            .dtype_f64()
            .build()
            .await;
        assert!(result.is_err(), "Missing name should error");
        assert!(result.unwrap_err().to_string().contains("name"));

        // Shape/chunk mismatch
        let result = writer
            .create_array()
            .name("bad_dims")
            .shape(vec![10, 10])
            .chunks(vec![10]) // Only 1 dimension
            .dtype_f64()
            .build()
            .await;
        assert!(result.is_err(), "Dimension mismatch should error");

        // Dimension names mismatch
        let result = writer
            .create_array()
            .name("bad_names")
            .shape(vec![10, 10])
            .chunks(vec![10, 10])
            .dimensions(vec!["x"]) // Only 1 name for 2 dimensions
            .dtype_f64()
            .build()
            .await;
        assert!(result.is_err(), "Dimension names mismatch should error");
    }

    #[tokio::test]
    async fn test_write_nonexistent_array_error() {
        let temp_dir = TempDir::new().unwrap();
        let store_path = temp_dir.path().join("missing_array.zarr");

        let writer = ZarrWriter::new(&store_path).await.unwrap();

        // Try to write to non-existent array
        let result = writer
            .write_chunk::<f64>("nonexistent", &[0], vec![1.0, 2.0, 3.0])
            .await;
        assert!(result.is_err(), "Writing to nonexistent array should error");
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
