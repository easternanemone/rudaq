//! Comedi Driver Performance Benchmarks
//!
//! Measures performance characteristics of the Comedi driver for NI PCI-MIO-16XE-10.
//!
//! # Running
//!
//! ```bash
//! # Requires hardware feature and real device
//! export COMEDI_DEVICE=/dev/comedi0
//! cargo bench -p driver-comedi --features hardware
//! ```
//!
//! # Benchmark Scenarios
//!
//! - Single-sample latency
//! - Streaming throughput
//! - HAL trait overhead
//! - Multi-channel scaling

#![cfg(feature = "hardware")]

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use driver_comedi::subsystem::AnalogReference;
use driver_comedi::{ComediDevice, Range, StreamAcquisition, StreamConfig};
use std::env;
use std::time::Duration;

/// Get device path from environment or default
fn device_path() -> String {
    env::var("COMEDI_DEVICE").unwrap_or_else(|_| "/dev/comedi0".to_string())
}

/// Benchmark single-sample analog input read latency
fn bench_single_read(c: &mut Criterion) {
    let device = match ComediDevice::open(&device_path()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping benchmark - device not available: {}", e);
            return;
        }
    };

    let ai = device.analog_input().expect("Failed to get AI");

    c.bench_function("ai_single_read_raw", |b| {
        b.iter(|| black_box(ai.read_raw(0, 0, AnalogReference::Ground).unwrap()))
    });

    c.bench_function("ai_single_read_voltage", |b| {
        let range = Range::default();
        b.iter(|| black_box(ai.read_voltage(0, range).unwrap()))
    });
}

/// Benchmark analog output write latency
fn bench_single_write(c: &mut Criterion) {
    let device = match ComediDevice::open(&device_path()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping benchmark - device not available: {}", e);
            return;
        }
    };

    let ao = device.analog_output().expect("Failed to get AO");
    let range = ao.range_info(0, 0).expect("Failed to get range");

    c.bench_function("ao_single_write_voltage", |b| {
        let mut voltage = 0.0;
        b.iter(|| {
            ao.write_voltage(0, black_box(voltage), range).unwrap();
            voltage = (voltage + 0.1) % 5.0;
        })
    });
}

/// Benchmark digital I/O operations
fn bench_dio(c: &mut Criterion) {
    let device = match ComediDevice::open(&device_path()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping benchmark - device not available: {}", e);
            return;
        }
    };

    let dio = match device.digital_io() {
        Ok(d) => d,
        Err(_) => return,
    };

    c.bench_function("dio_read_single", |b| {
        b.iter(|| black_box(dio.read(0).unwrap()))
    });

    c.bench_function("dio_read_port", |b| {
        b.iter(|| black_box(dio.read_port(0).unwrap()))
    });
}

/// Benchmark streaming acquisition throughput
fn bench_streaming_throughput(c: &mut Criterion) {
    let device = match ComediDevice::open(&device_path()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping benchmark - device not available: {}", e);
            return;
        }
    };

    let mut group = c.benchmark_group("streaming");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(20);

    // Test different channel counts
    for n_channels in [1, 2, 4, 8] {
        let channels: Vec<u32> = (0..n_channels).collect();
        let sample_rate = 10000.0 / n_channels as f64;

        let config = match StreamConfig::builder()
            .channels(&channels)
            .sample_rate(sample_rate)
            .buffer_size(4096)
            .build()
        {
            Ok(c) => c,
            Err(_) => continue,
        };

        let stream = match StreamAcquisition::new(&device, config) {
            Ok(s) => s,
            Err(_) => continue,
        };

        group.throughput(Throughput::Elements(1000));
        group.bench_function(format!("{}_channel_stream", n_channels), |b| {
            stream.start().unwrap();

            b.iter(|| {
                if let Ok(Some(samples)) = stream.read_available() {
                    black_box(samples.len());
                }
            });

            stream.stop().unwrap();
        });
    }

    group.finish();
}

/// Benchmark counter operations
fn bench_counter(c: &mut Criterion) {
    let device = match ComediDevice::open(&device_path()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping benchmark - device not available: {}", e);
            return;
        }
    };

    let counter = match device.counter() {
        Ok(c) => c,
        Err(_) => return,
    };

    c.bench_function("counter_read", |b| {
        b.iter(|| black_box(counter.read(0).unwrap()))
    });

    c.bench_function("counter_reset", |b| b.iter(|| counter.reset(0).unwrap()));
}

/// Benchmark device open/close cycle (resource management)
fn bench_device_lifecycle(c: &mut Criterion) {
    c.bench_function("device_open_close", |b| {
        b.iter(|| {
            let device = ComediDevice::open(&device_path()).unwrap();
            black_box(&device);
            drop(device);
        })
    });
}

criterion_group!(
    benches,
    bench_single_read,
    bench_single_write,
    bench_dio,
    bench_streaming_throughput,
    bench_counter,
    bench_device_lifecycle,
);
criterion_main!(benches);
