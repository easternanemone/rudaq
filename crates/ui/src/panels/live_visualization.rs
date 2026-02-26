//! Live Visualization Panel - Unified detector display during acquisition
//!
//! Displays camera frames, scalar line plots, and spectrum plots in a grid layout
//! with FPS/rate status.
//! Integrates:
//! - Camera frame streaming (via FrameUpdate messages)
//! - Line plot updates (via DataUpdate messages)
//! - MultiDetectorGrid for responsive layout
//! - AutoScalePlot for grow-to-fit plots
//!
//! ## Async Integration Pattern
//!
//! Uses message-passing for thread-safe async updates:
//! - Background tasks send FrameUpdate/DataUpdate/SpectrumUpdate via mpsc channels
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

/// Maximum queued spectrum updates per plot
const MAX_QUEUED_SPECTRA: usize = 16;

/// Maximum plot history (data points)
const MAX_PLOT_HISTORY: usize = 500;

/// Maximum retained spectrum snapshots per plot (ring buffer)
const MAX_SPECTRUM_HISTORY: usize = 32;

/// Maximum rendered points per spectrum line (UI decimation)
const MAX_SPECTRUM_POINTS_DISPLAY: usize = 2048;

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

/// Spectrum update message for vector plots.
#[derive(Debug, Clone)]
pub struct SpectrumUpdate {
    /// Plot key (often device ID, but may be a logical stream identifier).
    pub device_id: String,
    /// Human-readable series label (e.g. "merged", "order_52").
    pub label: String,
    /// X-axis values (wavelength or pixel axis).
    pub x_values: Vec<f64>,
    /// Y-axis values (flux/intensity).
    pub y_values: Vec<f64>,
    pub x_unit: Option<String>,
    pub y_unit: Option<String>,
    #[allow(dead_code)]
    pub timestamp_ns: u64,
}

/// Sender handles for pushing updates from async tasks
pub type FrameUpdateSender = mpsc::SyncSender<FrameUpdate>;
pub type DataUpdateSender = mpsc::SyncSender<DataUpdate>;
pub type SpectrumUpdateSender = mpsc::SyncSender<SpectrumUpdate>;

/// Receiver handles stored in panel
type FrameUpdateReceiver = mpsc::Receiver<FrameUpdate>;
type DataUpdateReceiver = mpsc::Receiver<DataUpdate>;
type SpectrumUpdateReceiver = mpsc::Receiver<SpectrumUpdate>;

/// Create a new bounded channel pair for frame updates
pub fn frame_channel() -> (FrameUpdateSender, FrameUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_FRAMES)
}

/// Create a new bounded channel pair for data updates
pub fn data_channel() -> (DataUpdateSender, DataUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_DATA)
}

/// Create a new bounded channel pair for spectrum updates
pub fn spectrum_channel() -> (SpectrumUpdateSender, SpectrumUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_SPECTRA)
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

#[derive(Debug, Clone)]
struct SpectrumSnapshotMeta {
    timestamp_ns: u64,
    raw_points: usize,
    displayed_points: usize,
}

/// Spectrum plot state for live visualization of streamed vector payloads.
struct SpectrumPlotState {
    #[allow(dead_code)]
    device_id: String,
    label: String,
    x_unit: Option<String>,
    y_unit: Option<String>,
    latest_points: Vec<[f64; 2]>,
    latest_raw_point_count: usize,
    history: VecDeque<SpectrumSnapshotMeta>,
    update_times: VecDeque<Instant>,
    plot: AutoScalePlot,
    max_display_points: usize,
}

impl SpectrumPlotState {
    fn new(device_id: String, label: String) -> Self {
        Self {
            device_id,
            label,
            x_unit: None,
            y_unit: None,
            latest_points: Vec::new(),
            latest_raw_point_count: 0,
            history: VecDeque::new(),
            update_times: VecDeque::new(),
            plot: AutoScalePlot::new(Default::default()),
            max_display_points: MAX_SPECTRUM_POINTS_DISPLAY,
        }
    }

    fn add_spectrum(&mut self, update: SpectrumUpdate) {
        self.label = update.label;
        self.x_unit = update.x_unit;
        self.y_unit = update.y_unit;
        self.latest_raw_point_count = update.x_values.len().min(update.y_values.len());

        self.latest_points =
            decimate_spectrum_points(&update.x_values, &update.y_values, self.max_display_points);

        self.plot.update_bounds(&self.latest_points);

        self.history.push_back(SpectrumSnapshotMeta {
            timestamp_ns: update.timestamp_ns,
            raw_points: self.latest_raw_point_count,
            displayed_points: self.latest_points.len(),
        });
        if self.history.len() > MAX_SPECTRUM_HISTORY {
            self.history.pop_front();
        }

        self.update_times.push_back(Instant::now());
        trim_recent_times(&mut self.update_times);
    }

    fn current_hz(&self) -> f64 {
        rate_from_instants(&self.update_times)
    }

    fn latest_display_points(&self) -> usize {
        self.latest_points.len()
    }

