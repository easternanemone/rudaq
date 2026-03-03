//! Criterion benchmarks comparing Arrow tensor serialization strategies.
//!
//! Run with:
//!   cargo bench -p storage --features storage_arrow --bench arrow_tensor
//!
//! Benchmarks:
//!   - 2D tensor (480x640 f64 image) write through ArrowDocumentWriter
//!   - 3D tensor (10x480x640 f64 image stack) write through ArrowDocumentWriter
//!   - Scalar-only baseline for comparison

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::HashMap;

use common::experiment::document::{DataKey, DescriptorDoc, Document, EventDoc, StartDoc, StopDoc};
use storage::ArrowDocumentWriter;

/// Helper: create a full run (start -> descriptor -> N events -> stop) through the writer.
async fn write_run(
    writer: &ArrowDocumentWriter,
    run_id: &str,
    data_keys: HashMap<String, DataKey>,
    events: Vec<EventDoc>,
) {
    let start = StartDoc {
        uid: run_id.to_string(),
        time_ns: 1000,
        plan_type: "bench".to_string(),
        plan_name: "Bench".to_string(),
        plan_args: HashMap::new(),
        metadata: HashMap::new(),
        hints: vec![],
    };
    writer
        .write(Document::Start(start))
        .await
        .expect("write start");

    let descriptor = DescriptorDoc {
        run_uid: run_id.to_string(),
        uid: format!("{run_id}_desc"),
        name: "primary".to_string(),
        data_keys,
        configuration: HashMap::new(),
        time_ns: 0,
    };
    writer
        .write(Document::Descriptor(descriptor))
        .await
        .expect("write descriptor");

    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: test/benchmark values are bounded
    let num_events = events.len() as u32;
    for event in events {
        writer
            .write(Document::Event(event))
            .await
            .expect("write event");
    }

    let stop = StopDoc {
        uid: format!("{run_id}_stop"),
        run_uid: run_id.to_string(),
        time_ns: 2_000_000_000,
        exit_status: "success".to_string(),
        reason: String::new(),
        num_events,
    };
    writer
        .write(Document::Stop(stop))
        .await
        .expect("write stop");
}

fn bench_scalar_baseline(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let temp_dir = tempfile::TempDir::new().expect("temp dir");

    let mut group = c.benchmark_group("arrow_write");
    group.throughput(Throughput::Elements(10));

    group.bench_function("scalar_only_10_events", |b| {
        let mut run_counter = 0u64;
        b.iter(|| {
            run_counter += 1;
            let run_id = format!("scalar_{run_counter}");
            let writer = ArrowDocumentWriter::new(temp_dir.path().to_path_buf());

            let mut data_keys = HashMap::new();
            data_keys.insert(
                "det1".to_string(),
                DataKey {
                    source: "det1".to_string(),
                    dtype: "number".to_string(),
                    shape: vec![],
                    units: String::new(),
                    precision: None,
                    lower_limit: None,
                    upper_limit: None,
                },
            );

            let events: Vec<EventDoc> = (0..10)
                .map(|i| {
                    let mut data = HashMap::new();
                    data.insert("det1".to_string(), f64::from(i) * 1.5);
                    EventDoc {
                        descriptor_uid: format!("{run_id}_desc"),
                        seq_num: i,
                        data,
                        arrays: HashMap::new(),
                        timestamps: HashMap::new(),
                        metadata: HashMap::new(),
                        run_uid: run_id.clone(),
                        time_ns: 1_000_000_000 + u64::from(i) * 100_000,
                        uid: format!("ev_{run_counter}_{i}"),
                        positions: HashMap::new(),
                    }
                })
                .collect();

            rt.block_on(write_run(&writer, &run_id, data_keys, events));
        });
    });

    group.finish();
}

