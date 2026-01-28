//! ROI (Region of Interest) selector widget for image analysis
//!
//! Supports multiple ROI types:
//! - Rectangle: Drag-to-select rectangular regions
//! - Polygon: Shift+click to add vertices, drag to adjust, right-click to delete
//!
//! Features:
//! - Multiple named ROIs with distinct colors
//! - Live statistics (mean, std dev, min, max, area, perimeter)
//! - Export coordinates
//! - Works with different bit depths (8, 12, 16-bit)

use eframe::egui;

/// Color palette for multiple ROIs
const ROI_COLORS: &[(u8, u8, u8)] = &[
    (255, 200, 100), // Orange (default)
    (100, 150, 255), // Blue
    (100, 255, 150), // Green
    (255, 100, 200), // Pink
    (200, 100, 255), // Purple
    (255, 255, 100), // Yellow
    (100, 255, 255), // Cyan
    (255, 150, 100), // Coral
];

/// ROI shape type
#[derive(Debug, Clone, PartialEq)]
pub enum RoiShape {
    /// Rectangular ROI (x, y, width, height)
    Rectangle {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Polygonal ROI (vertices in pixel coordinates)
    Polygon { vertices: Vec<(f32, f32)> },
}

impl RoiShape {
    /// Check if ROI has non-zero area
    pub fn is_valid(&self) -> bool {
        match self {
            RoiShape::Rectangle { width, height, .. } => *width > 0 && *height > 0,
            RoiShape::Polygon { vertices } => vertices.len() >= 3,
        }
    }

    /// Calculate area in pixels
    pub fn area(&self) -> f64 {
        match self {
            RoiShape::Rectangle { width, height, .. } => (*width * *height) as f64,
            RoiShape::Polygon { vertices } => {
                if vertices.len() < 3 {
                    return 0.0;
                }
                // Shoelace formula
                let mut sum = 0.0;
                for i in 0..vertices.len() {
                    let j = (i + 1) % vertices.len();
                    sum += (vertices[i].0 * vertices[j].1) as f64;
                    sum -= (vertices[j].0 * vertices[i].1) as f64;
                }
                (sum.abs() / 2.0)
            }
        }
    }

    /// Calculate perimeter in pixels
    pub fn perimeter(&self) -> f64 {
        match self {
            RoiShape::Rectangle { width, height, .. } => 2.0 * (*width + *height) as f64,
            RoiShape::Polygon { vertices } => {
                if vertices.len() < 2 {
                    return 0.0;
                }
                let mut sum = 0.0;
                for i in 0..vertices.len() {
                    let j = (i + 1) % vertices.len();
                    let dx = vertices[j].0 - vertices[i].0;
                    let dy = vertices[j].1 - vertices[i].1;
                    sum += ((dx * dx + dy * dy) as f64).sqrt();
                }
                sum
            }
        }
    }

    /// Calculate centroid
    pub fn centroid(&self) -> (f32, f32) {
        match self {
            RoiShape::Rectangle {
                x,
                y,
                width,
                height,
            } => (
                *x as f32 + *width as f32 / 2.0,
                *y as f32 + *height as f32 / 2.0,
            ),
            RoiShape::Polygon { vertices } => {
                if vertices.is_empty() {
                    return (0.0, 0.0);
                }
                let sum_x: f32 = vertices.iter().map(|v| v.0).sum();
                let sum_y: f32 = vertices.iter().map(|v| v.1).sum();
                (sum_x / vertices.len() as f32, sum_y / vertices.len() as f32)
            }
        }
    }

