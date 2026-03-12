//! Camera frame rendering and FPS tracking for the Slint UI.
//!
//! [`render_frame`] converts a 16-bit grayscale [`FrameData`] into an RGB8
//! [`slint::Image`] using per-frame auto-contrast and a viridis-inspired
//! colormap LUT.  The LUT is computed once via [`OnceLock`] on first use.
//!
//! [`FpsTracker`] measures display frame rate over a configurable rolling
//! window using a [`VecDeque`] so expired entries drain from the front in O(1).

use slint::{Image, Rgb8Pixel, SharedPixelBuffer};
use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::Instant;

/// A camera frame ready for display.
pub struct FrameData {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u16>,
    // Preserved for telemetry/ordering consumers outside render path.
    #[allow(dead_code)]
    pub frame_number: u64,
}

pub type FrameSender = mpsc::SyncSender<FrameData>;
pub type FrameReceiver = mpsc::Receiver<FrameData>;

/// Create a bounded channel for frame delivery.
pub fn frame_channel(capacity: usize) -> (FrameSender, FrameReceiver) {
    mpsc::sync_channel(capacity)
}

/// Renders a 16-bit grayscale frame into an RGB8 pixel buffer with auto-contrast.
///
/// Steps:
/// 1. Reallocate `buf` only when frame dimensions change.
/// 2. Compute per-frame min/max for auto-contrast normalisation.
/// 3. Map each pixel through the viridis LUT (dark purple → teal → yellow).
///
/// # Arguments
/// * `frame` – source frame; `data` is row-major 16-bit samples
/// * `buf`   – pixel buffer reused across calls; reallocated on size change
pub fn render_frame(frame: &FrameData, buf: &mut SharedPixelBuffer<Rgb8Pixel>) -> Image {
    if buf.width() != frame.width || buf.height() != frame.height {
        *buf = SharedPixelBuffer::new(frame.width, frame.height);
    }

    if frame.data.is_empty() {
        // Resize if the dimensions are valid, then zero the buffer so stale
        // pixels from a previous frame are not displayed.
        if frame.width > 0
            && frame.height > 0
            && (buf.width() != frame.width || buf.height() != frame.height)
        {
            *buf = SharedPixelBuffer::new(frame.width, frame.height);
        }
        buf.make_mut_bytes().fill(0);
        return Image::from_rgb8(buf.clone());
    }

    // Auto-contrast: find min/max
    let (mut vmin, mut vmax) = (u16::MAX, u16::MIN);
    for &v in &frame.data {
        vmin = vmin.min(v);
        vmax = vmax.max(v);
    }
    // Clamp to avoid underflow when vmin > vmax (impossible after the loop above,
    // but also covers the uniform-image case where vmin == vmax).
    let range = vmax.saturating_sub(vmin).max(1) as f32;

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

/// FPS tracker using a rolling time window.
///
/// Timestamps are stored in monotonically increasing order inside a [`VecDeque`].
/// On each [`tick`](Self::tick), entries older than `window_secs` are drained
/// from the front in O(k) where k is the number of expired entries — not O(n)
/// as with `Vec::retain`.
///
/// [`fps`](Self::fps) returns `(count − 1) / elapsed_secs` over the window,
/// so a window containing only a single timestamp returns `0.0`.
pub struct FpsTracker {
    timestamps: VecDeque<Instant>,
    window_secs: f64,
}

impl FpsTracker {
    /// Create a new tracker with the given rolling window in seconds.
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
        while self.timestamps.front().is_some_and(|t| *t < cutoff) {
            self.timestamps.pop_front();
        }
    }

    /// Returns the current frames-per-second estimate.
    ///
    /// Returns `0.0` when fewer than two timestamps are in the window or the
    /// elapsed span is under 1 ms (avoids division by near-zero).
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

    /// Inject a tick at an explicit `Instant`.  Used only in tests to avoid
    /// `thread::sleep` and make timing assertions deterministic.
    #[cfg(test)]
    fn tick_at(&mut self, at: Instant) {
        self.timestamps.push_back(at);
        let cutoff = at - std::time::Duration::from_secs_f64(self.window_secs);
        while self.timestamps.front().is_some_and(|t| *t < cutoff) {
            self.timestamps.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── viridis LUT ──────────────────────────────────────────────────────────

    #[test]
    fn viridis_lut_boundary_values() {
        let lut = viridis_lut();
        // t=0 → purple stop: (68, 1, 84)
        assert_eq!(lut[0], (68, 1, 84));
        // t=1 → yellow stop: (253, 231, 37)
        assert_eq!(lut[255], (253, 231, 37));
    }

    #[test]
    fn viridis_lut_is_256_entries() {
        assert_eq!(viridis_lut().len(), 256);
    }

    #[test]
    fn viridis_lut_is_cached() {
        // OnceLock must return the same pointer on every call.
        let a = viridis_lut() as *const _;
        let b = viridis_lut() as *const _;
        assert_eq!(a, b);
    }

    // ── render_frame ─────────────────────────────────────────────────────────

    #[test]
    fn render_frame_resizes_buffer_to_frame_dimensions() {
        let frame = FrameData {
            width: 8,
            height: 4,
            data: (0u16..32).collect(),
            frame_number: 1,
        };
        let mut buf = SharedPixelBuffer::new(1, 1);
        let _ = render_frame(&frame, &mut buf);
        assert_eq!(buf.width(), 8);
        assert_eq!(buf.height(), 4);
    }

    #[test]
    fn render_frame_uniform_data_does_not_panic() {
        // All pixels equal → vmin == vmax; range is clamped to 1 to avoid /0.
        let frame = FrameData {
            width: 4,
            height: 4,
            data: vec![32768u16; 16],
            frame_number: 0,
        };
        let mut buf = SharedPixelBuffer::new(1, 1);
        let _ = render_frame(&frame, &mut buf);
        // All pixels map to lut[0]; verify R channel matches purple stop.
        let bytes = buf.make_mut_bytes();
        assert_eq!(bytes[0], 68); // R of lut[0]
    }

    #[test]
    fn render_frame_empty_frame_does_not_panic() {
        let frame = FrameData {
            width: 0,
            height: 0,
            data: vec![],
            frame_number: 0,
        };
        let mut buf = SharedPixelBuffer::new(1, 1);
        let _ = render_frame(&frame, &mut buf);
    }

    // ── FpsTracker ───────────────────────────────────────────────────────────

    #[test]
    fn fps_tracker_empty_returns_zero() {
        let tracker = FpsTracker::new(2.0);
        assert_eq!(tracker.fps(), 0.0);
    }

    #[test]
    fn fps_tracker_single_tick_returns_zero() {
        let mut tracker = FpsTracker::new(2.0);
        tracker.tick();
        assert_eq!(tracker.fps(), 0.0);
    }

    #[test]
    fn fps_tracker_window_expiry_drains_front() {
        // Inject a timestamp 2 s in the past into a 1 s window; it must be
        // evicted on the next tick.  Uses tick_at to avoid thread::sleep.
        let now = Instant::now();
        let mut tracker = FpsTracker::new(1.0);
        tracker.tick_at(now - std::time::Duration::from_secs(2));
        tracker.tick_at(now);
        assert_eq!(
            tracker.timestamps.len(),
            1,
            "expired entry should be drained"
        );
    }

    #[test]
    fn fps_tracker_two_ticks_produce_positive_fps() {
        // 1 interval of 100 ms → ~10 fps.  Uses tick_at to avoid thread::sleep.
        let now = Instant::now();
        let mut tracker = FpsTracker::new(5.0);
        tracker.tick_at(now - std::time::Duration::from_millis(100));
        tracker.tick_at(now);
        let fps = tracker.fps();
        assert!(fps > 0.0);
        assert!((fps - 10.0).abs() < 1.0, "expected ~10 fps, got {fps:.1}");
    }
}
