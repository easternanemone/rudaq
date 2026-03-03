// =============================================================================
// Arrow Writer Integration Tests
// =============================================================================

use super::helpers::*;
use arrow::ipc::reader::FileReader;
use arrow::ipc::writer::FileWriter;
use common::core::Measurement;
use std::fs::File;
use tempfile::TempDir;

#[tokio::test]
async fn test_arrow_write_scalars() {
    let temp_dir = TempDir::new().unwrap();
    let arrow_path = temp_dir.path().join("test_scalars.arrow");

    let measurements = vec![
        create_test_scalar("power", 100.0),
        create_test_scalar("voltage", 5.0),
        create_test_scalar("current", 0.5),
    ];

    let batches = Measurement::into_arrow_batches(&measurements).unwrap();
    assert!(batches.scalars.is_some(), "Should have scalar batch");

    // Write to Arrow IPC file
    let file = File::create(&arrow_path).unwrap();
    let batch = batches.scalars.unwrap();
    let mut writer = FileWriter::try_new(file, &batch.schema()).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    // Verify file was created
    assert!(arrow_path.exists(), "Arrow file should exist");

    // Verify we can read it back
    let file = File::open(&arrow_path).unwrap();
    let reader = FileReader::try_new(file, None).unwrap();
    assert_eq!(reader.schema().fields().len(), 4, "Should have 4 fields");
}

#[tokio::test]
async fn test_arrow_write_vectors() {
    let temp_dir = TempDir::new().unwrap();
    let arrow_path = temp_dir.path().join("test_vectors.arrow");

    let measurements = vec![
        create_test_vector("waveform_1", vec![1.0, 2.0, 3.0]),
        create_test_vector("waveform_2", vec![4.0, 5.0, 6.0]),
    ];

    let batches = Measurement::into_arrow_batches(&measurements).unwrap();
    assert!(batches.vectors.is_some(), "Should have vector batch");

    let file = File::create(&arrow_path).unwrap();
    let batch = batches.vectors.unwrap();
    let mut writer = FileWriter::try_new(file, &batch.schema()).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    assert!(arrow_path.exists(), "Arrow file should exist");
}

#[tokio::test]
async fn test_arrow_write_spectra() {
    let temp_dir = TempDir::new().unwrap();
    let arrow_path = temp_dir.path().join("test_spectra.arrow");

    let measurements = vec![
        create_test_spectrum("fft_1", 128),
        create_test_spectrum("fft_2", 256),
    ];

    let batches = Measurement::into_arrow_batches(&measurements).unwrap();
    assert!(batches.spectra.is_some(), "Should have spectra batch");

    let file = File::create(&arrow_path).unwrap();
    let batch = batches.spectra.unwrap();
    let mut writer = FileWriter::try_new(file, &batch.schema()).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    assert!(arrow_path.exists(), "Arrow file should exist");
}

#[tokio::test]
async fn test_arrow_metadata_in_schema() {
    let measurements = vec![create_test_scalar("test", 42.0)];

    let batches = Measurement::into_arrow_batches(&measurements).unwrap();
    let batch = batches.scalars.unwrap();

    // Check schema fields
    let schema = batch.schema();
    assert_eq!(schema.fields().len(), 4, "Should have 4 fields");

    // Verify field names
    assert_eq!(schema.field(0).name(), "name");
    assert_eq!(schema.field(1).name(), "value");
    assert_eq!(schema.field(2).name(), "unit");
    assert_eq!(schema.field(3).name(), "timestamp_ns");
}

#[tokio::test]
async fn test_arrow_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let arrow_path = temp_dir.path().join("test_roundtrip.arrow");

    // Create test data
    let original_measurements = vec![
        create_test_scalar("power", 100.0),
        create_test_scalar("voltage", 5.0),
    ];

    // Write to Arrow
    let batches = Measurement::into_arrow_batches(&original_measurements).unwrap();
    let file = File::create(&arrow_path).unwrap();
    let batch = batches.scalars.unwrap();
    let mut writer = FileWriter::try_new(file, &batch.schema()).unwrap();
    writer.write(&batch).unwrap();
    writer.finish().unwrap();

    // Read back from Arrow
    let file = File::open(&arrow_path).unwrap();
    let mut reader = FileReader::try_new(file, None).unwrap();
    let read_batch = reader.next().unwrap().unwrap();

    // Verify data integrity
    assert_eq!(read_batch.num_rows(), 2, "Should have 2 rows");
    assert_eq!(read_batch.num_columns(), 4, "Should have 4 columns");

    // Check values
    use arrow::array::Float64Array;
    let value_column = read_batch
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();

    assert_eq!(value_column.value(0), 100.0, "First value should match");
    assert_eq!(value_column.value(1), 5.0, "Second value should match");
}

#[tokio::test]
async fn test_arrow_mixed_types() {
    let measurements = vec![
        create_test_scalar("power", 100.0),
        create_test_vector("waveform", vec![1.0, 2.0, 3.0]),
        create_test_spectrum("fft", 64),
    ];

    let batches = Measurement::into_arrow_batches(&measurements).unwrap();

    // Verify all types are represented
    assert!(batches.scalars.is_some(), "Should have scalars");
    assert!(batches.vectors.is_some(), "Should have vectors");
    assert!(batches.spectra.is_some(), "Should have spectra");

    // Verify counts
    assert_eq!(batches.scalars.unwrap().num_rows(), 1);
    assert_eq!(batches.vectors.unwrap().num_rows(), 1);
    assert_eq!(batches.spectra.unwrap().num_rows(), 1);
}
