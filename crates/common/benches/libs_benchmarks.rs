//! Benchmarks for LIBS signal-processing algorithms.
//!
//! Run with:
//! ```bash
//! cargo bench -p common --bench libs_benchmarks
//! # Specific group:
//! cargo bench -p common --bench libs_benchmarks -- radiance
//! ```
//!
//! HTML reports are written to `target/criterion/`.

use std::collections::HashMap;

use common::processing::radiance_calibration::{RadianceCalibrator, Spectrum, linear_interp};

// Re-export the standalone libs spectrum generator for use in benchmarks.
// (It lives in driver-andor-sdk3 but we define a local equivalent here to
//  avoid adding a bench dep on the camera crate.)
//
// This function mirrors `driver_andor_sdk3::mock::generate_libs_spectrum`.
fn bench_generate_libs_spectrum(num_pixels: usize) -> Vec<f64> {
    let mut spectrum = vec![0.0f64; num_pixels];
    let delay_us = 1.3_f64;
    let continuum_amp = 8000.0 * (-delay_us).exp();
    for v in &mut spectrum {
        *v += continuum_amp;
    }
    let gain_scale = 3600.0_f64 / 4095.0;
    #[allow(clippy::cast_precision_loss)] // benchmark: pixel counts fit well within f64 range
    let scale = num_pixels as f64 / 2560.0;
    let lines: &[(f64, f64, f64)] = &[
        (530.0 * scale, 50_000.0, 2.5),
        (674.0 * scale, 50_000.0, 2.5),
        (1240.0 * scale, 45_000.0, 2.5),
        (1253.0 * scale, 40_000.0, 2.5),
    ];
    for &(center, amp, sigma) in lines {
        for (i, v) in spectrum.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)] // benchmark: loop index, precision acceptable
            let dx = i as f64 - center;
            *v += amp * gain_scale * (-0.5 * (dx / sigma).powi(2)).exp();
        }
    }
    spectrum
}

fn make_calibrator(num_points: usize) -> RadianceCalibrator {
    #[allow(clippy::cast_precision_loss)]
    // benchmark: wavelength axis, sub-pixel precision acceptable
    let wl: Vec<f64> = (0..num_points)
        .map(|i| 200.0 + i as f64 * 400.0 / num_points as f64)
        .collect();
    let lamp = Spectrum::new(wl.clone(), vec![1000.0; num_points]).unwrap();
    let cal = Spectrum::new(wl, vec![800.0; num_points]).unwrap();
    let mut cals = HashMap::new();
    cals.insert(1u8, cal);
    RadianceCalibrator::new(lamp, cals)
}

fn make_spectrum(num_pixels: usize) -> Spectrum {
    #[allow(clippy::cast_precision_loss)]
    // benchmark: wavelength axis, sub-pixel precision acceptable
    let wl: Vec<f64> = (0..num_pixels)
        .map(|i| 200.0 + i as f64 * 400.0 / num_pixels as f64)
        .collect();
    let vals = bench_generate_libs_spectrum(num_pixels);
    Spectrum::new(wl, vals).unwrap()
}

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

// ── Benchmark groups ─────────────────────────────────────────────────────────

fn bench_radiance_calibration(c: &mut Criterion) {
    let mut group = c.benchmark_group("radiance_calibration");

    for &n in &[256usize, 512, 1024, 2560] {
        let calibrator = make_calibrator(n);
        let spectrum = make_spectrum(n);

        group.bench_with_input(BenchmarkId::new("calibrate_no_normalize", n), &n, |b, _| {
            b.iter(|| {
                calibrator
                    .calibrate(black_box(&spectrum), 1, false)
                    .unwrap()
            });
        });

        group.bench_with_input(BenchmarkId::new("calibrate_normalize", n), &n, |b, _| {
            b.iter(|| calibrator.calibrate(black_box(&spectrum), 1, true).unwrap());
        });
    }

    group.finish();
}

fn bench_linear_interp(c: &mut Criterion) {
    let mut group = c.benchmark_group("linear_interp");

    // Simulate interpolating a 256-point calibration file onto a 2560-point axis
    let src_x: Vec<f64> = (0..256).map(|i| f64::from(i) * 1.5625).collect();
    let src_y: Vec<f64> = src_x.iter().map(|x| x.sin() * 1000.0).collect();
    let target_x: Vec<f64> = (0..2560).map(|i| f64::from(i) * 0.15625).collect();

    group.bench_function("256→2560", |b| {
        b.iter(|| linear_interp(black_box(&src_x), black_box(&src_y), black_box(&target_x)));
    });

    group.finish();
}

fn bench_spectrum_normalize(c: &mut Criterion) {
    let mut group = c.benchmark_group("spectrum_normalize");

    for &n in &[512usize, 2560] {
        let spectrum = make_spectrum(n);
        group.bench_with_input(BenchmarkId::new("normalize", n), &n, |b, _| {
            b.iter(|| black_box(&spectrum).normalize());
        });
    }

    group.finish();
}

fn bench_libs_spectrum_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("libs_spectrum_generation");

    for &n in &[512usize, 1024, 2560] {
        group.bench_with_input(
            BenchmarkId::new("generate_libs_spectrum", n),
            &n,
            |b, &n| {
                b.iter(|| bench_generate_libs_spectrum(black_box(n)));
            },
        );
    }

    group.finish();
}

fn bench_step_scan_simulation(c: &mut Criterion) {
    // Full processing pipeline for a step scan: generate + calibrate + normalize
    let calibrator = make_calibrator(2560);

    c.bench_function("step_scan_10_point_pipeline", |b| {
        b.iter(|| {
            let results: Vec<_> = (0..10)
                .map(|_i| {
                    let spectrum = make_spectrum(2560);
                    calibrator.calibrate(&spectrum, 1, true).unwrap()
                })
                .collect();
            black_box(results)
        });
    });
}

criterion_group!(
    benches,
    bench_radiance_calibration,
    bench_linear_interp,
    bench_spectrum_normalize,
    bench_libs_spectrum_generation,
    bench_step_scan_simulation,
);
criterion_main!(benches);
