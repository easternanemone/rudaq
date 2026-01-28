//! Line Profile widget for intensity analysis along user-defined lines
//!
//! Extracts and displays intensity profiles from images:
//! - Draw lines on images (horizontal, vertical, arbitrary angles)
//! - Real-time intensity plot updates
//! - Multiple simultaneous profiles with different colors
//! - Statistics: min, max, mean, FWHM (Full Width at Half Maximum)
//! - CSV export of profile data
//! - Live updates during frame acquisition

use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};

/// A single line selection on the image
#[derive(Debug, Clone)]
pub struct LineSelection {
    /// Start point (image coordinates)
    pub start: egui::Pos2,
    /// End point (image coordinates)
    pub end: egui::Pos2,
    /// Line color
    pub color: egui::Color32,
    /// Line label
    pub label: String,
    /// Whether this line is currently being edited
    pub editing: bool,
}

impl LineSelection {
    /// Create a new line selection
    pub fn new(start: egui::Pos2, end: egui::Pos2, color: egui::Color32, label: String) -> Self {
        Self {
            start,
            end,
            color,
            label,
            editing: false,
        }
    }

    /// Get the length of the line in pixels
    pub fn length(&self) -> f32 {
        let dx = self.end.x - self.start.x;
        let dy = self.end.y - self.start.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// Check if a point is near the line (for selection)
    pub fn is_near(&self, point: egui::Pos2, threshold: f32) -> bool {
        let line_len = self.length();
        if line_len < 1.0 {
            return false;
        }

        // Project point onto line segment
        let dx = self.end.x - self.start.x;
        let dy = self.end.y - self.start.y;
        let t =
            ((point.x - self.start.x) * dx + (point.y - self.start.y) * dy) / (line_len * line_len);
        let t = t.clamp(0.0, 1.0);

        let proj_x = self.start.x + t * dx;
        let proj_y = self.start.y + t * dy;

        let dist = ((point.x - proj_x).powi(2) + (point.y - proj_y).powi(2)).sqrt();
        dist < threshold
    }
}

/// Intensity profile extracted from an image
#[derive(Debug, Clone)]
pub struct IntensityProfile {
    /// Distance along line (in pixels)
    pub distances: Vec<f64>,
    /// Intensity values
    pub intensities: Vec<f64>,
    /// Line selection this profile was extracted from
    pub line: LineSelection,
    /// Statistics
    pub stats: ProfileStats,
}

/// Statistical measures of an intensity profile
#[derive(Debug, Clone, Default)]
pub struct ProfileStats {
    /// Minimum intensity
    pub min: f64,
    /// Maximum intensity
    pub max: f64,
    /// Mean intensity
    pub mean: f64,
    /// Full Width at Half Maximum (in pixels)
    pub fwhm: Option<f64>,
}

impl IntensityProfile {
    /// Extract intensity profile from image data along a line
    pub fn extract(
        line: &LineSelection,
        image_data: &[u8],
        width: u32,
        height: u32,
        bit_depth: u32,
    ) -> Self {
        let num_samples = line.length().ceil() as usize;
        let num_samples = num_samples.max(2); // At least 2 samples

        let mut distances = Vec::with_capacity(num_samples);
        let mut intensities = Vec::with_capacity(num_samples);

        let dx = line.end.x - line.start.x;
        let dy = line.end.y - line.start.y;
        let line_length = line.length();

        for i in 0..num_samples {
            let t = i as f32 / (num_samples - 1) as f32;
            let x = line.start.x + t * dx;
            let y = line.start.y + t * dy;

            // Clamp to image bounds
            let x = x.clamp(0.0, (width - 1) as f32);
            let y = y.clamp(0.0, (height - 1) as f32);

            // Sample using bilinear interpolation
            let intensity = Self::sample_bilinear(image_data, width, height, bit_depth, x, y);

            distances.push((t * line_length) as f64);
            intensities.push(intensity);
        }

        let stats = Self::compute_stats(&intensities);

        Self {
            distances,
            intensities,
            line: line.clone(),
            stats,
        }
    }

