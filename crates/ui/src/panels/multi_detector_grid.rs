//! Multi-detector grid layout panel for simultaneous visualization.

use egui::{Color32, Stroke, StrokeKind, Ui};
use egui_extras::{Size, StripBuilder};

/// Type of detector visualization.
#[derive(Debug, Clone, PartialEq)]
pub enum DetectorType {
    /// 2D camera image display.
    Camera { device_id: String },
    /// 1D line plot (e.g., spectrometer, time series).
    LinePlot { device_id: String, label: String },
}

/// Individual detector panel configuration.
#[derive(Debug, Clone)]
pub struct DetectorPanel {
    pub detector_type: DetectorType,
    pub title: String,
}

impl DetectorPanel {
    /// Create a new detector panel.
    pub fn new(detector_type: DetectorType, title: impl Into<String>) -> Self {
        Self {
            detector_type,
            title: title.into(),
        }
    }

    /// Create a camera panel.
    pub fn camera(device_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::new(
            DetectorType::Camera {
                device_id: device_id.into(),
            },
            title,
        )
    }

    /// Create a line plot panel.
    pub fn line_plot(
        device_id: impl Into<String>,
        label: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self::new(
            DetectorType::LinePlot {
                device_id: device_id.into(),
                label: label.into(),
            },
            title,
        )
    }
}

/// Multi-detector grid layout panel.
///
/// Automatically arranges N detector panels in a responsive grid layout.
/// Grid dimensions are calculated as: cols = ceil(sqrt(n)), rows = ceil(n / cols).
///
/// # Examples
///
/// ```
/// use ui::panels::{MultiDetectorGrid, DetectorPanel};
///
/// let mut grid = MultiDetectorGrid::new();
/// grid.add_panel(DetectorPanel::camera("cam0", "Camera 0"));
/// grid.add_panel(DetectorPanel::camera("cam1", "Camera 1"));
/// // Creates 1x2 grid
/// ```
#[derive(Default)]
pub struct MultiDetectorGrid {
    panels: Vec<DetectorPanel>,
}

impl MultiDetectorGrid {
    /// Create a new empty grid.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a grid with initial panels.
    #[allow(dead_code)]
    pub fn with_panels(panels: Vec<DetectorPanel>) -> Self {
        Self { panels }
    }

    /// Add a detector panel to the grid.
    pub fn add_panel(&mut self, panel: DetectorPanel) {
        self.panels.push(panel);
    }

    /// Clear all panels from the grid.
    pub fn clear(&mut self) {
        self.panels.clear();
    }

    /// Get the number of panels in the grid.
    #[allow(dead_code)]
    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    /// Get a reference to the panels.
    pub fn panels(&self) -> &[DetectorPanel] {
        &self.panels
    }

    /// Show the grid layout using egui.
    #[allow(dead_code)]
    pub fn show(&mut self, ui: &mut Ui) {
        if self.panels.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No detectors configured");
            });
            return;
        }

        let (cols, rows) = calculate_grid_dimensions(self.panels.len());

        // Outer strip: vertical rows
        StripBuilder::new(ui)
            .size(Size::remainder())
            .vertical(|mut strip| {
                for row_idx in 0..rows {
                    strip.strip(|builder| {
                        // Inner strip: horizontal columns
                        builder.size(Size::remainder()).horizontal(|mut strip| {
                            for col_idx in 0..cols {
                                strip.cell(|ui| {
                                    let panel_idx = row_idx * cols + col_idx;
                                    if panel_idx < self.panels.len() {
                                        self.render_panel(ui, panel_idx);
                                    } else {
                                        self.render_empty_cell(ui);
                                    }
                                });
                            }
                        });
                    });
                }
            });
    }

    /// Render a single detector panel.
    #[allow(dead_code)]
    fn render_panel(&self, ui: &mut Ui, panel_idx: usize) {
        let panel = &self.panels[panel_idx];

        ui.group(|ui| {
            ui.set_min_size(ui.available_size());

            // Header
            ui.heading(&panel.title);
            ui.separator();

            // Content area (placeholder for actual visualization)
            match &panel.detector_type {
                DetectorType::Camera { device_id } => {
                    ui.vertical_centered(|ui| {
                        ui.label(format!("Camera: {}", device_id));
                        ui.label("(Image viewer integration pending)");
                    });
                }
                DetectorType::LinePlot { device_id, label } => {
                    ui.vertical_centered(|ui| {
                        ui.label(format!("Device: {}", device_id));
                        ui.label(format!("Signal: {}", label));
                        ui.label("(Plot integration pending)");
                    });
                }
            }
        });
    }

    /// Render an empty grid cell.
    #[allow(dead_code)]
    fn render_empty_cell(&self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, Color32::from_gray(100)),
            StrokeKind::Outside,
        );
        ui.allocate_rect(rect, egui::Sense::hover());
    }
}

