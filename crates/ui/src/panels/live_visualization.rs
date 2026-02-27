//! Live Visualization Panel - Unified detector display during acquisition
//!
//! Displays camera frames and line plots in a grid layout with FPS status.
//! Integrates:
//! - Camera frame streaming (via FrameUpdate messages)
//! - Line plot updates (via DataUpdate messages)
//! - MultiDetectorGrid for responsive layout
//! - AutoScalePlot for grow-to-fit plots
//!
//! ## Async Integration Pattern
//!
//! Uses message-passing for thread-safe async updates:
//! - Background tasks send FrameUpdate/DataUpdate via mpsc channels
//! - Panel drains channels each frame and updates state
//! - No mutable borrows cross async boundaries

use crate::time::Instant;
use eframe::egui::{self, TextureHandle};
use egui_plot::{Line, PlotPoints};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc;

use crate::panels::{DetectorPanel, DetectorType, MultiDetectorGrid};
use crate::widgets::AutoScalePlot;

/// Maximum queued frame updates per camera
const MAX_QUEUED_FRAMES: usize = 4;

/// Maximum queued data updates per plot
const MAX_QUEUED_DATA: usize = 100;

/// Maximum plot history (data points)
const MAX_PLOT_HISTORY: usize = 500;

/// FPS tracking window (seconds)
const FPS_WINDOW: f64 = 2.0;

/// Frame update message from camera streaming
#[derive(Debug)]
pub struct FrameUpdate {
    pub device_id: String,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA data
    pub frame_number: u64,
    #[allow(dead_code)]
    pub timestamp_ns: u64,
}

/// Data update message for line plots
#[derive(Debug, Clone)]
pub struct DataUpdate {
    pub device_id: String,
    pub value: f64,
    #[allow(dead_code)]
    pub timestamp_secs: f64,
}

/// Sender handles for pushing updates from async tasks
pub type FrameUpdateSender = mpsc::SyncSender<FrameUpdate>;
pub type DataUpdateSender = mpsc::SyncSender<DataUpdate>;

/// Receiver handles stored in panel
type FrameUpdateReceiver = mpsc::Receiver<FrameUpdate>;
type DataUpdateReceiver = mpsc::Receiver<DataUpdate>;

/// Create a new bounded channel pair for frame updates
pub fn frame_channel() -> (FrameUpdateSender, FrameUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_FRAMES)
}

/// Create a new bounded channel pair for data updates
pub fn data_channel() -> (DataUpdateSender, DataUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_DATA)
}

/// FPS tracking state
#[derive(Debug, Clone)]
struct FpsTracker {
    frame_times: VecDeque<Instant>,
    last_frame_number: u64,
}

impl FpsTracker {
    fn new() -> Self {
        Self {
            frame_times: VecDeque::new(),
            last_frame_number: 0,
        }
    }

    /// Record a new frame and return current FPS
    fn record_frame(&mut self, frame_number: u64) -> f64 {
        let now = Instant::now();
        self.frame_times.push_back(now);
        self.last_frame_number = frame_number;

        // Remove frames older than FPS window
        let fps_window = std::time::Duration::from_secs_f64(FPS_WINDOW);
        let cutoff = now.checked_sub(fps_window).unwrap_or(now);
        while let Some(&time) = self.frame_times.front() {
            if time < cutoff {
                self.frame_times.pop_front();
            } else {
                break;
            }
        }

        // Calculate FPS
        if self.frame_times.len() < 2 {
            0.0
        } else {
            let elapsed = (now - self.frame_times[0]).as_secs_f64();
            if elapsed > 0.0 {
                (self.frame_times.len() - 1) as f64 / elapsed
            } else {
                0.0
            }
        }
    }

    fn current_fps(&self) -> f64 {
        if self.frame_times.len() < 2 {
            return 0.0;
        }

        let now = Instant::now();
        let fps_window = std::time::Duration::from_secs_f64(FPS_WINDOW);
        let cutoff = now.checked_sub(fps_window).unwrap_or(now);
        let recent_frames: Vec<_> = self.frame_times.iter().filter(|&&t| t >= cutoff).collect();

        if recent_frames.len() < 2 {
            return 0.0;
        }

        let elapsed = (now - *recent_frames[0]).as_secs_f64();
        if elapsed > 0.0 {
            (recent_frames.len() - 1) as f64 / elapsed
        } else {
            0.0
        }
    }
}

/// Camera state for live visualization
struct CameraState {
    device_id: String,
    texture: Option<TextureHandle>,
    fps_tracker: FpsTracker,
    width: u32,
    height: u32,
}

impl CameraState {
    fn new(device_id: String) -> Self {
        Self {
            device_id,
            texture: None,
            fps_tracker: FpsTracker::new(),
            width: 0,
            height: 0,
        }
    }

    /// Update with new frame data
    fn update_frame(&mut self, ctx: &egui::Context, frame: FrameUpdate) {
        self.width = frame.width;
        self.height = frame.height;

        // Update texture
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.data,
        );

