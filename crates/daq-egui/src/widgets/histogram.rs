//! Histogram widget for image intensity distribution
//!
//! Provides a bar chart visualization of pixel intensity values:
//! - Configurable number of bins
//! - Linear or logarithmic Y-axis scale
//! - Support for 8, 12, and 16-bit images
//! - Live updates with streaming frames

use eframe::egui;

/// Number of bins for histogram
const DEFAULT_BINS: usize = 256;

/// Histogram data and rendering
#[derive(Debug, Clone)]
pub struct Histogram {
    /// Bin counts
    bins: Vec<u32>,
    /// Maximum bin count (for scaling)
    max_count: u32,
    /// Total pixel count
    total_pixels: usize,
    /// Bit depth of source image
    bit_depth: u32,
    /// Use logarithmic scale for Y-axis
    pub log_scale: bool,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            bins: vec![0; DEFAULT_BINS],
            max_count: 0,
            total_pixels: 0,
            bit_depth: 8,
            log_scale: false,
        }
    }
}

impl Histogram {
    /// Create a new histogram with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Create histogram with custom bin count
    ///
    /// # Panics
    /// Panics in debug mode if `num_bins` is 0.
    #[allow(dead_code)]
    pub fn with_bins(num_bins: usize) -> Self {
        debug_assert!(num_bins > 0, "Histogram must have at least 1 bin");
        let num_bins = num_bins.max(1); // Ensure at least 1 bin in release
        Self {
            bins: vec![0; num_bins],
            ..Self::default()
        }
    }

    /// Clear all bins
    pub fn clear(&mut self) {
        self.bins.fill(0);
        self.max_count = 0;
        self.total_pixels = 0;
    }

    /// Compute histogram from 8-bit pixel data
    pub fn from_u8_pixels(&mut self, pixels: &[u8]) {
        self.clear();
        self.bit_depth = 8;
        self.total_pixels = pixels.len();

        let num_bins = self.bins.len();
        let scale = num_bins as f32 / 256.0;

        for &pixel in pixels {
            let bin = ((pixel as f32 * scale) as usize).min(num_bins - 1);
            self.bins[bin] += 1;
        }

        self.max_count = *self.bins.iter().max().unwrap_or(&0);
    }

    /// Compute histogram from 16-bit pixel data
    pub fn from_u16_pixels(&mut self, pixels: &[u16], bit_depth: u32) {
        self.clear();
        self.bit_depth = bit_depth;
        self.total_pixels = pixels.len();

        let num_bins = self.bins.len();
        let max_val = match bit_depth {
            12 => 4095.0,
            16 => 65535.0,
            _ => 65535.0,
        };
        let scale = num_bins as f32 / (max_val + 1.0);

        for &pixel in pixels {
            let bin = ((pixel as f32 * scale) as usize).min(num_bins - 1);
            self.bins[bin] += 1;
        }

        self.max_count = *self.bins.iter().max().unwrap_or(&0);
    }

    /// Compute histogram from raw frame data
    pub fn from_frame_data(&mut self, data: &[u8], width: u32, height: u32, bit_depth: u32) {
        let expected_pixels = (width * height) as usize;

        match bit_depth {
            8 => {
                let pixels = &data[..expected_pixels.min(data.len())];
                self.from_u8_pixels(pixels);
            }
            12 | 16 => {
                let bytes_per_pixel = 2;
                let expected_bytes = expected_pixels * bytes_per_pixel;
                let actual_pixels = data.len().min(expected_bytes) / bytes_per_pixel;

                let mut pixels = Vec::with_capacity(actual_pixels);
                for i in 0..actual_pixels {
                    let byte_idx = i * 2;
                    if byte_idx + 1 < data.len() {
                        let pixel = u16::from_le_bytes([data[byte_idx], data[byte_idx + 1]]);
                        pixels.push(pixel);
                    }
                }
                self.from_u16_pixels(&pixels, bit_depth);
            }
            _ => {
                self.clear();
            }
        }
    }

    /// Get the bin count at a given index
    #[allow(dead_code)]
    pub fn bin_count(&self, index: usize) -> u32 {
        self.bins.get(index).copied().unwrap_or(0)
    }

    /// Get number of bins
    #[allow(dead_code)]
    pub fn num_bins(&self) -> usize {
        self.bins.len()
    }

