//! Benchmarks for echelle extraction pipeline.
//!
//! Run with:
//! ```bash
//! cargo bench -p common --bench echelle_benchmarks
//! # Specific group:
//! cargo bench -p common --bench echelle_benchmarks -- optimal_extraction
//! ```
//!
//! HTML reports are written to `target/criterion/`.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use common::echelle::{EchelleTraceModel, PolynomialBasis};
use common::echelle_optimal_extraction::{optimal_extract, OptimalExtractionConfig};
use common::echelle_rectification::{rectify_all_orders, rectify_order, OrderSpec, RectifyConfig};
use common::echelle_scattered_light::{subtract_scattered_light, ScatteredLightConfig, TraceInfo};

// ── Synthetic data builders ─────────────────────────────────────────────────

/// Build a flat trace at a given row center.
fn flat_trace(center: f64, width: u32) -> EchelleTraceModel {
    EchelleTraceModel::Polynomial {
        basis: PolynomialBasis::Monomial,
        coefficients: vec![center],
        domain_start: 0.0,
        domain_end: f64::from(width),
    }
}

/// Generate a synthetic echelle frame with `n_orders` Gaussian traces.
fn make_echelle_frame(
    width: u32,
    height: u32,
    n_orders: usize,
) -> (Vec<f32>, Vec<EchelleTraceModel>) {
    let w = width as usize;
    let h = height as usize;
    let mut frame = vec![10.0f32; w * h]; // uniform background

    let order_spacing = h as f64 / (n_orders as f64 + 1.0);
    let sigma = order_spacing * 0.15;
    let peak = 1000.0;

    let mut traces = Vec::with_capacity(n_orders);

    for order_idx in 0..n_orders {
        let center = order_spacing * (order_idx as f64 + 1.0);
        traces.push(flat_trace(center, width));

        for row in 0..h {
            let dist = row as f64 - center;
            let profile = peak * (-0.5 * (dist / sigma).powi(2)).exp();
            for col in 0..w {
                // Add deterministic pseudo-noise via sin for reproducibility.
                let noise = 3.0 * ((col as f64 * 0.73).sin() + (row as f64 * 0.37).cos());
                frame[row * w + col] += (profile + noise) as f32;
            }
        }
    }

    (frame, traces)
}

// ── Benchmark: Order rectification ──────────────────────────────────────────

fn bench_rectify_single_order(c: &mut Criterion) {
    let mut group = c.benchmark_group("rectify_single_order");

    for &size in &[512u32, 1024, 2048, 4096] {
        let n_orders = 10;
        let (frame, traces) = make_echelle_frame(size, size, n_orders);
        let config = RectifyConfig {
            aperture_half_width: 8.0,
            gaussian_weights: false,
            fwhm: 4.0,
        };
        let spec = OrderSpec {
            trace: &traces[n_orders / 2], // middle order
            disp_start: 0,
            disp_end: size - 1,
            order_index: 0,
        };

        group.bench_with_input(BenchmarkId::new("flat_aperture", size), &size, |b, _| {
            b.iter(|| rectify_order(black_box(&frame), size, size, &spec, &config));
        });
    }

    group.finish();
}

fn bench_rectify_all_orders(c: &mut Criterion) {
    let mut group = c.benchmark_group("rectify_all_orders");

    for &size in &[512u32, 1024, 2048] {
        let n_orders = 10;
        let (frame, traces) = make_echelle_frame(size, size, n_orders);
        let config = RectifyConfig {
            aperture_half_width: 8.0,
            gaussian_weights: false,
            fwhm: 4.0,
        };
        let specs: Vec<OrderSpec<'_>> = traces
            .iter()
            .enumerate()
            .map(|(i, t)| OrderSpec {
                trace: t,
                disp_start: 0,
                disp_end: size - 1,
                order_index: i as u32,
            })
            .collect();

        group.bench_with_input(BenchmarkId::new("10_orders", size), &size, |b, _| {
            b.iter(|| rectify_all_orders(black_box(&frame), size, size, &specs, &config));
        });
    }

    group.finish();
}

// ── Benchmark: Scattered light subtraction ──────────────────────────────────

fn bench_scattered_light(c: &mut Criterion) {
    let mut group = c.benchmark_group("scattered_light_subtraction");

    for &size in &[512u32, 1024, 2048] {
        let n_orders = 10;
        let (frame, traces) = make_echelle_frame(size, size, n_orders);
        let trace_infos: Vec<TraceInfo<'_>> = traces
            .iter()
            .map(|t| TraceInfo {
                trace: t,
                disp_start: 0,
                disp_end: size - 1,
            })
            .collect();
        let config = ScatteredLightConfig {
            aperture_half_width: 8.0,
            block_size: 16,
            poly_degree_x: 3,
            poly_degree_y: 3,
        };

        group.bench_with_input(BenchmarkId::new("2d_chebyshev", size), &size, |b, _| {
            b.iter(|| {
                subtract_scattered_light(black_box(&frame), size, size, &trace_infos, &config)
            });
        });
    }

    group.finish();
}

// ── Benchmark: Optimal extraction ───────────────────────────────────────────

fn bench_optimal_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("optimal_extraction");

    for &size in &[512u32, 1024, 2048] {
        let n_orders = 10;
        let (frame, traces) = make_echelle_frame(size, size, n_orders);
        let rect_config = RectifyConfig {
            aperture_half_width: 8.0,
            gaussian_weights: false,
            fwhm: 4.0,
        };
        let spec = OrderSpec {
            trace: &traces[n_orders / 2],
            disp_start: 0,
            disp_end: size - 1,
            order_index: 0,
        };
        let rect = rectify_order(&frame, size, size, &spec, &rect_config)
            .expect("rectification should succeed");

        let opt_config = OptimalExtractionConfig {
            read_noise: 3.0,
            gain: 1.0,
            ..Default::default()
        };

        group.bench_with_input(BenchmarkId::new("single_order", size), &size, |b, _| {
            b.iter(|| optimal_extract(black_box(&rect), None, &opt_config));
        });
    }

    group.finish();
}

// ── Benchmark: Full pipeline (rectify + extract all orders) ─────────────────

fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(10); // Larger frames take longer; reduce iterations.

    for &size in &[512u32, 1024, 2048] {
        let n_orders = 10;
        let (frame, traces) = make_echelle_frame(size, size, n_orders);
        let rect_config = RectifyConfig {
            aperture_half_width: 8.0,
            gaussian_weights: false,
            fwhm: 4.0,
        };
        let specs: Vec<OrderSpec<'_>> = traces
            .iter()
            .enumerate()
            .map(|(i, t)| OrderSpec {
                trace: t,
                disp_start: 0,
                disp_end: size - 1,
                order_index: i as u32,
            })
            .collect();
        let opt_config = OptimalExtractionConfig {
            read_noise: 3.0,
            gain: 1.0,
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new("10_orders_optimal", size),
            &size,
            |b, _| {
                b.iter(|| {
                    let rects =
                        rectify_all_orders(black_box(&frame), size, size, &specs, &rect_config);
                    let mut total_flux = 0.0f64;
                    for rect in rects.iter().flatten() {
                        if let Some(result) = optimal_extract(rect, None, &opt_config) {
                            total_flux += result.flux.iter().sum::<f64>();
                        }
                    }
                    black_box(total_flux)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_rectify_single_order,
    bench_rectify_all_orders,
    bench_scattered_light,
    bench_optimal_extraction,
    bench_full_pipeline,
);
criterion_main!(benches);
