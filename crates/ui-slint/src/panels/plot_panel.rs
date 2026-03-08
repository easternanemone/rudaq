use slint::{Image, Rgb8Pixel, SharedPixelBuffer};
use std::collections::VecDeque;
use std::sync::mpsc;

/// A single data point for the plot.
pub struct DataPoint {
    pub value: f64,
    pub timestamp_secs: f64,
}

pub type DataSender = mpsc::SyncSender<DataPoint>;
pub type DataReceiver = mpsc::Receiver<DataPoint>;

/// Create a bounded channel for data delivery.
pub fn data_channel(capacity: usize) -> (DataSender, DataReceiver) {
    mpsc::sync_channel(capacity)
}

/// Auto-scaling plot state (grow-only bounds, matching egui AutoScalePlot behavior).
pub struct PlotState {
    points: VecDeque<(f64, f64)>,
    max_points: usize,
    y_min: f64,
    y_max: f64,
    time_window_secs: f64,
}

impl PlotState {
    pub fn new(max_points: usize, time_window_secs: f64) -> Self {
        Self {
            points: VecDeque::with_capacity(max_points),
            max_points,
            y_min: f64::INFINITY,
            y_max: f64::NEG_INFINITY,
            time_window_secs,
        }
    }

    pub fn push(&mut self, t: f64, v: f64) {
        self.points.push_back((t, v));
        if self.points.len() > self.max_points {
            self.points.pop_front();
        }
        // Grow-only bounds
        if v < self.y_min {
            self.y_min = v;
        }
        if v > self.y_max {
            self.y_max = v;
        }
    }

    pub fn reset_bounds(&mut self) {
        self.y_min = f64::INFINITY;
        self.y_max = f64::NEG_INFINITY;
        for &(_, v) in &self.points {
            if v < self.y_min {
                self.y_min = v;
            }
            if v > self.y_max {
                self.y_max = v;
            }
        }
    }
}

/// Render the plot state into an RGB8 image.
pub fn render_plot(
    state: &PlotState,
    width: u32,
    height: u32,
    buf: &mut SharedPixelBuffer<Rgb8Pixel>,
) -> Image {
    if buf.width() != width || buf.height() != height {
        *buf = SharedPixelBuffer::new(width, height);
    }
    let pixels = buf.make_mut_bytes();

    // Dark background
    for chunk in pixels.chunks_exact_mut(3) {
        chunk[0] = 20;
        chunk[1] = 20;
        chunk[2] = 30;
    }

    if state.points.len() < 2 {
        return Image::from_rgb8(buf.clone());
    }

    // Time range: latest - window
    let t_max = state.points.back().map_or(0.0, |p| p.0);
    let t_min = t_max - state.time_window_secs;

    // Y range with padding
    let y_range = (state.y_max - state.y_min).max(0.001);
    let y_pad = y_range * 0.05;
    let y_lo = state.y_min - y_pad;
    let y_hi = state.y_max + y_pad;
    let y_span = y_hi - y_lo;

    let margin = 4u32;

    // Draw horizontal grid (5 lines)
    for gi in 0..5 {
        let gy = margin + (gi * (height - 2 * margin)) / 4;
        for x in 0..width {
            let idx = ((gy * width + x) * 3) as usize;
            if idx + 2 < pixels.len() {
                pixels[idx] = 40;
                pixels[idx + 1] = 40;
                pixels[idx + 2] = 50;
            }
        }
    }

    // Map data point to pixel coords
    let to_px = |t: f64, v: f64| -> (i32, i32) {
        let nx = (t - t_min) / (t_max - t_min).max(0.001);
        let ny = (v - y_lo) / y_span;
        let px = (nx * (width - 2 * margin) as f64 + margin as f64) as i32;
        let py = ((1.0 - ny) * (height - 2 * margin) as f64 + margin as f64) as i32;
        (px, py)
    };

    // Draw visible points as connected line segments.
    // Uses a rolling `prev` to avoid collecting into a Vec each frame.
    let mut prev: Option<(f64, f64)> = None;
    for &(t, v) in state.points.iter().filter(|(t, _)| *t >= t_min) {
        if let Some((pt, pv)) = prev {
            let (x0, y0) = to_px(pt, pv);
            let (x1, y1) = to_px(t, v);
            draw_line(pixels, width, height, x0, y0, x1, y1, [80, 220, 120]);
        }
        prev = Some((t, v));
    }

    Image::from_rgb8(buf.clone())
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 3],
) {
    let (mut cx, mut cy) = (x0, y0);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if cx >= 0 && (cx as u32) < width && cy >= 0 && (cy as u32) < height {
            let idx = ((cy as u32 * width + cx as u32) * 3) as usize;
            if idx + 2 < pixels.len() {
                pixels[idx] = color[0];
                pixels[idx + 1] = color[1];
                pixels[idx + 2] = color[2];
            }
        }
        if cx == x1 && cy == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            err += dx;
            cy += sy;
        }
    }
}
