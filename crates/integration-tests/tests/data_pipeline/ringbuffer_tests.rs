// =============================================================================
// Ring Buffer Integration Tests
// =============================================================================

use std::sync::Arc;
use storage::ring_buffer::RingBuffer;
use tempfile::TempDir;

#[tokio::test]
async fn test_ringbuffer_basic_operations() {
    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("test_ring.buf");

    let ring = RingBuffer::create(&ring_path, 1).unwrap();

    // Write test data
    let test_data = b"Hello, ring buffer!";
    ring.write(test_data).unwrap();

    // Read back
    let snapshot = ring.read_snapshot();
    assert_eq!(snapshot, test_data, "Data should match");

    // Verify positions
    assert_eq!(ring.write_head(), test_data.len() as u64);
    assert_eq!(ring.read_tail(), 0);

    // Advance tail
    ring.advance_tail(snapshot.len() as u64);
    assert_eq!(ring.read_tail(), test_data.len() as u64);
}

#[cfg(feature = "storage_arrow")]
#[tokio::test]
async fn test_ringbuffer_arrow_integration() {
    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("test_ring_arrow.buf");

    let ring = RingBuffer::create(&ring_path, 10).unwrap();

    // Create Arrow batch
    use arrow::array::{Float64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc as StdArc;

    let schema = Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]);

    let names = StringArray::from(vec!["power", "voltage", "current"]);
    let values = Float64Array::from(vec![100.0, 5.0, 0.5]);

    let batch = RecordBatch::try_new(
        StdArc::new(schema),
        vec![StdArc::new(names), StdArc::new(values)],
    )
    .unwrap();

    // Write to ring buffer
    ring.write_arrow_batch(&batch).unwrap();

    // Verify data was written
    assert!(ring.write_head() > 0, "Data should be written");

    // Read snapshot
    let snapshot = ring.read_snapshot();
    assert!(!snapshot.is_empty(), "Snapshot should not be empty");
}

#[tokio::test]
async fn test_ringbuffer_circular_wrap() {
    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("test_ring_wrap.buf");

    // Create small buffer to force wrapping
    let ring = RingBuffer::create(&ring_path, 1).unwrap(); // 1 MB
    let capacity = ring.capacity();

    // Write data that exceeds capacity
    let chunk_size = 512 * 1024; // 512 KB
    let test_data = vec![0xAA_u8; chunk_size];

    // Write 3 chunks (1.5 MB total, exceeds 1 MB capacity)
    for _ in 0..3 {
        ring.write(&test_data).unwrap();
    }

    // Verify buffer wrapped correctly
    let snapshot = ring.read_snapshot();
    assert!(
        snapshot.len() as u64 <= capacity,
        "Snapshot should not exceed capacity"
    );
}

#[tokio::test]
async fn test_ringbuffer_concurrent_access() {
    let temp_dir = TempDir::new().unwrap();
    let ring_path = temp_dir.path().join("test_ring_concurrent.buf");

    let ring = Arc::new(RingBuffer::create(&ring_path, 10).unwrap());

    // Spawn writer task
    let ring_writer = ring.clone();
    let writer = tokio::spawn(async move {
        for i in 0..100 {
            let data = format!("Message {i}");
            ring_writer.write(data.as_bytes()).unwrap();
            tokio::time::sleep(tokio::time::Duration::from_micros(100)).await;
        }
    });

    // Spawn reader task
    let ring_reader = ring.clone();
    let reader = tokio::spawn(async move {
        let mut read_count = 0;
        while read_count < 50 {
            let snapshot = ring_reader.read_snapshot();
            if !snapshot.is_empty() {
                read_count += 1;
                ring_reader.advance_tail(snapshot.len() as u64);
            }
            tokio::time::sleep(tokio::time::Duration::from_micros(500)).await;
        }
    });

    // Wait for both tasks
    writer.await.unwrap();
    reader.await.unwrap();
}