/// Calculate grid dimensions for N panels.
///
/// Uses the formula:
/// - cols = ceil(sqrt(n))
/// - rows = ceil(n / cols)
///
/// This produces roughly square grids:
/// - 1 panel: 1x1
/// - 2 panels: 2x1
/// - 3 panels: 2x2
/// - 4 panels: 2x2
/// - 5 panels: 3x2
/// - 6 panels: 3x2
/// - 7 panels: 3x3
/// - 8 panels: 3x3
/// - 9 panels: 3x3
pub fn calculate_grid_dimensions(count: usize) -> (usize, usize) {
    if count == 0 {
        return (0, 0);
    }

    let cols = (count as f64).sqrt().ceil() as usize;
    let rows = (count as f64 / cols as f64).ceil() as usize;
    (cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_dimensions() {
        assert_eq!(calculate_grid_dimensions(0), (0, 0));
        assert_eq!(calculate_grid_dimensions(1), (1, 1));
        assert_eq!(calculate_grid_dimensions(2), (2, 1));
        assert_eq!(calculate_grid_dimensions(3), (2, 2));
        assert_eq!(calculate_grid_dimensions(4), (2, 2));
        assert_eq!(calculate_grid_dimensions(5), (3, 2));
        assert_eq!(calculate_grid_dimensions(6), (3, 2));
        assert_eq!(calculate_grid_dimensions(7), (3, 3));
        assert_eq!(calculate_grid_dimensions(8), (3, 3));
        assert_eq!(calculate_grid_dimensions(9), (3, 3));
        assert_eq!(calculate_grid_dimensions(10), (4, 3));
        assert_eq!(calculate_grid_dimensions(16), (4, 4));
    }

    #[test]
    fn test_panel_management() {
        let mut grid = MultiDetectorGrid::new();
        assert_eq!(grid.panel_count(), 0);

        grid.add_panel(DetectorPanel::camera("cam0", "Camera 0"));
        assert_eq!(grid.panel_count(), 1);

        grid.add_panel(DetectorPanel::line_plot("pm0", "power", "Power Meter"));
        assert_eq!(grid.panel_count(), 2);

        grid.clear();
        assert_eq!(grid.panel_count(), 0);
    }

    #[test]
    fn test_detector_panel_constructors() {
        let camera = DetectorPanel::camera("cam0", "Test Camera");
        assert_eq!(camera.title, "Test Camera");
        assert!(matches!(camera.detector_type, DetectorType::Camera { .. }));

        let plot = DetectorPanel::line_plot("dev0", "signal", "Test Plot");
        assert_eq!(plot.title, "Test Plot");
        assert!(matches!(plot.detector_type, DetectorType::LinePlot { .. }));
    }

    #[test]
    fn test_with_panels_constructor() {
        let panels = vec![
            DetectorPanel::camera("cam0", "Camera 0"),
            DetectorPanel::camera("cam1", "Camera 1"),
        ];
        let grid = MultiDetectorGrid::with_panels(panels);
        assert_eq!(grid.panel_count(), 2);
    }
}