    /// Clamp ROI to image dimensions
    pub fn clamp_to_image(&self, image_width: u32, image_height: u32) -> Self {
        match self {
            RoiShape::Rectangle {
                x,
                y,
                width,
                height,
            } => {
                let x = (*x).min(image_width.saturating_sub(1));
                let y = (*y).min(image_height.saturating_sub(1));
                let width = (*width).min(image_width.saturating_sub(x));
                let height = (*height).min(image_height.saturating_sub(y));
                RoiShape::Rectangle {
                    x,
                    y,
                    width,
                    height,
                }
            }
            RoiShape::Polygon { vertices } => {
                let clamped = vertices
                    .iter()
                    .map(|(vx, vy)| {
                        let cx = vx.max(0.0).min(image_width as f32 - 1.0);
                        let cy = vy.max(0.0).min(image_height as f32 - 1.0);
                        (cx, cy)
                    })
                    .collect();
                RoiShape::Polygon { vertices: clamped }
            }
        }
    }

    /// Get bounding box (min_x, min_y, max_x, max_y)
    pub fn bounding_box(&self) -> (u32, u32, u32, u32) {
        match self {
            RoiShape::Rectangle {
                x,
                y,
                width,
                height,
            } => (*x, *y, x + width, y + height),
            RoiShape::Polygon { vertices } => {
                if vertices.is_empty() {
                    return (0, 0, 0, 0);
                }
                let min_x = vertices.iter().map(|v| v.0).fold(f32::INFINITY, f32::min);
                let min_y = vertices.iter().map(|v| v.1).fold(f32::INFINITY, f32::min);
                let max_x = vertices
                    .iter()
                    .map(|v| v.0)
                    .fold(f32::NEG_INFINITY, f32::max);
                let max_y = vertices
                    .iter()
                    .map(|v| v.1)
                    .fold(f32::NEG_INFINITY, f32::max);
                (
                    min_x as u32,
                    min_y as u32,
                    max_x.ceil() as u32,
                    max_y.ceil() as u32,
                )
            }
        }
    }