        match &mut self.texture {
            Some(texture) => {
                texture.set(image, Default::default());
            }
            None => {
                self.texture = Some(ctx.load_texture(
                    format!("camera_{}", self.device_id),
                    image,
                    Default::default(),
                ));
            }
        }

        // Track FPS
        self.fps_tracker.record_frame(frame.frame_number);
    }

    fn current_fps(&self) -> f64 {
        self.fps_tracker.current_fps()
    }
}

/// Plot state for live visualization
struct PlotState {
    #[allow(dead_code)]
    device_id: String,
    #[allow(dead_code)]
    label: String,
    data: VecDeque<[f64; 2]>, // [time, value] pairs
    plot: AutoScalePlot,
    start_time: Instant,
}

impl PlotState {
    fn new(device_id: String, label: String) -> Self {
        Self {
            device_id,
            label,
            data: VecDeque::new(),
            plot: AutoScalePlot::new(Default::default()),
            start_time: Instant::now(),
        }
    }

    /// Add a data point and update plot bounds
    fn add_data(&mut self, update: DataUpdate) {
        let time_offset = self.start_time.elapsed().as_secs_f64();
        let point = [time_offset, update.value];
        self.data.push_back(point);

        // Limit history
        if self.data.len() > MAX_PLOT_HISTORY {
            self.data.pop_front();
        }

        // Update plot bounds
        let points = self.data.iter().copied().collect::<Vec<_>>();
        self.plot.update_bounds(&points);
    }

    fn current_fps(&self) -> f64 {
        // Simple FPS estimate from data rate
        if self.data.len() < 2 {
            return 0.0;
        }

        let time_span = self.data.back().unwrap()[0] - self.data.front().unwrap()[0];
        if time_span > 0.0 {
            (self.data.len() - 1) as f64 / time_span
        } else {
            0.0
        }
    }
}

/// Live visualization panel
pub struct LiveVisualizationPanel {
    /// Camera states by device ID
    cameras: HashMap<String, CameraState>,
    /// Plot states by device ID
    plots: HashMap<String, PlotState>,
    /// Multi-detector grid for layout
    grid: MultiDetectorGrid,
    /// Frame update receiver
    frame_rx: Option<FrameUpdateReceiver>,
    /// Data update receiver
    data_rx: Option<DataUpdateReceiver>,
    /// Whether acquisition is active
    active: bool,
}

impl LiveVisualizationPanel {
    /// Create a new live visualization panel
    pub fn new() -> Self {
        Self {
            cameras: HashMap::new(),
            plots: HashMap::new(),
            grid: MultiDetectorGrid::new(),
            frame_rx: None,
            data_rx: None,
            active: false,
        }
    }

    /// Configure detectors to display
    ///
    /// # Arguments
    ///
    /// - `cameras`: List of (device_id, title) for camera panels
    /// - `plots`: List of (device_id, label, title) for plot panels
    pub fn configure_detectors(
        &mut self,
        cameras: Vec<(String, String)>,
        plots: Vec<(String, String, String)>,
    ) {
        // Clear existing state
        self.cameras.clear();
        self.plots.clear();
        self.grid.clear();

        // Add camera panels
        for (device_id, title) in cameras {
            self.cameras
                .insert(device_id.clone(), CameraState::new(device_id.clone()));
            self.grid.add_panel(DetectorPanel::camera(device_id, title));
        }

        // Add plot panels
        for (device_id, label, title) in plots {
            self.plots.insert(
                device_id.clone(),
                PlotState::new(device_id.clone(), label.clone()),
            );
            self.grid
                .add_panel(DetectorPanel::line_plot(device_id, label, title));
        }
    }

    /// Set the frame update receiver
    pub fn set_frame_receiver(&mut self, rx: FrameUpdateReceiver) {
        self.frame_rx = Some(rx);
    }

    /// Set the data update receiver
    pub fn set_data_receiver(&mut self, rx: DataUpdateReceiver) {
        self.data_rx = Some(rx);
    }

    /// Start acquisition
    pub fn start(&mut self) {
        self.active = true;
        // Reset start times for plots
        for plot in self.plots.values_mut() {
            plot.start_time = Instant::now();
            plot.data.clear();
        }
    }

    /// Stop acquisition
    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Poll for updates from async channels
    fn poll_updates(&mut self, ctx: &egui::Context) {
        // Drain frame updates
        if let Some(rx) = &self.frame_rx {
            while let Ok(frame) = rx.try_recv() {
                if let Some(camera) = self.cameras.get_mut(&frame.device_id) {
                    camera.update_frame(ctx, frame);
                }
            }
        }

        // Drain data updates
        if let Some(rx) = &self.data_rx {
            while let Ok(update) = rx.try_recv() {
                if let Some(plot) = self.plots.get_mut(&update.device_id) {
                    plot.add_data(update);
                }
            }
        }
    }

    /// Show the panel
    pub fn show(&mut self, ui: &mut egui::Ui) {
        // Poll for updates
        self.poll_updates(ui.ctx());

        // Request repaint if active
        if self.active {
            ui.ctx().request_repaint();
        }

        ui.vertical(|ui| {
            // Status bar
            self.render_status_bar(ui);

            ui.separator();

            // Detector grid
            self.render_grid(ui);
        });
    }

