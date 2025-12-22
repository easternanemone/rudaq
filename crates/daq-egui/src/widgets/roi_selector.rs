//! ROI (Region of Interest) selector widget for image analysis
//!
//! Provides drag-to-select rectangle ROI on images with live statistics:
//! - Mean, standard deviation, min, max pixel values
//! - ROI coordinates and dimensions
//! - Works with different bit depths (8, 12, 16-bit)

use eframe::egui;

/// ROI rectangle (in pixel coordinates)
#[derive(Debug, Clone, Copy, Default)]
pub struct Roi {
    /// X coordinate of top-left corner
    pub x: u32,
    /// Y coordinate of top-left corner
    pub y: u32,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

impl Roi {
    /// Create a new ROI with explicit coordinates (for external use)
    #[allow(dead_code)]
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    /// Create ROI from two corners (normalizes to ensure positive dimensions)
    pub fn from_corners(x1: i32, y1: i32, x2: i32, y2: i32) -> Self {
        let x = x1.min(x2).max(0) as u32;
        let y = y1.min(y2).max(0) as u32;
        let width = (x1 - x2).unsigned_abs();
        let height = (y1 - y2).unsigned_abs();
        Self { x, y, width, height }
    }

    /// Check if ROI has non-zero area
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Clamp ROI to image dimensions
    pub fn clamp_to_image(&self, image_width: u32, image_height: u32) -> Self {
        let x = self.x.min(image_width.saturating_sub(1));
        let y = self.y.min(image_height.saturating_sub(1));
        let width = self.width.min(image_width.saturating_sub(x));
        let height = self.height.min(image_height.saturating_sub(y));
        Self { x, y, width, height }
    }

    /// Get pixel count in ROI
    pub fn pixel_count(&self) -> usize {
        (self.width * self.height) as usize
    }
}

/// Statistics computed from ROI pixels
#[derive(Debug, Clone, Default)]
pub struct RoiStatistics {
    pub mean: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    pub pixel_count: usize,
}

impl RoiStatistics {
    /// Compute statistics from 8-bit grayscale pixels
    pub fn from_u8_pixels(pixels: &[u8]) -> Self {
        if pixels.is_empty() {
            return Self::default();
        }

        let n = pixels.len();
        let mut sum: f64 = 0.0;
        let mut min = u8::MAX;
        let mut max = u8::MIN;

        for &p in pixels {
            sum += p as f64;
            min = min.min(p);
            max = max.max(p);
        }

        let mean = sum / n as f64;

        let variance: f64 = pixels.iter()
            .map(|&p| (p as f64 - mean).powi(2))
            .sum::<f64>() / n as f64;

        Self {
            mean,
            std_dev: variance.sqrt(),
            min: min as f64,
            max: max as f64,
            pixel_count: n,
        }
    }

    /// Compute statistics from 16-bit pixels
    pub fn from_u16_pixels(pixels: &[u16]) -> Self {
        if pixels.is_empty() {
            return Self::default();
        }

        let n = pixels.len();
        let mut sum: f64 = 0.0;
        let mut min = u16::MAX;
        let mut max = u16::MIN;

        for &p in pixels {
            sum += p as f64;
            min = min.min(p);
            max = max.max(p);
        }

        let mean = sum / n as f64;

        let variance: f64 = pixels.iter()
            .map(|&p| (p as f64 - mean).powi(2))
            .sum::<f64>() / n as f64;

        Self {
            mean,
            std_dev: variance.sqrt(),
            min: min as f64,
            max: max as f64,
            pixel_count: n,
        }
    }

