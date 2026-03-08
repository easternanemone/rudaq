#![allow(dead_code)]

use slint::{Image, Rgb8Pixel, SharedPixelBuffer};
use std::sync::mpsc;
use std::time::Instant;

/// A camera frame ready for display.
pub struct FrameData {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u16>,
    pub frame_number: u64,
}

pub type FrameSender = mpsc::SyncSender<FrameData>;
pub type FrameReceiver = mpsc::Receiver<FrameData>;

/// Create a bounded channel for frame delivery.
pub fn frame_channel(capacity: usize) -> (FrameSender, FrameReceiver) {
    mpsc::sync_channel(capacity)
}

/// Renders a 16-bit grayscale frame into an RGB8 pixel buffer with auto-contrast.
pub fn render_frame(frame: &FrameData, buf: &mut SharedPixelBuffer<Rgb8Pixel>) -> Image {
    if buf.width() != frame.width || buf.height() != frame.height {
        *buf = SharedPixelBuffer::new(frame.width, frame.height);
    }

    // Auto-contrast: find min/max
    let (mut vmin, mut vmax) = (u16::MAX, u16::MIN);
    for &v in &frame.data {
        vmin = vmin.min(v);
        vmax = vmax.max(v);
    }
    let range = (vmax - vmin).max(1) as f32;

    let pixels = buf.make_mut_bytes();
    for (i, &v) in frame.data.iter().enumerate() {
        let norm = ((v - vmin) as f32 / range * 255.0) as u8;
        let idx = i * 3;
        if idx + 2 < pixels.len() {
            // Viridis-inspired: dark purple -> teal -> yellow
            pixels[idx] = viridis_r(norm);
            pixels[idx + 1] = viridis_g(norm);
            pixels[idx + 2] = viridis_b(norm);
        }
    }

    Image::from_rgb8(buf.clone())
}

// Simplified viridis colormap (3 key stops: purple -> teal -> yellow)
fn viridis_r(v: u8) -> u8 {
    let t = v as f32 / 255.0;
    if t < 0.5 {
        (68.0 + t * 2.0 * (49.0 - 68.0)) as u8
    } else {
        (49.0 + (t - 0.5) * 2.0 * (253.0 - 49.0)) as u8
    }
}

fn viridis_g(v: u8) -> u8 {
    let t = v as f32 / 255.0;
    if t < 0.5 {
        (1.0 + t * 2.0 * (163.0 - 1.0)) as u8
    } else {
        (163.0 + (t - 0.5) * 2.0 * (231.0 - 163.0)) as u8
    }
}

fn viridis_b(v: u8) -> u8 {
    let t = v as f32 / 255.0;
    if t < 0.5 {
        (84.0 + t * 2.0 * (164.0 - 84.0)) as u8
    } else {
        (164.0 + (t - 0.5) * 2.0 * (37.0 - 164.0)) as u8
    }
}

/// FPS tracker using a rolling window.
pub struct FpsTracker {
    timestamps: Vec<Instant>,
    window_secs: f64,
}

impl FpsTracker {
    pub fn new(window_secs: f64) -> Self {
        Self {
            timestamps: Vec::with_capacity(256),
            window_secs,
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        self.timestamps.push(now);
        let cutoff = now - std::time::Duration::from_secs_f64(self.window_secs);
        self.timestamps.retain(|t| *t >= cutoff);
    }

    pub fn fps(&self) -> f64 {
        if self.timestamps.len() < 2 {
            return 0.0;
        }
        let elapsed = self
            .timestamps
            .last()
            .unwrap()
            .duration_since(*self.timestamps.first().unwrap());
        if elapsed.as_secs_f64() < 0.001 {
            return 0.0;
        }
        (self.timestamps.len() - 1) as f64 / elapsed.as_secs_f64()
    }
}
