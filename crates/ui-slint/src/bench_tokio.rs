//! Tokio integration benchmark for Slint.
//!
//! Spawns a Tokio runtime that pushes simulated camera frames over a channel
//! into the Slint event loop. Measures achieved FPS to quantify async overhead
//! vs the pure-Slint 560 fps baseline.

mod panels;

use panels::camera_panel::{self, FpsTracker, FrameData};
use panels::plot_panel::{self, DataPoint, PlotState};
use rand::Rng;
use slint::{Rgb8Pixel, SharedPixelBuffer, Timer, TimerMode};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

const CAMERA_W: u32 = 512;
const CAMERA_H: u32 = 512;
const TRACE_POINTS_PER_FRAME: usize = 8;
const BENCH_DURATION_SECS: f64 = 5.0;

slint::slint! {
    export component BenchWindow inherits Window {
        title: "Slint + Tokio Integration Benchmark";
        preferred-width: 1100px;
        preferred-height: 650px;

        in property <image> camera-image;
        in property <image> plot-image;
        in property <string> status-text: "Starting Tokio runtime...";
        in property <string> camera-fps: "";
        in property <string> plot-info: "";

        VerticalLayout {
            padding: 12px;
            spacing: 8px;

            Text {
                text: "Tokio → Slint Channel Benchmark";
                font-size: 18px;
                horizontal-alignment: center;
            }

            HorizontalLayout {
                spacing: 12px;

                VerticalLayout {
                    spacing: 4px;
                    Text { text: "Camera (512x512 u16)"; font-size: 13px; horizontal-alignment: center; }
                    Image {
                        source: camera-image;
                        preferred-width: 512px;
                        preferred-height: 512px;
                        image-fit: contain;
                    }
                    Text { text: camera-fps; font-size: 12px; horizontal-alignment: center; }
                }

                VerticalLayout {
                    spacing: 4px;
                    Text { text: "Live Signal Plot"; font-size: 13px; horizontal-alignment: center; }
                    Image {
                        source: plot-image;
                        preferred-width: 512px;
                        preferred-height: 512px;
                        image-fit: contain;
                    }
                    Text { text: plot-info; font-size: 12px; horizontal-alignment: center; }
                }
            }

            Text {
                text: status-text;
                font-size: 14px;
                horizontal-alignment: center;
            }
        }
    }
}

fn generate_frame(frame_num: u64) -> FrameData {
    let mut rng = rand::thread_rng();
    let t = frame_num as f64 * 0.02;
    let mut data = Vec::with_capacity((CAMERA_W * CAMERA_H) as usize);
    for y in 0..CAMERA_H {
        for x in 0..CAMERA_W {
            let fx = x as f64 / CAMERA_W as f64;
            let fy = y as f64 / CAMERA_H as f64;
            let signal = ((fx * 4.0 * std::f64::consts::PI + t).sin()
                * (fy * 3.0 * std::f64::consts::PI + t * 0.7).cos()
                + 1.0)
                * 0.5;
            let noise = rng.gen::<f64>() * 0.05;
            let val = ((signal + noise) * 65535.0).clamp(0.0, 65535.0) as u16;
            data.push(val);
        }
    }
    FrameData {
        width: CAMERA_W,
        height: CAMERA_H,
        data,
        frame_number: frame_num,
    }
}