    /// Check if point is inside the ROI
    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        match self {
            RoiShape::Rectangle {
                x,
                y,
                width,
                height,
            } => {
                let x = *x as f32;
                let y = *y as f32;
                px >= x && px < x + *width as f32 && py >= y && py < y + *height as f32
            }
            RoiShape::Polygon { vertices } => {
                if vertices.len() < 3 {
                    return false;
                }
                // Ray casting algorithm
                let mut inside = false;
                let mut j = vertices.len() - 1;
                for i in 0..vertices.len() {
                    let vi = vertices[i];
                    let vj = vertices[j];
                    if ((vi.1 > py) != (vj.1 > py))
                        && (px < (vj.0 - vi.0) * (py - vi.1) / (vj.1 - vi.1) + vi.0)
                    {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
        }
    }
}

/// A named ROI with shape and color
#[derive(Debug, Clone)]
pub struct NamedRoi {
    pub name: String,
    pub shape: RoiShape,
    pub color_index: usize,
}

impl NamedRoi {
    pub fn new(name: String, shape: RoiShape, color_index: usize) -> Self {
        Self {
            name,
            shape,
            color_index,
        }
    }

    pub fn color(&self) -> egui::Color32 {
        let (r, g, b) = ROI_COLORS[self.color_index % ROI_COLORS.len()];
        egui::Color32::from_rgb(r, g, b)
    }

    pub fn fill_color(&self) -> egui::Color32 {
        let (r, g, b) = ROI_COLORS[self.color_index % ROI_COLORS.len()];
        egui::Color32::from_rgba_unmultiplied(r, g, b, 30)
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
    pub area: f64,
    pub perimeter: f64,
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

        let variance: f64 = pixels
            .iter()
            .map(|&p| (p as f64 - mean).powi(2))
            .sum::<f64>()
            / n as f64;

        Self {
            mean,
            std_dev: variance.sqrt(),
            min: min as f64,
            max: max as f64,
            pixel_count: n,
            area: n as f64,
            perimeter: 0.0,
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

        let variance: f64 = pixels
            .iter()
            .map(|&p| (p as f64 - mean).powi(2))
            .sum::<f64>()
            / n as f64;

        Self {
            mean,
            std_dev: variance.sqrt(),
            min: min as f64,
            max: max as f64,
            pixel_count: n,
            area: n as f64,
            perimeter: 0.0,
        }
    }

    /// Compute statistics from raw frame data with given bit depth
    ///
    /// # Arguments
    /// * `data` - Raw pixel data
    /// * `image_width` - Image width in pixels
    /// * `image_height` - Image height in pixels
    /// * `bit_depth` - Bits per pixel (8, 12, or 16)
    /// * `shape` - ROI shape to analyze
    pub fn from_frame_roi(
        data: &[u8],
        image_width: u32,
        image_height: u32,
        bit_depth: u32,
        shape: &RoiShape,
    ) -> Self {
        // Clamp ROI to actual image dimensions
        let shape = shape.clamp_to_image(image_width, image_height);
        if !shape.is_valid() || image_width == 0 || image_height == 0 {
            return Self::default();
        }

        let area = shape.area();
        let perimeter = shape.perimeter();

        // Get bounding box for pixel extraction
        let (min_x, min_y, max_x, max_y) = shape.bounding_box();

        match bit_depth {
            8 => {
                let mut pixels = Vec::new();
                for y in min_y..max_y {
                    for x in min_x..max_x {
                        if shape.contains_point(x as f32 + 0.5, y as f32 + 0.5) {
                            let idx = (y * image_width + x) as usize;
                            if idx < data.len() {
                                pixels.push(data[idx]);
                            }
                        }
                    }
                }
                let mut stats = Self::from_u8_pixels(&pixels);
                stats.area = area;
                stats.perimeter = perimeter;
                stats
            }
            12 | 16 => {
                let mut pixels = Vec::new();
                for y in min_y..max_y {
                    for x in min_x..max_x {
                        if shape.contains_point(x as f32 + 0.5, y as f32 + 0.5) {
                            let pixel_idx = (y * image_width + x) as usize;
                            let byte_idx = pixel_idx * 2;
                            if byte_idx + 1 < data.len() {
                                let pixel =
                                    u16::from_le_bytes([data[byte_idx], data[byte_idx + 1]]);
                                pixels.push(pixel);
                            }
                        }
                    }
                }
                let mut stats = Self::from_u16_pixels(&pixels);
                stats.area = area;
                stats.perimeter = perimeter;
                stats
            }
            _ => Self::default(),
        }
    }
}

/// ROI selection mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiMode {
    Rectangle,
    Polygon,
}

/// Polygon editing state
#[derive(Debug, Default)]
struct PolygonEditor {
    /// Vertices being added (before polygon is closed)
    temp_vertices: Vec<(f32, f32)>,
    /// Index of vertex being dragged (if any)
    dragging_vertex: Option<usize>,
    /// Index of vertex being hovered
    hovered_vertex: Option<usize>,
}

/// ROI selection state for the image viewer
#[derive(Debug)]
pub struct RoiSelector {
    /// List of named ROIs
    rois: Vec<NamedRoi>,
    /// Currently selected ROI index
    selected_roi: Option<usize>,
    /// Statistics for selected ROI
    stats: Option<RoiStatistics>,
    /// Selection in progress (for rectangles)
    drag_start: Option<(i32, i32)>,
    /// Current drag position (for rectangles)
    drag_current: Option<(i32, i32)>,
    /// Polygon editor state
    polygon_editor: PolygonEditor,
    /// Current ROI mode
    pub mode: RoiMode,
    /// Is selection mode active
    pub selection_mode: bool,
    /// Counter for naming ROIs
    roi_counter: usize,
}

impl Default for RoiSelector {
    fn default() -> Self {
        Self {
            rois: Vec::new(),
            selected_roi: None,
            stats: None,
            drag_start: None,
            drag_current: None,
            polygon_editor: PolygonEditor::default(),
            mode: RoiMode::Rectangle,
            selection_mode: false,
            roi_counter: 0,
        }
    }
}

impl RoiSelector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get current ROI if valid (for backward compatibility)
    pub fn roi(&self) -> Option<&RoiShape> {
        self.selected_roi
            .and_then(|idx| self.rois.get(idx).map(|r| &r.shape))
    }

    /// Get all ROIs
    pub fn rois(&self) -> &[NamedRoi] {
        &self.rois
    }

    /// Get selected ROI
    pub fn selected_roi(&self) -> Option<&NamedRoi> {
        self.selected_roi.and_then(|idx| self.rois.get(idx))
    }

    /// Get current statistics if available
    pub fn statistics(&self) -> Option<&RoiStatistics> {
        self.stats.as_ref()
    }

    /// Clear current selection
    pub fn clear(&mut self) {
        self.selected_roi = None;
        self.stats = None;
        self.drag_start = None;
        self.drag_current = None;
        self.polygon_editor = PolygonEditor::default();
    }

    /// Clear all ROIs
    pub fn clear_all(&mut self) {
        self.rois.clear();
        self.clear();
    }

    /// Delete selected ROI
    pub fn delete_selected(&mut self) {
        if let Some(idx) = self.selected_roi {
            if idx < self.rois.len() {
                self.rois.remove(idx);
                self.selected_roi = if self.rois.is_empty() {
                    None
                } else {
                    Some(idx.min(self.rois.len() - 1))
                };
                self.stats = None;
            }
        }
    }

    /// Update statistics from new frame data (same ROI)
    pub fn update_statistics(
        &mut self,
        frame_data: &[u8],
        image_width: u32,
        image_height: u32,
        bit_depth: u32,
    ) {
        if let Some(roi) = self.selected_roi() {
            let stats = RoiStatistics::from_frame_roi(
                frame_data,
                image_width,
                image_height,
                bit_depth,
                &roi.shape,
            );
            self.stats = Some(stats);
        }
    }

    /// Set ROI from shape (for backward compatibility with existing code)
    pub fn set_roi_from_frame(
        &mut self,
        shape: RoiShape,
        frame_data: &[u8],
        image_width: u32,
        image_height: u32,
        bit_depth: u32,
    ) {
        self.roi_counter += 1;
        let name = format!("ROI {}", self.roi_counter);
        let color_index = self.rois.len() % ROI_COLORS.len();
        let roi = NamedRoi::new(name, shape, color_index);

        let stats = RoiStatistics::from_frame_roi(
            frame_data,
            image_width,
            image_height,
            bit_depth,
            &roi.shape,
        );

        self.rois.push(roi);
        self.selected_roi = Some(self.rois.len() - 1);
        self.stats = Some(stats);
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

        let image_offset = (image_rect.size()
            - egui::vec2(image_size.0 as f32 * zoom, image_size.1 as f32 * zoom))
            / 2.0
            + pan;

        // Convert screen position to pixel coordinates
        let screen_to_pixel = |pos: egui::Pos2| -> (f32, f32) {
            let relative = pos - image_rect.min - image_offset;
            let pixel_x = relative.x / zoom;
            let pixel_y = relative.y / zoom;
            (pixel_x, pixel_y)
        };

        let mut finalized = false;

        match self.mode {
            RoiMode::Rectangle => {
                // Handle rectangle drag
                if response.drag_started_by(egui::PointerButton::Primary) {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (px, py) = screen_to_pixel(pos);
                        self.drag_start = Some((px as i32, py as i32));
                        self.drag_current = self.drag_start;
                    }
                }

                if response.dragged_by(egui::PointerButton::Primary) {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (px, py) = screen_to_pixel(pos);
                        self.drag_current = Some((px as i32, py as i32));
                    }
                }

                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some(start), Some(end)) = (self.drag_start, self.drag_current) {
                        let x = start.0.min(end.0).max(0) as u32;
                        let y = start.1.min(end.1).max(0) as u32;
                        let width = (start.0 - end.0).unsigned_abs();
                        let height = (start.1 - end.1).unsigned_abs();

                        if width > 0 && height > 0 {
                            let shape = RoiShape::Rectangle {
                                x,
                                y,
                                width,
                                height,
                            };
                            let shape = shape.clamp_to_image(image_size.0, image_size.1);

                            if shape.is_valid() {
                                self.roi_counter += 1;
                                let name = format!("ROI {}", self.roi_counter);
                                let color_index = self.rois.len() % ROI_COLORS.len();
                                let roi = NamedRoi::new(name, shape, color_index);
                                self.rois.push(roi);
                                self.selected_roi = Some(self.rois.len() - 1);
                                finalized = true;
                            }
                        }
                    }
                    self.drag_start = None;
                    self.drag_current = None;
                }
            }
            RoiMode::Polygon => {
                // Handle polygon vertex manipulation
                let shift_held = response.ctx.input(|i| i.modifiers.shift);

                if let Some(pos) = response.hover_pos() {
                    let (px, py) = screen_to_pixel(pos);

                    // Check for vertex hover
                    self.polygon_editor.hovered_vertex = None;
                    for (i, &(vx, vy)) in self.polygon_editor.temp_vertices.iter().enumerate() {
                        let dx = (vx - px) * zoom;
                        let dy = (vy - py) * zoom;
                        if dx * dx + dy * dy < 25.0 {
                            // 5 pixel radius in screen space
                            self.polygon_editor.hovered_vertex = Some(i);
                            break;
                        }
                    }
                }

                // Shift+click to add vertex
                if shift_held && response.clicked_by(egui::PointerButton::Primary) {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (px, py) = screen_to_pixel(pos);

                        // Check if clicking near first vertex to close polygon
                        if self.polygon_editor.temp_vertices.len() >= 3 {
                            let first = self.polygon_editor.temp_vertices[0];
                            let dx = (first.0 - px) * zoom;
                            let dy = (first.1 - py) * zoom;
                            if dx * dx + dy * dy < 25.0 {
                                // Close polygon
                                let shape = RoiShape::Polygon {
                                    vertices: self.polygon_editor.temp_vertices.clone(),
                                };
                                let shape = shape.clamp_to_image(image_size.0, image_size.1);

                                if shape.is_valid() {
                                    self.roi_counter += 1;
                                    let name = format!("ROI {}", self.roi_counter);
                                    let color_index = self.rois.len() % ROI_COLORS.len();
                                    let roi = NamedRoi::new(name, shape, color_index);
                                    self.rois.push(roi);
                                    self.selected_roi = Some(self.rois.len() - 1);
                                    self.polygon_editor.temp_vertices.clear();
                                    finalized = true;
                                }
                                return finalized;
                            }
                        }

                        // Add new vertex
                        self.polygon_editor.temp_vertices.push((px, py));
                    }
                }

