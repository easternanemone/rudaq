use std::sync::Once;

use chrono::{Duration, Utc};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3::PyCell;
use rust_daq::core::DataPoint;
use rust_daq_py::PyDataPoint;
use serde_json::json;

static PY_INIT: Once = Once::new();

fn ensure_python_initialized() {
    PY_INIT.call_once(|| unsafe {
        if pyo3::ffi::Py_IsInitialized() == 0 {
            pyo3::ffi::Py_Initialize();
        }
    });
}

fn base_datapoint() -> DataPoint {
    DataPoint {
        timestamp: Utc::now(),
        channel: "ch1".to_string(),
        value: 1.2345,
        unit: "V".to_string(),
        metadata: Some(json!({ "source": "bench", "sequence": 0 })),
    }
}

fn make_data_points(count: usize) -> Vec<DataPoint> {
    let mut base = base_datapoint();
    (0..count)
        .map(|idx| {
            base.timestamp = base.timestamp + Duration::microseconds(idx as i64);
            let mut dp = base.clone();
            dp.channel = format!("ch{}", idx % 4);
            dp.value = idx as f64 * 0.01;
            if let Some(meta) = &mut dp.metadata {
                meta["sequence"] = json!(idx);
            }
            dp
        })
        .collect()
}

fn benchmark_datapoint_conversion(c: &mut Criterion) {
    let template = base_datapoint();
    c.bench_function("datapoint_rust_to_py_struct", |b| {
        b.iter(|| {
            let py_dp = PyDataPoint::from(template.clone());
            black_box(py_dp);
        });
    });
}

fn benchmark_python_roundtrip(c: &mut Criterion) {
    ensure_python_initialized();
    let template = base_datapoint();
    c.bench_function("datapoint_roundtrip_rust_python_rust", |b| {
        b.iter(|| {
            Python::with_gil(|py| {
                let py_obj = Py::new(py, PyDataPoint::from(template.clone())).expect("create py datapoint");
                let py_cell = py_obj.as_ref(py);
                let borrowed = py_cell.borrow();
                let dp_back = DataPoint {
                    timestamp: borrowed.timestamp,
                    channel: borrowed.channel.clone(),
                    value: borrowed.value,
                    unit: borrowed.unit.clone(),
                    metadata: borrowed.metadata.clone(),
                };
                black_box(dp_back);
            });
        });
    });
}

fn benchmark_batch_roundtrip(c: &mut Criterion) {
    ensure_python_initialized();
    let mut group = c.benchmark_group("batch_roundtrip");
    for &size in &[1usize, 100, 1_000] {
        group.throughput(criterion::Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &count| {
            b.iter_batched(
                || make_data_points(count),
                |points| {
                    Python::with_gil(|py| {
                        let py_list = PyList::new(
                            py,
                            points
                                .iter()
                                .cloned()
                                .map(PyDataPoint::from)
                                .collect::<Vec<_>>(),
                        );
                        let back: Vec<DataPoint> = py_list
                            .iter()
                            .map(|item| {
                                let cell: &PyCell<PyDataPoint> = item.downcast().expect("PyDataPoint downcast");
                                let borrow = cell.borrow();
                                DataPoint {
                                    timestamp: borrow.timestamp,
                                    channel: borrow.channel.clone(),
                                    value: borrow.value,
                                    unit: borrow.unit.clone(),
                                    metadata: borrow.metadata.clone(),
                                }
                            })
                            .collect();
                        black_box(back);
                    });
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn criterion_benchmark(c: &mut Criterion) {
    benchmark_datapoint_conversion(c);
    benchmark_python_roundtrip(c);
    benchmark_batch_roundtrip(c);
}

criterion_group!(name = benches; config = Criterion::default().sample_size(50); targets = criterion_benchmark);
criterion_main!(benches);