fn main() {
    let window = BenchWindow::new().expect("Failed to create window");

    // Channels
    let (frame_tx, frame_rx) = camera_panel::frame_channel(4);
    let (data_tx, data_rx) = plot_panel::data_channel(1000);

    // Shared state for Tokio producer
    let running = Arc::new(AtomicBool::new(true));
    let produced_frames = Arc::new(AtomicU64::new(0));

    // Spawn Tokio runtime in background thread
    let running_clone = running.clone();
    let produced_clone = produced_frames.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to build Tokio runtime");

        rt.block_on(async move {
            let mut frame_num = 0u64;
            while running_clone.load(Ordering::Relaxed) {
                frame_num += 1;

                // Generate and send camera frame
                let frame = generate_frame(frame_num);
                if frame_tx.try_send(frame).is_ok() {
                    produced_clone.fetch_add(1, Ordering::Relaxed);
                }

                // Generate and send plot data points
                let t = frame_num as f64 * 0.02;
                for i in 0..TRACE_POINTS_PER_FRAME {
                    let phase = i as f64 * 0.1;
                    let _ = data_tx.try_send(DataPoint {
                        value: (t * 2.0 + phase).sin() * 50.0 + 100.0,
                        timestamp_secs: t + phase * 0.01,
                    });
                }

                // Yield to prevent busy-spin
                tokio::task::yield_now().await;
            }
        });
    });

    // Slint-side state
    let camera_buf = Rc::new(RefCell::new(SharedPixelBuffer::<Rgb8Pixel>::new(1, 1)));
    let plot_buf = Rc::new(RefCell::new(SharedPixelBuffer::<Rgb8Pixel>::new(1, 1)));
    let plot_state = Rc::new(RefCell::new(PlotState::new(2000, 10.0)));
    let fps_tracker = Rc::new(RefCell::new(FpsTracker::new(2.0)));
    let bench_start = Rc::new(RefCell::new(None::<Instant>));
    let display_frames = Rc::new(RefCell::new(0u64));
    let done = Rc::new(RefCell::new(false));

    // Poll timer: 1ms (as fast as event loop allows)
    let timer = Timer::default();
    {
        let window_weak = window.as_weak();
        let running = running.clone();
        let produced = produced_frames.clone();

        timer.start(TimerMode::Repeated, std::time::Duration::from_millis(1), move || {
            let Some(win) = window_weak.upgrade() else { return };

            if *done.borrow() {
                return;
            }

            // Init bench start on first frame
            let mut start = bench_start.borrow_mut();
            if start.is_none() {
                *start = Some(Instant::now());
            }

            // Drain camera frames (take latest)
            let mut latest_frame = None;
            while let Ok(frame) = frame_rx.try_recv() {
                latest_frame = Some(frame);
            }

            if let Some(frame) = latest_frame {
                let mut cb = camera_buf.borrow_mut();
                let img = camera_panel::render_frame(&frame, &mut cb);
                win.set_camera_image(img);
                fps_tracker.borrow_mut().tick();
                *display_frames.borrow_mut() += 1;
            }

            // Drain plot data
            let drained = plot_state.borrow_mut().drain(&data_rx);

            // Render plot
            if drained > 0 {
                let ps = plot_state.borrow();
                let mut pb = plot_buf.borrow_mut();
                let img = plot_panel::render_plot(&ps, 512, 512, &mut pb);
                win.set_plot_image(img);
            }

            // Update FPS display
            let fps = fps_tracker.borrow().fps();
            win.set_camera_fps(format!("{:.1} fps (display)", fps).into());
            win.set_plot_info(format!("{} pts buffered", drained).into());

            // Check if benchmark duration elapsed
            let elapsed = start.as_ref().map_or(0.0, |s| s.elapsed().as_secs_f64());
            if elapsed >= BENCH_DURATION_SECS {
                let disp = *display_frames.borrow();
                let prod = produced.load(Ordering::Relaxed);
                let display_fps = disp as f64 / elapsed;
                let produce_fps = prod as f64 / elapsed;
                let drop_rate = if prod > 0 {
                    (1.0 - disp as f64 / prod as f64) * 100.0
                } else {
                    0.0
                };

                let summary = format!(
                    "DONE | Tokio produced: {:.1} fps | Slint displayed: {:.1} fps | Drop rate: {:.1}% | Overhead vs 560fps baseline: {:.0}%",
                    produce_fps,
                    display_fps,
                    drop_rate,
                    (1.0 - display_fps / 560.0) * 100.0,
                );

                println!("\n=== Tokio Integration Benchmark Results ===");
                println!("Duration:          {:.1}s", elapsed);
                println!("Tokio produced:    {:.1} fps ({} frames)", produce_fps, prod);
                println!("Slint displayed:   {:.1} fps ({} frames)", display_fps, disp);
                println!("Frame drop rate:   {:.1}%", drop_rate);
                println!("vs 560fps baseline: {:.1}% overhead", (1.0 - display_fps / 560.0) * 100.0);
                println!("============================================\n");

                win.set_status_text(summary.into());
                running.store(false, Ordering::Relaxed);
                *done.borrow_mut() = true;
            } else {
                win.set_status_text(
                    format!("Benchmarking... {:.1}s / {:.0}s", elapsed, BENCH_DURATION_SECS).into(),
                );
            }
        });
    }

    window.run().expect("Failed to run window");
    running.store(false, Ordering::Relaxed);
}