                // Drag vertex to adjust
                if !shift_held && response.drag_started_by(egui::PointerButton::Primary) {
                    if let Some(idx) = self.polygon_editor.hovered_vertex {
                        self.polygon_editor.dragging_vertex = Some(idx);
                    }
                }

                if !shift_held && response.dragged_by(egui::PointerButton::Primary) {
                    if let (Some(idx), Some(pos)) = (
                        self.polygon_editor.dragging_vertex,
                        response.interact_pointer_pos(),
                    ) {
                        let (px, py) = screen_to_pixel(pos);
                        if idx < self.polygon_editor.temp_vertices.len() {
                            self.polygon_editor.temp_vertices[idx] = (px, py);
                        }
                    }
                }

                if response.drag_stopped() {
                    self.polygon_editor.dragging_vertex = None;
                }

                // Right-click or backspace to delete vertex
                if response.clicked_by(egui::PointerButton::Secondary) {
                    if let Some(idx) = self.polygon_editor.hovered_vertex {
                        self.polygon_editor.temp_vertices.remove(idx);
                    }
                } else if response.ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
                    if !self.polygon_editor.temp_vertices.is_empty() {
                        self.polygon_editor.temp_vertices.pop();
                    }
                }
            }
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
        let image_offset = (image_rect.size()
            - egui::vec2(image_size.0 as f32 * zoom, image_size.1 as f32 * zoom))
            / 2.0
            + pan;

        // Convert pixel coordinates to screen position
        let pixel_to_screen = |px: f32, py: f32| -> egui::Pos2 {
            image_rect.min + image_offset + egui::vec2(px * zoom, py * zoom)
        };

        // Draw all finalized ROIs
        for (idx, roi) in self.rois.iter().enumerate() {
            let is_selected = self.selected_roi == Some(idx);
            let stroke_width = if is_selected { 3.0 } else { 2.0 };

            match &roi.shape {
                RoiShape::Rectangle {
                    x,
                    y,
                    width,
                    height,
                } => {
                    let p1 = pixel_to_screen(*x as f32, *y as f32);
                    let p2 = pixel_to_screen((*x + *width) as f32, (*y + *height) as f32);
                    let rect = egui::Rect::from_two_pos(p1, p2);

                    painter.rect_filled(rect, 0.0, roi.fill_color());
                    painter.rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(stroke_width, roi.color()),
                        egui::StrokeKind::Outside,
                    );

                    // Draw label at top-left
                    if is_selected {
                        let text_pos = p1 - egui::vec2(0.0, 16.0);
                        painter.text(
                            text_pos,
                            egui::Align2::LEFT_BOTTOM,
                            &roi.name,
                            egui::FontId::default(),
                            roi.color(),
                        );
                    }
                }
                RoiShape::Polygon { vertices } => {
                    if vertices.len() < 2 {
                        continue;
                    }

                    // Draw filled polygon
                    let screen_points: Vec<egui::Pos2> = vertices
                        .iter()
                        .map(|(x, y)| pixel_to_screen(*x, *y))
                        .collect();

                    painter.add(egui::Shape::convex_polygon(
                        screen_points.clone(),
                        roi.fill_color(),
                        egui::Stroke::NONE,
                    ));

                    // Draw polygon edges
                    for i in 0..vertices.len() {
                        let j = (i + 1) % vertices.len();
                        let p1 = pixel_to_screen(vertices[i].0, vertices[i].1);
                        let p2 = pixel_to_screen(vertices[j].0, vertices[j].1);
                        painter
                            .line_segment([p1, p2], egui::Stroke::new(stroke_width, roi.color()));
                    }

                    // Draw vertices
                    for (vx, vy) in vertices {
                        let pos = pixel_to_screen(*vx, *vy);
                        painter.circle_filled(pos, 4.0, roi.color());
                    }

                    // Draw label at centroid
                    if is_selected {
                        let (cx, cy) = roi.shape.centroid();
                        let text_pos = pixel_to_screen(cx, cy);
                        painter.text(
                            text_pos,
                            egui::Align2::CENTER_CENTER,
                            &roi.name,
                            egui::FontId::default(),
                            roi.color(),
                        );

                        // Draw hover stats at centroid
                        if let Some(stats) = &self.stats {
                            let stats_text = format!(
                                "Area: {:.0} px²\nPerim: {:.1} px",
                                stats.area, stats.perimeter
                            );
                            let stats_pos = text_pos + egui::vec2(0.0, 20.0);
                            painter.text(
                                stats_pos,
                                egui::Align2::CENTER_TOP,
                                stats_text,
                                egui::FontId::monospace(12.0),
                                roi.color(),
                            );
                        }
                    }
                }
            }
        }

        // Draw in-progress rectangle selection
        if let (Some(start), Some(current)) = (self.drag_start, self.drag_current) {
            let p1 = pixel_to_screen(start.0.max(0) as f32, start.1.max(0) as f32);
            let p2 = pixel_to_screen(current.0.max(0) as f32, current.1.max(0) as f32);
            let rect = egui::Rect::from_two_pos(p1, p2);

            painter.rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(100, 150, 255, 50),
            );
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
                egui::StrokeKind::Outside,
            );
        }

        // Draw in-progress polygon
        if !self.polygon_editor.temp_vertices.is_empty() {
            let color = egui::Color32::from_rgb(100, 150, 255);

            // Draw edges
            for i in 0..self.polygon_editor.temp_vertices.len() {
                let p1 = pixel_to_screen(
                    self.polygon_editor.temp_vertices[i].0,
                    self.polygon_editor.temp_vertices[i].1,
                );

                if i + 1 < self.polygon_editor.temp_vertices.len() {
                    let p2 = pixel_to_screen(
                        self.polygon_editor.temp_vertices[i + 1].0,
                        self.polygon_editor.temp_vertices[i + 1].1,
                    );
                    painter.line_segment([p1, p2], egui::Stroke::new(2.0, color));
                }
            }

            // Draw vertices
            for (i, (vx, vy)) in self.polygon_editor.temp_vertices.iter().enumerate() {
                let pos = pixel_to_screen(*vx, *vy);
                let is_hovered = self.polygon_editor.hovered_vertex == Some(i);
                let radius = if is_hovered { 6.0 } else { 4.0 };
                painter.circle_filled(pos, radius, color);

                // Highlight first vertex when polygon can be closed
                if i == 0 && self.polygon_editor.temp_vertices.len() >= 3 {
                    painter.circle_stroke(pos, 8.0, egui::Stroke::new(2.0, color));
                }
            }

            // Draw closing line preview when hovering near first vertex
            if self.polygon_editor.temp_vertices.len() >= 3 {
                if let Some(hover_pos) = painter.ctx().pointer_hover_pos() {
                    let (px, py) = {
                        let relative = hover_pos - image_rect.min - image_offset;
                        (relative.x / zoom, relative.y / zoom)
                    };
                    let first = self.polygon_editor.temp_vertices[0];
                    let dx = (first.0 - px) * zoom;
                    let dy = (first.1 - py) * zoom;
                    if dx * dx + dy * dy < 25.0 {
                        let last_idx = self.polygon_editor.temp_vertices.len() - 1;
                        let p1 = pixel_to_screen(
                            self.polygon_editor.temp_vertices[last_idx].0,
                            self.polygon_editor.temp_vertices[last_idx].1,
                        );
                        let p2 = pixel_to_screen(first.0, first.1);
                        painter.line_segment(
                            [p1, p2],
                            egui::Stroke::new(
                                2.0,
                                egui::Color32::from_rgba_unmultiplied(100, 150, 255, 128),
                            ),
                        );
                    }
                }
            }
        }
    }

    /// Render statistics panel
    pub fn show_statistics_panel(&self, ui: &mut egui::Ui) {
        if let (Some(roi), Some(stats)) = (self.selected_roi(), self.statistics()) {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&roi.name).color(roi.color()).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Delete").clicked() {
                            // Signal deletion (handled by caller)
                        }
                    });
                });
                ui.separator();

                egui::Grid::new("roi_stats_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        match &roi.shape {
                            RoiShape::Rectangle {
                                x,
                                y,
                                width,
                                height,
                            } => {
                                ui.label("Position:");
                                ui.label(format!("({}, {})", x, y));
                                ui.end_row();

                                ui.label("Size:");
                                ui.label(format!("{}x{}", width, height));
                                ui.end_row();
                            }
                            RoiShape::Polygon { vertices } => {
                                ui.label("Vertices:");
                                ui.label(format!("{}", vertices.len()));
                                ui.end_row();

                                let (cx, cy) = roi.shape.centroid();
                                ui.label("Centroid:");
                                ui.label(format!("({:.1}, {:.1})", cx, cy));
                                ui.end_row();
                            }
                        }

                        ui.label("Area:");
                        ui.label(format!("{:.1} px²", stats.area));
                        ui.end_row();

                        ui.label("Perimeter:");
                        ui.label(format!("{:.1} px", stats.perimeter));
                        ui.end_row();

                        ui.separator();
                        ui.end_row();

                        ui.label("Pixels:");
                        ui.label(format!("{}", stats.pixel_count));
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
                match self.mode {
                    RoiMode::Rectangle => {
                        ui.label("Drag on image to select rectangle ROI");
                    }
                    RoiMode::Polygon => {
                        ui.label("Shift+click to add vertices");
                        ui.label("Click near first point to close polygon");
                        ui.label("Drag vertices to adjust shape");
                        ui.label("Right-click or backspace to delete vertex");
                    }
                }
            }
        }

        // Show ROI list
        if !self.rois.is_empty() {
            ui.separator();
            ui.label("All ROIs:");
            for (idx, roi) in self.rois.iter().enumerate() {
                let is_selected = self.selected_roi == Some(idx);
                ui.horizontal(|ui| {
                    let color = roi.color();
                    ui.colored_label(color, "●");
                    if ui.selectable_label(is_selected, &roi.name).clicked() {
                        // Signal selection change (handled by caller)
                    }
                });
            }
        }
    }

    /// Export ROI coordinates
    pub fn export_coordinates(&self) -> String {
        let mut output = String::new();
        for roi in &self.rois {
            output.push_str(&format!("{}\n", roi.name));
            match &roi.shape {
                RoiShape::Rectangle {
                    x,
                    y,
                    width,
                    height,
                } => {
                    output.push_str(&format!(
                        "  Rectangle: x={}, y={}, width={}, height={}\n",
                        x, y, width, height
                    ));
                }
                RoiShape::Polygon { vertices } => {
                    output.push_str("  Polygon vertices:\n");
                    for (i, (vx, vy)) in vertices.iter().enumerate() {
                        output.push_str(&format!("    {}: ({:.2}, {:.2})\n", i, vx, vy));
                    }
                }
            }
            output.push('\n');
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polygon_area() {
        // Simple square: (0,0), (10,0), (10,10), (0,10)
        let square = RoiShape::Polygon {
            vertices: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
        };
        assert_eq!(square.area(), 100.0);

        // Triangle: (0,0), (10,0), (5,10)
        let triangle = RoiShape::Polygon {
            vertices: vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)],
        };
        assert_eq!(triangle.area(), 50.0);
    }

    #[test]
    fn test_polygon_perimeter() {
        // Simple square
        let square = RoiShape::Polygon {
            vertices: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
        };
        assert_eq!(square.perimeter(), 40.0);
    }

    #[test]
    fn test_polygon_centroid() {
        // Square centered at (5, 5)
        let square = RoiShape::Polygon {
            vertices: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
        };
        let (cx, cy) = square.centroid();
        assert!((cx - 5.0).abs() < 0.01);
        assert!((cy - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_point_in_polygon() {
        // Simple square
        let square = RoiShape::Polygon {
            vertices: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
        };

        assert!(square.contains_point(5.0, 5.0)); // Inside
        assert!(square.contains_point(0.5, 0.5)); // Inside, near corner
        assert!(!square.contains_point(-1.0, 5.0)); // Outside left
        assert!(!square.contains_point(15.0, 5.0)); // Outside right
        assert!(!square.contains_point(5.0, -1.0)); // Outside top
        assert!(!square.contains_point(5.0, 15.0)); // Outside bottom
    }

    #[test]
    fn test_rectangle_backward_compat() {
        // Ensure Rectangle still works as before
        let rect = RoiShape::Rectangle {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };

        assert!(rect.is_valid());
        assert_eq!(rect.area(), 5000.0);
        assert_eq!(rect.perimeter(), 300.0);

        let (cx, cy) = rect.centroid();
        assert_eq!(cx, 60.0);
        assert_eq!(cy, 45.0);
    }
}