    /// Render status bar with LIVE/IDLE indicator and FPS
    fn render_status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Status indicator
            let status_text = if self.active { "● LIVE" } else { "○ IDLE" };
            let status_color = if self.active {
                egui::Color32::from_rgb(0, 255, 0)
            } else {
                egui::Color32::GRAY
            };
            ui.colored_label(status_color, status_text);

            ui.separator();

            // FPS summary
            let total_cameras = self.cameras.len();
            let total_plots = self.plots.len();
            let avg_camera_fps = if total_cameras > 0 {
                self.cameras.values().map(|c| c.current_fps()).sum::<f64>() / total_cameras as f64
            } else {
                0.0
            };
            let avg_plot_fps = if total_plots > 0 {
                self.plots.values().map(|p| p.current_fps()).sum::<f64>() / total_plots as f64
            } else {
                0.0
            };

            if total_cameras > 0 {
                ui.label(format!(
                    "Cameras: {} @ {:.1} FPS",
                    total_cameras, avg_camera_fps
                ));
            }
            if total_plots > 0 {
                ui.label(format!("Plots: {} @ {:.1} Hz", total_plots, avg_plot_fps));
            }
        });
    }

    /// Render detector grid with actual content
    fn render_grid(&mut self, ui: &mut egui::Ui) {
        if self.cameras.is_empty() && self.plots.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No detectors configured");
            });
            return;
        }

        // Get panel list for rendering
        let panels: Vec<DetectorPanel> = self.grid.panels().to_vec();

        if panels.is_empty() {
            return;
        }

        let (cols, rows) =
            crate::panels::multi_detector_grid::calculate_grid_dimensions(panels.len());

        // Render grid using StripBuilder
        use egui_extras::{Size, StripBuilder};

        StripBuilder::new(ui)
            .size(Size::remainder())
            .vertical(|mut strip| {
                for row_idx in 0..rows {
                    strip.strip(|builder| {
                        builder.size(Size::remainder()).horizontal(|mut strip| {
                            for col_idx in 0..cols {
                                strip.cell(|ui| {
                                    let panel_idx = row_idx * cols + col_idx;
                                    if panel_idx < panels.len() {
                                        self.render_panel_content(ui, &panels[panel_idx]);
                                    }
                                });
                            }
                        });
                    });
                }
            });
    }

    /// Render a single detector panel with actual content
    fn render_panel_content(&mut self, ui: &mut egui::Ui, panel: &DetectorPanel) {
        ui.group(|ui| {
            ui.set_min_size(ui.available_size());

            // Header
            ui.heading(&panel.title);

            // FPS indicator
            match &panel.detector_type {
                DetectorType::Camera { device_id } => {
                    if let Some(camera) = self.cameras.get(device_id) {
                        ui.label(format!("{:.1} FPS", camera.current_fps()));
                    }
                }
                DetectorType::LinePlot { device_id, .. } => {
                    if let Some(plot) = self.plots.get(device_id) {
                        ui.label(format!("{:.1} Hz", plot.current_fps()));
                    }
                }
            }

            ui.separator();

            // Content area
            match &panel.detector_type {
                DetectorType::Camera { device_id } => {
                    self.render_camera_panel(ui, device_id);
                }
                DetectorType::LinePlot { device_id, label } => {
                    self.render_plot_panel(ui, device_id, label);
                }
            }
        });
    }

    /// Render camera image display
    fn render_camera_panel(&self, ui: &mut egui::Ui, device_id: &str) {
        if let Some(camera) = self.cameras.get(device_id) {
            if let Some(texture) = &camera.texture {
                // Calculate fit size
                let available = ui.available_size();
                let aspect = camera.width as f32 / camera.height.max(1) as f32;
                let fit_size = if available.x / aspect <= available.y {
                    egui::vec2(available.x, available.x / aspect)
                } else {
                    egui::vec2(available.y * aspect, available.y)
                };

                ui.centered_and_justified(|ui| {
                    ui.image((texture.id(), fit_size));
                });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Waiting for frames...");
                });
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Camera not configured");
            });
        }
    }

    /// Render line plot display
    fn render_plot_panel(&mut self, ui: &mut egui::Ui, device_id: &str, label: &str) {
        if let Some(plot_state) = self.plots.get_mut(device_id) {
            if plot_state.data.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("Waiting for data...");
                });
                return;
            }

            // Render plot with controls
            let points: Vec<[f64; 2]> = plot_state.data.iter().copied().collect();
            let label_owned = label.to_string();
            plot_state
                .plot
                .show_with_controls(ui, format!("plot_{}", device_id), |plot_ui| {
                    let line = Line::new(label_owned.clone(), PlotPoints::from(points.clone()));
                    plot_ui.line(line);
                });
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Plot not configured");
            });
        }
    }
}

impl Default for LiveVisualizationPanel {
    fn default() -> Self {
        Self::new()
    }
}