    /// Sample image intensity at fractional coordinates using bilinear interpolation
    fn sample_bilinear(
        data: &[u8],
        width: u32,
        height: u32,
        bit_depth: u32,
        x: f32,
        y: f32,
    ) -> f64 {
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(width - 1);
        let y1 = (y0 + 1).min(height - 1);

        let fx = x - x0 as f32;
        let fy = y - y0 as f32;

        // Get pixel values at four corners
        let v00 = Self::get_pixel(data, width, x0, y0, bit_depth);
        let v10 = Self::get_pixel(data, width, x1, y0, bit_depth);
        let v01 = Self::get_pixel(data, width, x0, y1, bit_depth);
        let v11 = Self::get_pixel(data, width, x1, y1, bit_depth);

        // Bilinear interpolation
        let v0 = v00 * (1.0 - fx as f64) + v10 * fx as f64;
        let v1 = v01 * (1.0 - fx as f64) + v11 * fx as f64;
        v0 * (1.0 - fy as f64) + v1 * fy as f64
    }

    /// Get pixel value at integer coordinates
    fn get_pixel(data: &[u8], width: u32, x: u32, y: u32, bit_depth: u32) -> f64 {
        let idx = (y * width + x) as usize;

        match bit_depth {
            8 => {
                if idx < data.len() {
                    data[idx] as f64
                } else {
                    0.0
                }
            }
            12 | 16 => {
                let byte_idx = idx * 2;
                if byte_idx + 1 < data.len() {
                    let pixel = u16::from_le_bytes([data[byte_idx], data[byte_idx + 1]]);
                    pixel as f64
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }

    /// Compute statistics for the profile
    fn compute_stats(intensities: &[f64]) -> ProfileStats {
        if intensities.is_empty() {
            return ProfileStats::default();
        }

        let min = intensities.iter().copied().fold(f64::INFINITY, f64::min);
        let max = intensities
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let sum: f64 = intensities.iter().sum();
        let mean = sum / intensities.len() as f64;

        // Compute FWHM (Full Width at Half Maximum)
        let fwhm = Self::compute_fwhm(intensities, max, min);

        ProfileStats {
            min,
            max,
            mean,
            fwhm,
        }
    }

    /// Compute Full Width at Half Maximum
    fn compute_fwhm(intensities: &[f64], max: f64, min: f64) -> Option<f64> {
        if max <= min {
            return None;
        }

        let half_max = min + (max - min) / 2.0;

        // Find first crossing of half-max
        let mut first_idx = None;
        for (i, &val) in intensities.iter().enumerate() {
            if val >= half_max {
                first_idx = Some(i);
                break;
            }
        }

        // Find last crossing of half-max
        let mut last_idx = None;
        for (i, &val) in intensities.iter().enumerate().rev() {
            if val >= half_max {
                last_idx = Some(i);
                break;
            }
        }

        match (first_idx, last_idx) {
            (Some(first), Some(last)) if last > first => Some((last - first) as f64),
            _ => None,
        }
    }

    /// Export profile data to CSV format
    pub fn to_csv(&self) -> String {
        let mut csv = String::new();
        csv.push_str("Distance (px),Intensity\n");

        for (dist, intensity) in self.distances.iter().zip(self.intensities.iter()) {
            csv.push_str(&format!("{:.2},{:.2}\n", dist, intensity));
        }

        csv
    }
}

/// Line Profile widget state and UI
#[derive(Debug)]
pub struct LineProfileWidget {
    /// Active line selections
    lines: Vec<LineSelection>,
    /// Extracted intensity profiles
    profiles: Vec<IntensityProfile>,
    /// Currently drawing a new line
    drawing: bool,
    /// Temporary line being drawn
    temp_line_start: Option<egui::Pos2>,
    /// Show the profile plot
    pub show_plot: bool,
    /// Show statistics panel
    pub show_stats: bool,
    /// Color palette for new lines
    color_palette: Vec<egui::Color32>,
    /// Next color index
    next_color_idx: usize,
}

impl Default for LineProfileWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl LineProfileWidget {
    /// Create a new line profile widget
    pub fn new() -> Self {
        let color_palette = vec![
            egui::Color32::from_rgb(255, 100, 100), // Red
            egui::Color32::from_rgb(100, 255, 100), // Green
            egui::Color32::from_rgb(100, 100, 255), // Blue
            egui::Color32::from_rgb(255, 255, 100), // Yellow
            egui::Color32::from_rgb(255, 100, 255), // Magenta
            egui::Color32::from_rgb(100, 255, 255), // Cyan
        ];

        Self {
            lines: Vec::new(),
            profiles: Vec::new(),
            drawing: false,
            temp_line_start: None,
            show_plot: true,
            show_stats: true,
            color_palette,
            next_color_idx: 0,
        }
    }

    /// Get the next color from the palette
    fn next_color(&mut self) -> egui::Color32 {
        let color = self.color_palette[self.next_color_idx];
        self.next_color_idx = (self.next_color_idx + 1) % self.color_palette.len();
        color
    }

    /// Update profiles from current frame data
    pub fn update_profiles(&mut self, image_data: &[u8], width: u32, height: u32, bit_depth: u32) {
        self.profiles.clear();
        for line in &self.lines {
            if !line.editing {
                let profile = IntensityProfile::extract(line, image_data, width, height, bit_depth);
                self.profiles.push(profile);
            }
        }
    }

    /// Handle mouse input for line drawing/editing (returns true if interaction occurred)
    pub fn handle_input(
        &mut self,
        _ui: &mut egui::Ui,
        image_rect: egui::Rect,
        pointer_pos: egui::Pos2,
        primary_down: bool,
        primary_released: bool,
    ) -> bool {
        // Convert screen position to image coordinates
        if !image_rect.contains(pointer_pos) {
            return false;
        }

        let rel_x = (pointer_pos.x - image_rect.min.x) / image_rect.width();
        let rel_y = (pointer_pos.y - image_rect.min.y) / image_rect.height();
        let image_pos = egui::pos2(rel_x * image_rect.width(), rel_y * image_rect.height());

        // Start drawing a new line
        if primary_down && !self.drawing && self.temp_line_start.is_none() {
            // Check if clicking near an existing line
            let mut clicked_line = false;
            for line in &mut self.lines {
                if line.is_near(image_pos, 10.0) {
                    line.editing = !line.editing;
                    clicked_line = true;
                    break;
                }
            }

            if !clicked_line {
                self.temp_line_start = Some(image_pos);
                self.drawing = true;
            }
            return true;
        }

        // Finish drawing the line
        if primary_released && self.drawing {
            if let Some(start) = self.temp_line_start {
                let color = self.next_color();
                let label = format!("Line {}", self.lines.len() + 1);
                let line = LineSelection::new(start, image_pos, color, label);
                self.lines.push(line);
            }
            self.temp_line_start = None;
            self.drawing = false;
            return true;
        }

        false
    }

    /// Draw line overlays on the image
    pub fn draw_overlays(
        &self,
        ui: &mut egui::Ui,
        image_rect: egui::Rect,
        _current_mouse_pos: Option<egui::Pos2>,
    ) {
        let painter = ui.painter();

        // Draw existing lines
        for line in &self.lines {
            let screen_start = egui::pos2(
                image_rect.min.x + (line.start.x / image_rect.width()) * image_rect.width(),
                image_rect.min.y + (line.start.y / image_rect.height()) * image_rect.height(),
            );
            let screen_end = egui::pos2(
                image_rect.min.x + (line.end.x / image_rect.width()) * image_rect.width(),
                image_rect.min.y + (line.end.y / image_rect.height()) * image_rect.height(),
            );

            let stroke = egui::Stroke::new(2.0, line.color);
            painter.line_segment([screen_start, screen_end], stroke);

            // Draw endpoints
            painter.circle_filled(screen_start, 4.0, line.color);
            painter.circle_filled(screen_end, 4.0, line.color);

            // Draw label
            painter.text(
                screen_start,
                egui::Align2::LEFT_BOTTOM,
                &line.label,
                egui::FontId::default(),
                egui::Color32::WHITE,
            );
        }

        // Draw temporary line being drawn
        if let Some(start) = self.temp_line_start {
            if let Some(current_pos) = _current_mouse_pos {
                if image_rect.contains(current_pos) {
                    let screen_start = egui::pos2(
                        image_rect.min.x + (start.x / image_rect.width()) * image_rect.width(),
                        image_rect.min.y + (start.y / image_rect.height()) * image_rect.height(),
                    );

                    let stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
                    painter.line_segment([screen_start, current_pos], stroke);
                }
            }
        }
    }

    /// Show the profile plot panel
    pub fn show_plot_panel(&self, ui: &mut egui::Ui) {
        if self.profiles.is_empty() {
            ui.label("No profiles to display. Draw a line on the image.");
            return;
        }

        Plot::new("line_profile_plot")
            .view_aspect(2.0)
            .legend(egui_plot::Legend::default())
            .show(ui, |plot_ui: &mut egui_plot::PlotUi| {
                for profile in &self.profiles {
                    let points: PlotPoints = profile
                        .distances
                        .iter()
                        .zip(profile.intensities.iter())
                        .map(|(&x, &y)| [x, y])
                        .collect();

                    let line = Line::new(&profile.line.label, points).color(profile.line.color);

                    plot_ui.line(line);
                }
            });
    }

    /// Show statistics panel
    pub fn show_stats_panel(&mut self, ui: &mut egui::Ui) {
        if self.profiles.is_empty() {
            return;
        }

        ui.heading("Profile Statistics");

        for (i, profile) in self.profiles.iter().enumerate() {
            ui.collapsing(&profile.line.label, |ui| {
                egui::Grid::new(format!("stats_grid_{}", i))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Min:");
                        ui.label(format!("{:.2}", profile.stats.min));
                        ui.end_row();

                        ui.label("Max:");
                        ui.label(format!("{:.2}", profile.stats.max));
                        ui.end_row();

                        ui.label("Mean:");
                        ui.label(format!("{:.2}", profile.stats.mean));
                        ui.end_row();

                        ui.label("FWHM:");
                        if let Some(fwhm) = profile.stats.fwhm {
                            ui.label(format!("{:.2} px", fwhm));
                        } else {
                            ui.label("N/A");
                        }
                        ui.end_row();
                    });

                if ui.button("Export to CSV").clicked() {
                    let csv = profile.to_csv();
                    // Copy to clipboard
                    ui.ctx().copy_text(csv);
                    ui.label("✓ Copied to clipboard");
                }
            });
        }

        ui.separator();

        if ui.button("Clear All Lines").clicked() {
            self.lines.clear();
            self.profiles.clear();
        }
    }

    /// Get number of active lines
    pub fn num_lines(&self) -> usize {
        self.lines.len()
    }

    /// Clear all lines and profiles
    pub fn clear(&mut self) {
        self.lines.clear();
        self.profiles.clear();
        self.drawing = false;
        self.temp_line_start = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_selection_length() {
        let line = LineSelection::new(
            egui::pos2(0.0, 0.0),
            egui::pos2(3.0, 4.0),
            egui::Color32::RED,
            "Test".to_string(),
        );
        assert!((line.length() - 5.0).abs() < 0.01); // 3-4-5 triangle
    }

    #[test]
    fn test_line_selection_is_near() {
        let line = LineSelection::new(
            egui::pos2(0.0, 0.0),
            egui::pos2(10.0, 0.0),
            egui::Color32::RED,
            "Test".to_string(),
        );

        // Point on the line
        assert!(line.is_near(egui::pos2(5.0, 0.0), 1.0));

        // Point near the line
        assert!(line.is_near(egui::pos2(5.0, 0.5), 1.0));

        // Point far from the line
        assert!(!line.is_near(egui::pos2(5.0, 5.0), 1.0));
    }

    #[test]
    fn test_intensity_profile_extraction_8bit() {
        // Create a simple 4x4 gradient image (8-bit)
        let image_data: Vec<u8> = (0..16).map(|i| i * 16).collect();
        let width = 4;
        let height = 4;
        let bit_depth = 8;

        // Extract profile along first row (should be 0, 16, 32, 48)
        let line = LineSelection::new(
            egui::pos2(0.0, 0.0),
            egui::pos2(3.0, 0.0),
            egui::Color32::RED,
            "Test".to_string(),
        );

        let profile = IntensityProfile::extract(&line, &image_data, width, height, bit_depth);

        // Check that we got reasonable values
        assert!(!profile.intensities.is_empty());
        assert!(profile.stats.min >= 0.0);
        assert!(profile.stats.max <= 255.0);
        assert!(profile.stats.mean > 0.0);
    }

    #[test]
    fn test_intensity_profile_extraction_16bit() {
        // Create a simple 4x4 gradient image (16-bit, stored as little-endian bytes)
        let mut image_data = Vec::new();
        for i in 0..16 {
            let pixel = (i * 256) as u16;
            image_data.extend_from_slice(&pixel.to_le_bytes());
        }
        let width = 4;
        let height = 4;
        let bit_depth = 16;

        let line = LineSelection::new(
            egui::pos2(0.0, 0.0),
            egui::pos2(3.0, 0.0),
            egui::Color32::RED,
            "Test".to_string(),
        );

        let profile = IntensityProfile::extract(&line, &image_data, width, height, bit_depth);

        assert!(!profile.intensities.is_empty());
        assert!(profile.stats.min >= 0.0);
        assert!(profile.stats.max <= 65535.0);
    }

    #[test]
    fn test_profile_stats_computation() {
        let intensities = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let stats = IntensityProfile::compute_stats(&intensities);

        assert_eq!(stats.min, 1.0);
        assert_eq!(stats.max, 5.0);
        assert_eq!(stats.mean, 3.0);
    }

    #[test]
    fn test_fwhm_computation() {
        // Create a peak profile: [0, 1, 2, 3, 4, 5, 4, 3, 2, 1, 0]
        let intensities: Vec<f64> = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0];
        let max = 5.0;
        let min = 0.0;

        let fwhm = IntensityProfile::compute_fwhm(&intensities, max, min);

        // Half-max is 2.5, so FWHM should span from index 3 to 7 (width of 4)
        // Indices with values >= 2.5 are: 3, 4, 5, 6, 7
        assert!(fwhm.is_some());
        assert!((fwhm.unwrap() - 4.0).abs() < 0.1);
    }

    #[test]
    fn test_csv_export() {
        let line = LineSelection::new(
            egui::pos2(0.0, 0.0),
            egui::pos2(2.0, 0.0),
            egui::Color32::RED,
            "Test".to_string(),
        );

        let profile = IntensityProfile {
            distances: vec![0.0, 1.0, 2.0],
            intensities: vec![10.0, 20.0, 30.0],
            line,
            stats: ProfileStats::default(),
        };

        let csv = profile.to_csv();
        assert!(csv.contains("Distance (px),Intensity"));
        assert!(csv.contains("0.00,10.00"));
        assert!(csv.contains("1.00,20.00"));
        assert!(csv.contains("2.00,30.00"));
    }

    #[test]
    fn test_widget_creation() {
        let widget = LineProfileWidget::new();
        assert_eq!(widget.num_lines(), 0);
        assert_eq!(widget.profiles.len(), 0);
        assert!(widget.show_plot);
        assert!(widget.show_stats);
    }

    #[test]
    fn test_widget_color_cycling() {
        let mut widget = LineProfileWidget::new();
        let num_colors = widget.color_palette.len();

        // Get colors and verify they cycle
        let color1 = widget.next_color();
        for _ in 1..num_colors {
            widget.next_color();
        }
        let color_after_cycle = widget.next_color();

        assert_eq!(color1, color_after_cycle);
    }
}