    /// Get maximum bin count
    #[allow(dead_code)]
    pub fn max_count(&self) -> u32 {
        self.max_count
    }

    /// Get total pixel count
    #[allow(dead_code)]
    pub fn total_pixels(&self) -> usize {
        self.total_pixels
    }

    /// Render histogram as a small overlay widget
    pub fn show_overlay(&self, ui: &mut egui::Ui, size: egui::Vec2) {
        let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());

        if self.max_count == 0 {
            return;
        }

        let painter = ui.painter();

        // Semi-transparent background
        painter.rect_filled(
            rect,
            4.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
        );

        // Draw histogram bars
        let num_bins = self.bins.len();
        let bar_width = rect.width() / num_bins as f32;
        let max_height = rect.height() - 4.0;

        let max_value = if self.log_scale {
            (self.max_count as f64 + 1.0).ln()
        } else {
            self.max_count as f64
        };

        for (i, &count) in self.bins.iter().enumerate() {
            if count == 0 {
                continue;
            }

            let normalized = if self.log_scale {
                (count as f64 + 1.0).ln() / max_value
            } else {
                count as f64 / max_value
            };

            let bar_height = (normalized * max_height as f64) as f32;
            let x = rect.left() + i as f32 * bar_width;
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.bottom() - bar_height - 2.0),
                egui::vec2(bar_width.max(1.0), bar_height),
            );

            // Color gradient based on intensity (white for low bins, yellow/orange for high)
            let intensity = i as f32 / num_bins as f32;
            let color = egui::Color32::from_rgb(
                200 + (55.0 * intensity) as u8,
                200 - (100.0 * intensity) as u8,
                200 - (150.0 * intensity) as u8,
            );

            painter.rect_filled(bar_rect, 0.0, color);
        }
    }

    /// Render histogram in a panel with controls
    pub fn show_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Histogram");
            ui.checkbox(&mut self.log_scale, "Log");
        });

        let available = ui.available_size();
        let height = available.y.clamp(60.0, 120.0);
        let size = egui::vec2(available.x, height);

        self.show_overlay(ui, size);

        // Stats below histogram
        if self.total_pixels > 0 {
            ui.horizontal(|ui| {
                ui.small(format!("{} px", self.total_pixels));
                ui.separator();
                ui.small(format!("{}-bit", self.bit_depth));
            });
        }
    }
}

/// Histogram overlay position
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistogramPosition {
    /// Hidden
    Hidden,
    /// Bottom-right corner overlay
    #[default]
    BottomRight,
    /// Bottom-left corner overlay
    BottomLeft,
    /// Top-right corner overlay
    TopRight,
    /// Top-left corner overlay
    TopLeft,
    /// Side panel (not overlay)
    SidePanel,
}

impl HistogramPosition {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Hidden => "Hidden",
            Self::BottomRight => "Bottom Right",
            Self::BottomLeft => "Bottom Left",
            Self::TopRight => "Top Right",
            Self::TopLeft => "Top Left",
            Self::SidePanel => "Side Panel",
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self, Self::Hidden)
    }

    pub fn is_overlay(&self) -> bool {
        matches!(
            self,
            Self::BottomRight | Self::BottomLeft | Self::TopRight | Self::TopLeft
        )
    }

    /// Calculate overlay rect within an image rect
    pub fn overlay_rect(&self, image_rect: egui::Rect, size: egui::Vec2) -> egui::Rect {
        let margin = 8.0;
        match self {
            Self::BottomRight => egui::Rect::from_min_size(
                egui::pos2(
                    image_rect.right() - size.x - margin,
                    image_rect.bottom() - size.y - margin,
                ),
                size,
            ),
            Self::BottomLeft => egui::Rect::from_min_size(
                egui::pos2(
                    image_rect.left() + margin,
                    image_rect.bottom() - size.y - margin,
                ),
                size,
            ),
            Self::TopRight => egui::Rect::from_min_size(
                egui::pos2(
                    image_rect.right() - size.x - margin,
                    image_rect.top() + margin,
                ),
                size,
            ),
            Self::TopLeft => egui::Rect::from_min_size(
                egui::pos2(image_rect.left() + margin, image_rect.top() + margin),
                size,
            ),
            _ => egui::Rect::NOTHING,
        }
    }
}