fn bench_tensor_2d(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let temp_dir = tempfile::TempDir::new().expect("temp dir");

    let height = 480i32;
    let width = 640i32;
    #[allow(clippy::cast_sign_loss)]
    // SAFETY: test/benchmark values are bounded
    let num_elements = (height * width) as usize;
    let bytes_per_image = num_elements * 8; // f64

    let mut group = c.benchmark_group("arrow_tensor_2d");
    group.throughput(Throughput::Bytes((bytes_per_image * 10) as u64));

    // Pre-generate image data for consistency
    #[allow(clippy::cast_precision_loss)]
    // SAFETY: test/benchmark values are bounded
    let image_data: Vec<f64> = (0..num_elements).map(|i| i as f64 * 0.001).collect();
    let raw_bytes: Vec<u8> = image_data.iter().flat_map(|v| v.to_le_bytes()).collect();
    let shared_bytes = Bytes::from(raw_bytes);

    group.bench_with_input(
        BenchmarkId::new("480x640_f64", "10_events"),
        &shared_bytes,
        |b, image_bytes| {
            let mut run_counter = 0u64;
            b.iter(|| {
                run_counter += 1;
                let run_id = format!("t2d_{run_counter}");
                let writer = ArrowDocumentWriter::new(temp_dir.path().to_path_buf());

                let mut data_keys = HashMap::new();
                data_keys.insert(
                    "image".to_string(),
                    DataKey {
                        source: "cam".to_string(),
                        dtype: "float64".to_string(),
                        shape: vec![height, width],
                        units: String::new(),
                        precision: None,
                        lower_limit: None,
                        upper_limit: None,
                    },
                );

                let events: Vec<EventDoc> = (0..10)
                    .map(|i| {
                        let mut arrays = HashMap::new();
                        arrays.insert("image".to_string(), image_bytes.clone());
                        EventDoc {
                            descriptor_uid: format!("{run_id}_desc"),
                            seq_num: i,
                            data: HashMap::new(),
                            arrays,
                            timestamps: HashMap::new(),
                            metadata: HashMap::new(),
                            run_uid: run_id.clone(),
                            time_ns: 1_000_000_000 + u64::from(i) * 100_000,
                            uid: format!("ev_{run_counter}_{i}"),
                            positions: HashMap::new(),
                        }
                    })
                    .collect();

                rt.block_on(write_run(&writer, &run_id, data_keys, events));
            });
        },
    );

    group.finish();
}

fn bench_tensor_3d(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let temp_dir = tempfile::TempDir::new().expect("temp dir");

    let depth = 10i32;
    let height = 480i32;
    let width = 640i32;
    #[allow(clippy::cast_sign_loss)]
    // SAFETY: test/benchmark values are bounded
    let num_elements = (depth * height * width) as usize;
    let bytes_per_cube = num_elements * 8; // f64

    let mut group = c.benchmark_group("arrow_tensor_3d");
    // Single event with a 3D cube (large data)
    group.throughput(Throughput::Bytes(bytes_per_cube as u64));
    group.sample_size(10); // Fewer samples for large data

    #[allow(clippy::cast_precision_loss)]
    // SAFETY: test/benchmark values are bounded
    let cube_data: Vec<f64> = (0..num_elements).map(|i| i as f64 * 0.0001).collect();
    let raw_bytes: Vec<u8> = cube_data.iter().flat_map(|v| v.to_le_bytes()).collect();
    let shared_bytes = Bytes::from(raw_bytes);

    group.bench_with_input(
        BenchmarkId::new("10x480x640_f64", "1_event"),
        &shared_bytes,
        |b, cube_bytes| {
            let mut run_counter = 0u64;
            b.iter(|| {
                run_counter += 1;
                let run_id = format!("t3d_{run_counter}");
                let writer = ArrowDocumentWriter::new(temp_dir.path().to_path_buf());

                let mut data_keys = HashMap::new();
                data_keys.insert(
                    "cube".to_string(),
                    DataKey {
                        source: "spec".to_string(),
                        dtype: "float64".to_string(),
                        shape: vec![depth, height, width],
                        units: String::new(),
                        precision: None,
                        lower_limit: None,
                        upper_limit: None,
                    },
                );

                let mut arrays = HashMap::new();
                arrays.insert("cube".to_string(), cube_bytes.clone());
                let events = vec![EventDoc {
                    descriptor_uid: format!("{run_id}_desc"),
                    seq_num: 0,
                    data: HashMap::new(),
                    arrays,
                    timestamps: HashMap::new(),
                    metadata: HashMap::new(),
                    run_uid: run_id.clone(),
                    time_ns: 1_000_000_000,
                    uid: format!("ev_{run_counter}_0"),
                    positions: HashMap::new(),
                }];

                rt.block_on(write_run(&writer, &run_id, data_keys, events));
            });
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    bench_scalar_baseline,
    bench_tensor_2d,
    bench_tensor_3d
);
criterion_main!(benches);
