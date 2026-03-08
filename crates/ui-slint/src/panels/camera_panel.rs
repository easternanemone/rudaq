use slint::{Image, Rgb8Pixel, SharedPixelBuffer};
use std::collections::VecDeque;
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

    // Build LUT once; eliminates per-pixel float operations on hot path.
    let lut = viridis_lut();
    let pixels = buf.make_mut_bytes();
    for (i, &v) in frame.data.iter().enumerate() {
        let norm = ((v - vmin) as f32 / range * 255.0) as u8;
        let idx = i * 3;
        if idx + 2 < pixels.len() {
            let (r, g, b) = lut[norm as usize];
            pixels[idx] = r;
            pixels[idx + 1] = g;
            pixels[idx + 2] = b;
        }
    }

    Image::from_rgb8(buf.clone())
}

/// Viridis-inspired colormap LUT (dark purple → teal → yellow), 256 entries.
/// Computed once via `OnceLock`; all subsequent calls return the cached array.
fn viridis_lut() -> &'static [(u8, u8, u8); 256] {
    use std::sync::OnceLock;
    static LUT: OnceLock<[(u8, u8, u8); 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [(0u8, 0u8, 0u8); 256];
        for (v, entry) in lut.iter_mut().enumerate() {
            let t = v as f32 / 255.0;
            let r = if t < 0.5 {
                68.0 + t * 2.0 * (49.0 - 68.0)
            } else {
                49.0 + (t - 0.5) * 2.0 * (253.0 - 49.0)
            };
            let g = if t < 0.5 {
                1.0 + t * 2.0 * (163.0 - 1.0)
            } else {
                163.0 + (t - 0.5) * 2.0 * (231.0 - 163.0)
            };
            let b = if t < 0.5 {
                84.0 + t * 2.0 * (164.0 - 84.0)
            } else {
                164.0 + (t - 0.5) * 2.0 * (37.0 - 164.0)
            };
            *entry = (r as u8, g as u8, b as u8);
        }
        lut
    })
}

/// FPS tracker using a rolling window.
pub struct FpsTracker {
    timestamps: VecDeque<Instant>,
    window_secs: f64,
}

impl FpsTracker {
    pub fn new(window_secs: f64) -> Self {
        Self {
            timestamps: VecDeque::with_capacity(256),
            window_secs,
        }
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        self.timestamps.push_back(now);
        // Drain expired entries from the front; timestamps are monotonically
        // increasing so we stop at the first entry still within the window.
        let cutoff = now - std::time::Duration::from_secs_f64(self.window_secs);
        while self.timestamps.front().map_or(false, |t| *t < cutoff) {
            self.timestamps.pop_front();
        }
    }

    pub fn fps(&self) -> f64 {
        if self.timestamps.len() < 2 {
            return 0.0;
        }
        let elapsed = self
            .timestamps
            .back()
            .unwrap()
            .duration_since(*self.timestamps.front().unwrap());
        if elapsed.as_secs_f64() < 0.001 {
            return 0.0;
        }
        (self.timestamps.len() - 1) as f64 / elapsed.as_secs_f64()
    }
}