    /// Compute statistics from raw frame data with given bit depth
    ///
    /// # Arguments
    /// * `data` - Raw pixel data
    /// * `image_width` - Image width in pixels
    /// * `image_height` - Image height in pixels
    /// * `bit_depth` - Bits per pixel (8, 12, or 16)
    /// * `roi` - Region of interest to analyze
    pub fn from_frame_roi(
        data: &[u8],
        image_width: u32,
        image_height: u32,
        bit_depth: u32,
        roi: &Roi,
    ) -> Self {
        // Clamp ROI to actual image dimensions
        let roi = roi.clamp_to_image(image_width, image_height);
        if !roi.is_valid() || image_width == 0 || image_height == 0 {
            return Self::default();
        }

        match bit_depth {
            8 => {
                // Extract 8-bit pixels from ROI
                let mut pixels = Vec::with_capacity(roi.pixel_count());
                for row in 0..roi.height {
                    let y = roi.y + row;
                    // Use checked arithmetic to prevent overflow
                    let Some(row_offset) = (y as u64).checked_mul(image_width as u64) else {
                        continue;
                    };
                    let row_start = (row_offset + roi.x as u64) as usize;
                    let row_end = row_start.saturating_add(roi.width as usize);
                    if row_end <= data.len() {
                        pixels.extend_from_slice(&data[row_start..row_end]);
                    }
                }
                Self::from_u8_pixels(&pixels)
            }
            12 | 16 => {
                // Extract 16-bit pixels from ROI
                let bytes_per_pixel: u64 = 2;
                let mut pixels = Vec::with_capacity(roi.pixel_count());
                for row in 0..roi.height {
                    let y = roi.y + row;
                    for col in 0..roi.width {
                        let x = roi.x + col;
                        // Use checked arithmetic to prevent overflow
                        let Some(pixel_offset) = (y as u64).checked_mul(image_width as u64) else {
                            continue;
                        };
                        let Some(byte_offset) = (pixel_offset + x as u64).checked_mul(bytes_per_pixel) else {
                            continue;
                        };
                        let byte_idx = byte_offset as usize;
                        if byte_idx.saturating_add(1) < data.len() {
                            let pixel = u16::from_le_bytes([data[byte_idx], data[byte_idx + 1]]);
                            pixels.push(pixel);
                        }
                    }
                }
                Self::from_u16_pixels(&pixels)
            }
            _ => Self::default(),
        }
    }
}

/// ROI selection state for the image viewer
#[derive(Debug, Default)]
pub struct RoiSelector {
    /// Current ROI (None if no selection)
    roi: Option<Roi>,
    /// Statistics for current ROI
    stats: Option<RoiStatistics>,
    /// Selection in progress (start corner)
    drag_start: Option<(i32, i32)>,
    /// Current drag position
    drag_current: Option<(i32, i32)>,
    /// Is selection mode active
    pub selection_mode: bool,
}

impl RoiSelector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get current ROI if valid
    pub fn roi(&self) -> Option<&Roi> {
        self.roi.as_ref()
    }

    /// Get current statistics if available
    #[allow(dead_code)]
    pub fn statistics(&self) -> Option<&RoiStatistics> {
        self.stats.as_ref()
    }

    /// Clear current selection
    pub fn clear(&mut self) {
        self.roi = None;
        self.stats = None;
        self.drag_start = None;
        self.drag_current = None;
    }

    /// Set ROI and compute statistics from frame data
    pub fn set_roi_from_frame(
        &mut self,
        roi: Roi,
        frame_data: &[u8],
        image_width: u32,
        image_height: u32,
        bit_depth: u32,
    ) {
        let stats = RoiStatistics::from_frame_roi(frame_data, image_width, image_height, bit_depth, &roi);
        self.roi = Some(roi);
        self.stats = Some(stats);
    }

    /// Update statistics from new frame data (same ROI)
    pub fn update_statistics(
        &mut self,
        frame_data: &[u8],
        image_width: u32,
        image_height: u32,
        bit_depth: u32,
    ) {
        if let Some(roi) = &self.roi {
            let stats = RoiStatistics::from_frame_roi(frame_data, image_width, image_height, bit_depth, roi);
            self.stats = Some(stats);
        }
    }