    fn history_len(&self) -> usize {
        self.history.len()
    }

    fn latest_timestamp_ns(&self) -> Option<u64> {
        self.history.back().map(|h| h.timestamp_ns)
    }

    fn last_snapshot_displayed_points(&self) -> Option<usize> {
        self.history.back().map(|h| h.displayed_points)
    }

    fn last_snapshot_raw_points(&self) -> Option<usize> {
        self.history.back().map(|h| h.raw_points)
    }
}

fn trim_recent_times(times: &mut VecDeque<Instant>) {
    let now = Instant::now();
    let fps_window = std::time::Duration::from_secs_f64(FPS_WINDOW);
    let cutoff = now.checked_sub(fps_window).unwrap_or(now);
    while let Some(&time) = times.front() {
        if time < cutoff {
            times.pop_front();
        } else {
            break;
        }
    }
}

fn rate_from_instants(times: &VecDeque<Instant>) -> f64 {
    if times.len() < 2 {
        return 0.0;
    }
    let now = Instant::now();
    let fps_window = std::time::Duration::from_secs_f64(FPS_WINDOW);
    let cutoff = now.checked_sub(fps_window).unwrap_or(now);
    let recent: Vec<_> = times.iter().filter(|&&t| t >= cutoff).collect();
    if recent.len() < 2 {
        return 0.0;
    }
    let elapsed = (now - *recent[0]).as_secs_f64();
    if elapsed > 0.0 {
        (recent.len() - 1) as f64 / elapsed
    } else {
        0.0
    }
}

fn decimate_spectrum_points(
    x_values: &[f64],
    y_values: &[f64],
    max_points: usize,
) -> Vec<[f64; 2]> {
    let n = x_values.len().min(y_values.len());
    if n == 0 {
        return Vec::new();
    }
    let max_points = max_points.max(2);
    if n <= max_points {
        return x_values
            .iter()
            .zip(y_values.iter())
            .take(n)
            .map(|(&x, &y)| [x, y])
            .collect();
    }

    let last_index = n - 1;
    let interior_target = max_points.saturating_sub(2);
    let step = ((n - 2) as f64 / interior_target.max(1) as f64).ceil() as usize;

    let mut out = Vec::with_capacity(max_points);
    out.push([x_values[0], y_values[0]]);
    let mut idx = 1usize;
    while idx < last_index && out.len() < max_points - 1 {
        out.push([x_values[idx], y_values[idx]]);
        idx = idx.saturating_add(step.max(1));
    }
    out.push([x_values[last_index], y_values[last_index]]);
    out
}

/// Live visualization panel
pub struct LiveVisualizationPanel {
    /// Camera states by device ID
    cameras: HashMap<String, CameraState>,
    /// Plot states by device ID
    plots: HashMap<String, PlotState>,
    /// Spectrum plot states by plot key (often device ID or logical stream name)
    spectrum_plots: HashMap<String, SpectrumPlotState>,
    /// Multi-detector grid for layout
    grid: MultiDetectorGrid,
    /// Frame update receiver
    frame_rx: Option<FrameUpdateReceiver>,
    /// Data update receiver
    data_rx: Option<DataUpdateReceiver>,
    /// Spectrum update receiver
    spectrum_rx: Option<SpectrumUpdateReceiver>,
    /// Whether acquisition is active
    active: bool,
}

