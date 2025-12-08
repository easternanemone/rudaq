use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

use rust_daq::error::DaqError;
use rust_daq::parameter::Parameter;

fn bench_set_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("parameter_set");

    // f64
    group.bench_function("set_f64", |b| {
        let rt = Runtime::new().expect("tokio runtime");
        let param = Parameter::new("p_f64", 0.0f64);
        let counter = AtomicU64::new(0);
        b.to_async(&rt).iter(|| async {
            let next = counter.fetch_add(1, Ordering::SeqCst) as f64;
            param.set(black_box(next)).await.unwrap();
        });
    });

    // bool
    group.bench_function("set_bool", |b| {
        let rt = Runtime::new().expect("tokio runtime");
        let param = Parameter::new("p_bool", false);
        let toggle = AtomicBool::new(false);
        b.to_async(&rt).iter(|| async {
            let next = toggle.fetch_xor(true, Ordering::SeqCst);
            param.set(black_box(!next)).await.unwrap();
        });
    });

    // String
    group.bench_function("set_string", |b| {
        let rt = Runtime::new().expect("tokio runtime");
        let param = Parameter::new("p_str", String::from("off"));
        let toggle = AtomicBool::new(false);
        b.to_async(&rt).iter(|| async {
            let next_state = toggle.fetch_xor(true, Ordering::SeqCst);
            let next = if !next_state { "on" } else { "off" };
            param.set(black_box(next.to_string())).await.unwrap();
        });
    });

    group.finish();
}

fn bench_get_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("parameter_get");

    group.bench_function("get_f64", |b| {
        let param = Parameter::new("p_f64_get", 42.0f64);
        b.iter(|| black_box(param.get()));
    });

    group.bench_function("get_bool", |b| {
        let param = Parameter::new("p_bool_get", true);
        b.iter(|| black_box(param.get()));
    });

    group.bench_function("get_string", |b| {
        let param = Parameter::new("p_str_get", String::from("ready"));
        b.iter(|| black_box(param.get()));
    });

    group.finish();
}

fn bench_async_hardware_callback(c: &mut Criterion) {
    let mut group = c.benchmark_group("parameter_set_hw_callback");

    group.bench_function("set_with_callback", |b| {
        let rt = Runtime::new().expect("tokio runtime");
        let mut param = Parameter::new("p_hw", 0u64);
        let counter = std::sync::Arc::new(AtomicUsize::new(0));

        param.connect_to_hardware_write({
            let counter = counter.clone();
            move |val| {
                let counter = counter.clone();
                Box::pin(async move {
                    counter.fetch_add(val as usize, Ordering::SeqCst);
                    Ok::<(), DaqError>(())
                })
            }
        });

        let val_counter = AtomicU64::new(0);
        b.to_async(&rt).iter(|| async {
            let next = val_counter.fetch_add(1, Ordering::SeqCst) + 1;
            param.set(black_box(next)).await.unwrap();
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_set_latency,
    bench_get_latency,
    bench_async_hardware_callback
);
criterion_main!(benches);