    /// Handle input for ROI selection on image
    ///
    /// Returns true if a new ROI was finalized
    pub fn handle_input(
        &mut self,
        response: &egui::Response,
        image_rect: egui::Rect,
        image_size: (u32, u32),
        zoom: f32,
        pan: egui::Vec2,
    ) -> bool {
        if !self.selection_mode {
            return false;
        }

        let image_offset = (image_rect.size() - egui::vec2(image_size.0 as f32 * zoom, image_size.1 as f32 * zoom)) / 2.0 + pan;

        // Convert screen position to pixel coordinates
        let screen_to_pixel = |pos: egui::Pos2| -> (i32, i32) {
            let relative = pos - image_rect.min - image_offset;
            let pixel_x = (relative.x / zoom) as i32;
            let pixel_y = (relative.y / zoom) as i32;
            (pixel_x, pixel_y)
        };

        let mut finalized = false;

        // Handle drag start
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                self.drag_start = Some(screen_to_pixel(pos));
                self.drag_current = self.drag_start;
            }
        }

        // Handle drag
        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                self.drag_current = Some(screen_to_pixel(pos));
            }
        }

        // Handle drag end
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let (Some(start), Some(end)) = (self.drag_start, self.drag_current) {
                let roi = Roi::from_corners(start.0, start.1, end.0, end.1);
                let roi = roi.clamp_to_image(image_size.0, image_size.1);
                if roi.is_valid() {
                    self.roi = Some(roi);
                    finalized = true;
                }
            }
            self.drag_start = None;
            self.drag_current = None;
        }

        finalized
    }

    /// Draw ROI overlay on the image
    pub fn draw_overlay(
        &self,
        painter: &egui::Painter,
        image_rect: egui::Rect,
        image_size: (u32, u32),
        zoom: f32,
        pan: egui::Vec2,
    ) {
        let image_offset = (image_rect.size() - egui::vec2(image_size.0 as f32 * zoom, image_size.1 as f32 * zoom)) / 2.0 + pan;

        // Convert pixel coordinates to screen position
        let pixel_to_screen = |px: u32, py: u32| -> egui::Pos2 {
            image_rect.min + image_offset + egui::vec2(px as f32 * zoom, py as f32 * zoom)
        };

        // Draw in-progress selection
        if let (Some(start), Some(current)) = (self.drag_start, self.drag_current) {
            let p1 = pixel_to_screen(start.0.max(0) as u32, start.1.max(0) as u32);
            let p2 = pixel_to_screen(current.0.max(0) as u32, current.1.max(0) as u32);
            let rect = egui::Rect::from_two_pos(p1, p2);

            // Semi-transparent fill
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(100, 150, 255, 50));
            // Border
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
                egui::StrokeKind::Outside,
            );
        }

        // Draw finalized ROI
        if let Some(roi) = &self.roi {
            let p1 = pixel_to_screen(roi.x, roi.y);
            let p2 = pixel_to_screen(roi.x + roi.width, roi.y + roi.height);
            let rect = egui::Rect::from_two_pos(p1, p2);

            // Semi-transparent fill
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(255, 200, 100, 30));
            // Border
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 100)),
                egui::StrokeKind::Outside,
            );
        }
    }

    /// Render statistics panel
    pub fn show_statistics_panel(&self, ui: &mut egui::Ui) {
        if let (Some(roi), Some(stats)) = (&self.roi, &self.stats) {
            ui.group(|ui| {
                ui.label("ROI Statistics");
                ui.separator();

                egui::Grid::new("roi_stats_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Position:");
                        ui.label(format!("({}, {})", roi.x, roi.y));
                        ui.end_row();

                        ui.label("Size:");
                        ui.label(format!("{}x{}", roi.width, roi.height));
                        ui.end_row();

                        ui.label("Pixels:");
                        ui.label(format!("{}", stats.pixel_count));
                        ui.end_row();

                        ui.separator();
                        ui.end_row();

                        ui.label("Mean:");
                        ui.label(format!("{:.2}", stats.mean));
                        ui.end_row();

                        ui.label("Std Dev:");
                        ui.label(format!("{:.2}", stats.std_dev));
                        ui.end_row();

                        ui.label("Min:");
                        ui.label(format!("{:.0}", stats.min));
                        ui.end_row();

                        ui.label("Max:");
                        ui.label(format!("{:.0}", stats.max));
                        ui.end_row();
                    });
            });
        } else {
            ui.label("No ROI selected");
            if self.selection_mode {
                ui.label("Drag on image to select ROI");
            }
        }
    }
}