impl LiveVisualizationPanel {
    /// Create a new live visualization panel
    pub fn new() -> Self {
        Self {
            cameras: HashMap::new(),
            plots: HashMap::new(),
            spectrum_plots: HashMap::new(),
            grid: MultiDetectorGrid::new(),
            frame_rx: None,
            data_rx: None,
            spectrum_rx: None,
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
        self.configure_detectors_with_spectra(cameras, plots, Vec::new());
    }

    /// Configure detectors including streamed spectrum plot panels.
    pub fn configure_detectors_with_spectra(
        &mut self,
        cameras: Vec<(String, String)>,
        plots: Vec<(String, String, String)>,
        spectrum_plots: Vec<(String, String, String)>,
    ) {
        // Clear existing state
        self.cameras.clear();
        self.plots.clear();
        self.spectrum_plots.clear();
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

        // Add spectrum panels
        for (device_id, label, title) in spectrum_plots {
            self.spectrum_plots.insert(
                device_id.clone(),
                SpectrumPlotState::new(device_id.clone(), label.clone()),
            );
            self.grid
                .add_panel(DetectorPanel::spectrum_plot(device_id, label, title));
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

    /// Set the spectrum update receiver
    pub fn set_spectrum_receiver(&mut self, rx: SpectrumUpdateReceiver) {
        self.spectrum_rx = Some(rx);
    }

    /// Start acquisition
    pub fn start(&mut self) {
        self.active = true;
        // Reset start times for plots
        for plot in self.plots.values_mut() {
            plot.start_time = Instant::now();
            plot.data.clear();
        }
        for spectrum in self.spectrum_plots.values_mut() {
            spectrum.latest_points.clear();
            spectrum.latest_raw_point_count = 0;
            spectrum.history.clear();
            spectrum.update_times.clear();
            spectrum.plot.reset_bounds();
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

        // Drain spectrum updates
        if let Some(rx) = &self.spectrum_rx {
            while let Ok(update) = rx.try_recv() {
                if let Some(plot) = self.spectrum_plots.get_mut(&update.device_id) {
                    plot.add_spectrum(update);
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
            let total_spectra = self.spectrum_plots.len();
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
            let avg_spectrum_hz = if total_spectra > 0 {
                self.spectrum_plots
                    .values()
                    .map(SpectrumPlotState::current_hz)
                    .sum::<f64>()
                    / total_spectra as f64
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
            if total_spectra > 0 {
                ui.label(format!(
                    "Spectra: {} @ {:.1} Hz",
                    total_spectra, avg_spectrum_hz
                ));
            }
        });
    }

    /// Render detector grid with actual content
    fn render_grid(&mut self, ui: &mut egui::Ui) {
        if self.cameras.is_empty() && self.plots.is_empty() && self.spectrum_plots.is_empty() {
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
                DetectorType::SpectrumPlot { device_id, .. } => {
                    if let Some(plot) = self.spectrum_plots.get(device_id) {
                        ui.label(format!("{:.1} Hz", plot.current_hz()));
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
                DetectorType::SpectrumPlot { device_id, label } => {
                    self.render_spectrum_panel(ui, device_id, label);
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

    /// Render streamed spectrum plot display
    fn render_spectrum_panel(&mut self, ui: &mut egui::Ui, device_id: &str, label: &str) {
        if let Some(plot_state) = self.spectrum_plots.get_mut(device_id) {
            if plot_state.latest_points.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("Waiting for spectrum data...");
                });
                return;
            }

            let points = plot_state.latest_points.clone();
            let label_owned = if label.is_empty() {
                plot_state.label.clone()
            } else {
                label.to_string()
            };

            ui.horizontal_wrapped(|ui| {
                let raw = plot_state
                    .last_snapshot_raw_points()
                    .unwrap_or(plot_state.latest_raw_point_count);
                let shown = plot_state
                    .last_snapshot_displayed_points()
                    .unwrap_or(plot_state.latest_display_points());
                ui.label(format!("pts: {} -> {}", raw, shown));
                ui.label(format!("buf: {}", plot_state.history_len()));
                if let Some(xu) = &plot_state.x_unit {
                    if !xu.is_empty() {
                        ui.label(format!("x: {}", xu));
                    }
                }
                if let Some(yu) = &plot_state.y_unit {
                    if !yu.is_empty() {
                        ui.label(format!("y: {}", yu));
                    }
                }
                if let Some(ts) = plot_state.latest_timestamp_ns() {
                    ui.label(format!("ts: {}", ts));
                }
            });

            plot_state.plot.show_with_controls(
                ui,
                format!("spectrum_plot_{}", device_id),
                |plot_ui| {
                    let line = Line::new(label_owned, PlotPoints::from(points));
                    plot_ui.line(line);
                },
            );
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Spectrum plot not configured");
            });
        }
    }
}

impl Default for LiveVisualizationPanel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimate_spectrum_points_preserves_endpoints_and_caps_length() {
        let x = (0..5000).map(|i| i as f64).collect::<Vec<_>>();
        let y = (0..5000).map(|i| (i as f64) * 2.0).collect::<Vec<_>>();

        let out = decimate_spectrum_points(&x, &y, 256);
        assert!(out.len() <= 256);
        assert_eq!(out.first().copied(), Some([0.0, 0.0]));
        assert_eq!(out.last().copied(), Some([4999.0, 9998.0]));
    }

    #[test]
    fn spectrum_plot_state_uses_ring_buffer_and_decimation() {
        let mut state = SpectrumPlotState::new("spec".to_string(), "merged".to_string());
        state.max_display_points = 64;

        for i in 0..(MAX_SPECTRUM_HISTORY + 5) {
            let n = 512usize;
            let x = (0..n).map(|j| j as f64).collect::<Vec<_>>();
            let y = (0..n).map(|j| j as f64 + i as f64).collect::<Vec<_>>();
            state.add_spectrum(SpectrumUpdate {
                device_id: "spec".to_string(),
                label: "merged".to_string(),
                x_values: x,
                y_values: y,
                x_unit: Some("nm".to_string()),
                y_unit: Some("counts".to_string()),
                timestamp_ns: i as u64,
            });
        }

        assert_eq!(state.history_len(), MAX_SPECTRUM_HISTORY);
        assert_eq!(state.latest_raw_point_count, 512);
        assert!(state.latest_display_points() <= 64);
        assert_eq!(
            state.latest_timestamp_ns(),
            Some((MAX_SPECTRUM_HISTORY + 4) as u64)
        );
        assert_eq!(state.last_snapshot_raw_points(), Some(512));
        assert_eq!(state.x_unit.as_deref(), Some("nm"));
        assert_eq!(state.y_unit.as_deref(), Some("counts"));
    }
}
