//! Image Viewer Panel - 2D camera frame visualization
//!
//! Displays live camera frames from FrameProducer devices with:
//! - Real-time frame streaming via gRPC
//! - Configurable colormaps (grayscale, viridis, etc.)
//! - Zoom/pan controls
//! - Frame metadata display (dimensions, FPS, frame count)
//!
//! ## Async Integration Pattern
//!
//! Uses message-passing for thread-safe async updates:
//! - Background task receives frames from gRPC stream
//! - Frames sent to panel via mpsc channel
//! - Panel drains channel each frame and updates texture

use eframe::egui;
use std::sync::mpsc;
use std::time::Instant;
use tokio::runtime::Runtime;

use crate::client::DaqClient;
use crate::widgets::{Histogram, HistogramPosition, RoiSelector};
use daq_proto::daq::FrameData;

/// Maximum frame queue depth (prevents memory buildup if GUI is slow)
const MAX_QUEUED_FRAMES: usize = 4;

/// Frame update message for async integration
#[derive(Debug)]
pub struct FrameUpdate {
    pub device_id: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub data: Vec<u8>,
    pub frame_number: u64,
    /// Timestamp in nanoseconds (for future frame timing analysis)
    #[allow(dead_code)]
    pub timestamp_ns: u64,
}

impl From<FrameData> for FrameUpdate {
    fn from(frame: FrameData) -> Self {
        Self {
            device_id: frame.device_id,
            width: frame.width,
            height: frame.height,
            bit_depth: frame.bit_depth,
            data: frame.data,
            frame_number: frame.frame_number,
            timestamp_ns: frame.timestamp_ns,
        }
    }
}

/// Sender for pushing frame updates from async tasks
pub type FrameUpdateSender = mpsc::SyncSender<FrameUpdate>;

/// Receiver for frame updates in the panel
pub type FrameUpdateReceiver = mpsc::Receiver<FrameUpdate>;

/// Create a new bounded channel pair for frame updates
/// Using a small buffer prevents memory growth when UI can't keep up
pub fn frame_channel() -> (FrameUpdateSender, FrameUpdateReceiver) {
    mpsc::sync_channel(MAX_QUEUED_FRAMES)
}

/// Colormap for image display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Grayscale,
    Viridis,
    Inferno,
    Plasma,
    Magma,
}

impl Colormap {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Grayscale => "Grayscale",
            Self::Viridis => "Viridis",
            Self::Inferno => "Inferno",
            Self::Plasma => "Plasma",
            Self::Magma => "Magma",
        }
    }

    /// Apply colormap to a normalized value (0.0-1.0) returning RGB
    pub fn apply(&self, value: f32) -> [u8; 3] {
        let v = value.clamp(0.0, 1.0);
        match self {
            Self::Grayscale => {
                let g = (v * 255.0) as u8;
                [g, g, g]
            }
            Self::Viridis => Self::viridis_lut(v),
            Self::Inferno => Self::inferno_lut(v),
            Self::Plasma => Self::plasma_lut(v),
            Self::Magma => Self::magma_lut(v),
        }
    }

    // Simplified colormap LUTs (approximations)
    fn viridis_lut(v: f32) -> [u8; 3] {
        // Viridis: purple -> blue -> green -> yellow
        let r = (0.267 + v * (0.993 - 0.267)) * 255.0;
        let g = v * 0.906 * 255.0;
        let b = (0.329 + v * (0.143_f32 - 0.329).abs()) * 255.0;
        [(r.clamp(0.0, 255.0)) as u8, (g.clamp(0.0, 255.0)) as u8, (b.clamp(0.0, 255.0)) as u8]
    }

    fn inferno_lut(v: f32) -> [u8; 3] {
        // Inferno: black -> purple -> red -> yellow
        let r = v.powf(0.5) * 255.0;
        let g = v.powf(1.5) * 200.0;
        let b = (1.0 - v) * v * 4.0 * 255.0;
        [(r.clamp(0.0, 255.0)) as u8, (g.clamp(0.0, 255.0)) as u8, (b.clamp(0.0, 255.0)) as u8]
    }

    fn plasma_lut(v: f32) -> [u8; 3] {
        // Plasma: blue -> purple -> orange -> yellow
        let r = (0.05 + v * 0.95) * 255.0;
        let g = v * v * 255.0;
        let b = (1.0 - v * 0.7) * 255.0;
        [(r.clamp(0.0, 255.0)) as u8, (g.clamp(0.0, 255.0)) as u8, (b.clamp(0.0, 255.0)) as u8]
    }

    fn magma_lut(v: f32) -> [u8; 3] {
        // Magma: black -> purple -> pink -> white
        let r = v.powf(0.7) * 255.0;
        let g = v * v * 200.0;
        let b = (0.3 + v * 0.7) * v * 255.0;
        [(r.clamp(0.0, 255.0)) as u8, (g.clamp(0.0, 255.0)) as u8, (b.clamp(0.0, 255.0)) as u8]
    }
}

/// Scale mode for pixel intensity mapping
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScaleMode {
    #[default]
    Linear,
    Log,
    Sqrt,
}

impl ScaleMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Log => "Log",
            Self::Sqrt => "Sqrt",
        }
    }

    /// Apply scaling to a normalized value (0.0-1.0)
    pub fn apply(&self, value: f32) -> f32 {
        match self {
            Self::Linear => value,
            Self::Log => (1.0 + value * 99.0).log10() / 2.0, // log10(1-100) -> 0-2 -> 0-1
            Self::Sqrt => value.sqrt(),
        }
    }
}

/// Stream subscription handle (for future external stream control)
#[allow(dead_code)]
pub struct FrameStreamSubscription {
    cancel_tx: tokio::sync::mpsc::Sender<()>,
    device_id: String,
}

#[allow(dead_code)]
impl FrameStreamSubscription {
    /// Cancel this subscription
    pub async fn cancel(self) {
        let _ = self.cancel_tx.send(()).await;
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }
}

/// FPS calculation state
struct FpsCounter {
    frame_times: std::collections::VecDeque<Instant>,
    max_samples: usize,
}

impl FpsCounter {
    fn new(max_samples: usize) -> Self {
        Self {
            frame_times: std::collections::VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        self.frame_times.push_back(now);
        while self.frame_times.len() > self.max_samples {
            self.frame_times.pop_front();
        }
    }

    fn fps(&self) -> f32 {
        if self.frame_times.len() < 2 {
            return 0.0;
        }
        let first = self.frame_times.front().unwrap();
        let last = self.frame_times.back().unwrap();
        let duration = last.duration_since(*first).as_secs_f32();
        if duration > 0.0 {
            (self.frame_times.len() - 1) as f32 / duration
        } else {
            0.0
        }
    }
}

/// Image Viewer Panel state
pub struct ImageViewerPanel {
    /// Currently selected device ID
    device_id: Option<String>,
    /// Current frame dimensions
    width: u32,
    height: u32,
    /// Current frame bit depth
    bit_depth: u32,
    /// Frame counter
    frame_count: u64,
    /// Cached texture handle
    texture: Option<egui::TextureHandle>,
    /// Current colormap
    colormap: Colormap,
    /// Current scale mode
    scale_mode: ScaleMode,
    /// Zoom level (1.0 = fit to window)
    zoom: f32,
    /// Pan offset
    pan: egui::Vec2,
    /// Frame update receiver
    frame_rx: Option<FrameUpdateReceiver>,
    /// Frame update sender (for cloning to async tasks)
    #[allow(dead_code)] // Used in start_stream (future feature)
    frame_tx: Option<FrameUpdateSender>,
    /// Active stream subscription
    subscription: Option<FrameStreamSubscription>,
    /// FPS counter
    fps_counter: FpsCounter,
    /// Auto-fit zoom on next frame
    auto_fit: bool,
    /// Error message
    error: Option<String>,
    /// Status message
    status: Option<String>,
    /// Max FPS for streaming (rate limit)
    #[allow(dead_code)] // Used in start_stream (future feature)
    max_fps: u32,
    /// ROI selector state
    roi_selector: RoiSelector,
    /// Last frame raw data (for ROI statistics computation)
    last_frame_data: Option<Vec<u8>>,
    /// Show ROI statistics panel
    show_roi_panel: bool,
    /// Histogram for intensity distribution
    histogram: Histogram,
    /// Histogram display position
    histogram_position: HistogramPosition,
}

impl Default for ImageViewerPanel {
    fn default() -> Self {
        let (tx, rx) = frame_channel();
        Self {
            device_id: None,
            width: 0,
            height: 0,
            bit_depth: 0,
            frame_count: 0,
            texture: None,
            colormap: Colormap::default(),
            scale_mode: ScaleMode::default(),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            frame_rx: Some(rx),
            frame_tx: Some(tx),
            subscription: None,
            fps_counter: FpsCounter::new(30),
            auto_fit: true,
            error: None,
            status: None,
            max_fps: 30,
            roi_selector: RoiSelector::new(),
            last_frame_data: None,
            show_roi_panel: true,
            histogram: Histogram::new(),
            histogram_position: HistogramPosition::BottomRight,
        }
    }
}

impl ImageViewerPanel {
    /// Create a new image viewer panel
    pub fn new() -> Self {
        Self::default()
    }

    /// Get sender for async frame updates (public API for external frame producers)
    #[allow(dead_code)]
    pub fn get_sender(&self) -> Option<FrameUpdateSender> {
        self.frame_tx.clone()
    }

    /// Start streaming frames from a device (public API for external control)
    #[allow(dead_code)]
    pub fn start_stream(
        &mut self,
        device_id: &str,
        client: &mut DaqClient,
        runtime: &Runtime,
    ) {
        // Cancel existing subscription
        if let Some(sub) = self.subscription.take() {
            let cancel_tx = sub.cancel_tx.clone();
            runtime.spawn(async move {
                let _ = cancel_tx.send(()).await;
            });
        }

        self.device_id = Some(device_id.to_string());
        self.error = None;
        self.status = Some(format!("Connecting to {}...", device_id));

        let Some(frame_tx) = self.frame_tx.clone() else {
            self.error = Some("Internal error: no frame channel".to_string());
            return;
        };

        let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut client = client.clone();
        let device_id_clone = device_id.to_string();
        let max_fps = self.max_fps;

        runtime.spawn(async move {
            use futures::StreamExt;

            // Start the frame stream
            let stream = match client.stream_frames(&device_id_clone, max_fps).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(device_id = %device_id_clone, error = %e, "Failed to start frame stream");
                    return;
                }
            };

            tokio::pin!(stream);

            loop {
                tokio::select! {
                    _ = cancel_rx.recv() => {
                        tracing::info!(device_id = %device_id_clone, "Frame stream cancelled");
                        break;
                    }
                    frame_result = stream.next() => {
                        match frame_result {
                            Some(Ok(frame_data)) => {
                                let update = FrameUpdate::from(frame_data);
                                // Use try_send to avoid blocking when queue is full
                                // Dropping frames is preferred over blocking the stream
                                match frame_tx.try_send(update) {
                                    Ok(()) => {}
                                    Err(mpsc::TrySendError::Full(_)) => {
                                        // Queue full - frame dropped, UI will catch up
                                        tracing::trace!(device_id = %device_id_clone, "Frame dropped - UI queue full");
                                    }
                                    Err(mpsc::TrySendError::Disconnected(_)) => {
                                        // Receiver dropped
                                        break;
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                tracing::warn!(device_id = %device_id_clone, error = %e, "Frame stream error");
                                // Continue on transient errors
                            }
                            None => {
                                // Stream ended
                                tracing::info!(device_id = %device_id_clone, "Frame stream ended");
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.subscription = Some(FrameStreamSubscription {
            cancel_tx,
            device_id: device_id.to_string(),
        });
    }

    /// Stop streaming
    pub fn stop_stream(&mut self, runtime: &Runtime) {
        if let Some(sub) = self.subscription.take() {
            let cancel_tx = sub.cancel_tx.clone();
            runtime.spawn(async move {
                let _ = cancel_tx.send(()).await;
            });
        }
        self.status = Some("Stream stopped".to_string());
    }

    /// Drain pending frame updates, keeping only the most recent
    ///
    /// Fully drains the channel to prevent latency buildup.
    /// With bounded channel, producer blocks when queue is full.
    fn drain_updates(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.frame_rx else { return };

        // Drain ALL pending frames, keeping only the last one
        // This ensures we always display the most recent frame
        let mut latest_frame: Option<FrameUpdate> = None;

        while let Ok(frame) = rx.try_recv() {
            latest_frame = Some(frame);
        }

        // Process only the latest frame
        if let Some(frame) = latest_frame {
            self.process_frame(ctx, frame);
        }
    }

    /// Process a single frame update
    fn process_frame(&mut self, ctx: &egui::Context, frame: FrameUpdate) {
        // Validate frame belongs to currently selected device (bd-tjwm.3)
        if let Some(expected_device) = &self.device_id {
            if &frame.device_id != expected_device {
                tracing::trace!(
                    expected = %expected_device,
                    received = %frame.device_id,
                    "Dropping frame from unexpected device"
                );
                return;
            }
        }

        self.fps_counter.tick();
        self.width = frame.width;
        self.height = frame.height;
        self.bit_depth = frame.bit_depth;
        self.frame_count = frame.frame_number;
        self.error = None;
        self.status = None;

        // Store frame data for ROI statistics
        self.last_frame_data = Some(frame.data.clone());

        // Update ROI statistics if we have an active ROI
        self.roi_selector.update_statistics(&frame.data, frame.width, frame.height, frame.bit_depth);

        // Update histogram
        self.histogram.from_frame_data(&frame.data, frame.width, frame.height, frame.bit_depth);

        // Convert frame data to RGBA based on bit depth
        let rgba = self.convert_to_rgba(&frame);

        // Create or update texture
        let size = [frame.width as usize, frame.height as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);

        if let Some(texture) = &mut self.texture {
            texture.set(image, egui::TextureOptions::NEAREST);
        } else {
            self.texture = Some(ctx.load_texture(
                "camera_frame",
                image,
                egui::TextureOptions::NEAREST,
            ));
        }
    }

    /// Convert raw frame data to RGBA based on bit depth and colormap
    fn convert_to_rgba(&self, frame: &FrameUpdate) -> Vec<u8> {
        // Guard against zero or invalid dimensions
        if frame.width == 0 || frame.height == 0 {
            return Vec::new();
        }

        // Use checked arithmetic to prevent overflow on large dimensions
        let Some(pixel_count) = (frame.width as u64).checked_mul(frame.height as u64) else {
            return Vec::new();
        };

        // Cap allocation to reasonable size (256 MB max for RGBA)
        const MAX_PIXELS: u64 = 64 * 1024 * 1024; // 64M pixels = 256MB RGBA
        if pixel_count > MAX_PIXELS {
            tracing::warn!(
                width = frame.width,
                height = frame.height,
                "Frame too large, capping allocation"
            );
            return Vec::new();
        }

        let pixel_count = pixel_count as usize;
        let mut rgba = vec![255u8; pixel_count * 4]; // Pre-fill alpha

        match frame.bit_depth {
            8 => {
                // 8-bit grayscale - validate data length (bd-tjwm.7)
                if frame.data.len() < pixel_count {
                    tracing::warn!(
                        expected = pixel_count,
                        actual = frame.data.len(),
                        "8-bit frame data truncated"
                    );
                }
                for (i, &pixel) in frame.data.iter().take(pixel_count).enumerate() {
                    let normalized = pixel as f32 / 255.0;
                    let scaled = self.scale_mode.apply(normalized);
                    let [r, g, b] = self.colormap.apply(scaled);
                    rgba[i * 4] = r;
                    rgba[i * 4 + 1] = g;
                    rgba[i * 4 + 2] = b;
                    // Alpha already set to 255
                }
            }
            12 | 16 => {
                // 16-bit (or 12-bit stored as 16-bit) little-endian
                let max_val = if frame.bit_depth == 12 { 4095.0 } else { 65535.0 };
                for i in 0..pixel_count {
                    let byte_idx = i * 2;
                    if byte_idx + 1 >= frame.data.len() {
                        break;
                    }
                    let pixel = u16::from_le_bytes([frame.data[byte_idx], frame.data[byte_idx + 1]]);
                    let normalized = pixel as f32 / max_val;
                    let scaled = self.scale_mode.apply(normalized);
                    let [r, g, b] = self.colormap.apply(scaled);
                    rgba[i * 4] = r;
                    rgba[i * 4 + 1] = g;
                    rgba[i * 4 + 2] = b;
                }
            }
            _ => {
                // Unknown bit depth - show error pattern (checkerboard)
                // Safe: width already validated as non-zero above
                let width = frame.width as usize;
                for i in 0..pixel_count {
                    let checkerboard = ((i % width) / 16 + (i / width) / 16) % 2;
                    let color = if checkerboard == 0 { 255u8 } else { 128u8 };
                    rgba[i * 4] = color;
                    rgba[i * 4 + 1] = 0;
                    rgba[i * 4 + 2] = color;
                }
            }
        }

        rgba
    }

    /// Render the image viewer panel
    pub fn ui(&mut self, ui: &mut egui::Ui, _client: Option<&mut DaqClient>, runtime: &Runtime) {
        // Drain async updates
        self.drain_updates(ui.ctx());

        // Request continuous repaint while streaming
        if self.subscription.is_some() {
            ui.ctx().request_repaint();
        }

        // Toolbar
        ui.horizontal(|ui| {
            ui.heading("Image Viewer");
            ui.separator();

            // Device selector (simple text for now)
            if let Some(device_id) = &self.device_id {
                ui.label(format!("Device: {}", device_id));
            } else {
                ui.label("No device selected");
            }

            ui.separator();

            // Stream controls
            let is_streaming = self.subscription.is_some();
            if is_streaming {
                if ui.button("Stop").clicked() {
                    self.stop_stream(runtime);
                }
            }

            // Colormap selector
            ui.separator();
            egui::ComboBox::from_label("")
                .selected_text(self.colormap.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.colormap, Colormap::Grayscale, "Grayscale");
                    ui.selectable_value(&mut self.colormap, Colormap::Viridis, "Viridis");
                    ui.selectable_value(&mut self.colormap, Colormap::Inferno, "Inferno");
                    ui.selectable_value(&mut self.colormap, Colormap::Plasma, "Plasma");
                    ui.selectable_value(&mut self.colormap, Colormap::Magma, "Magma");
                });

            // Scale mode selector
            egui::ComboBox::from_id_salt("scale_mode")
                .selected_text(self.scale_mode.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.scale_mode, ScaleMode::Linear, "Linear");
                    ui.selectable_value(&mut self.scale_mode, ScaleMode::Log, "Log");
                    ui.selectable_value(&mut self.scale_mode, ScaleMode::Sqrt, "Sqrt");
                });

            // Zoom controls
            ui.separator();
            if ui.button("Fit").clicked() {
                self.auto_fit = true;
            }
            if ui.button("1:1").clicked() {
                self.zoom = 1.0;
                self.pan = egui::Vec2::ZERO;
                self.auto_fit = false;
            }
            ui.label(format!("{:.0}%", self.zoom * 100.0));

            // ROI controls
            ui.separator();
            let roi_label = if self.roi_selector.selection_mode { "ROI [ON]" } else { "ROI" };
            if ui.selectable_label(self.roi_selector.selection_mode, roi_label).clicked() {
                self.roi_selector.selection_mode = !self.roi_selector.selection_mode;
            }
            if self.roi_selector.roi().is_some() {
                if ui.button("Clear ROI").clicked() {
                    self.roi_selector.clear();
                }
            }
            ui.checkbox(&mut self.show_roi_panel, "Stats");

            // Histogram controls
            ui.separator();
            egui::ComboBox::from_id_salt("histogram_pos")
                .selected_text(format!("Hist: {}", self.histogram_position.label()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.histogram_position, HistogramPosition::Hidden, "Hidden");
                    ui.selectable_value(&mut self.histogram_position, HistogramPosition::BottomRight, "Bottom Right");
                    ui.selectable_value(&mut self.histogram_position, HistogramPosition::BottomLeft, "Bottom Left");
                    ui.selectable_value(&mut self.histogram_position, HistogramPosition::TopRight, "Top Right");
                    ui.selectable_value(&mut self.histogram_position, HistogramPosition::TopLeft, "Top Left");
                    ui.selectable_value(&mut self.histogram_position, HistogramPosition::SidePanel, "Side Panel");
                });
            if self.histogram_position.is_visible() {
                ui.checkbox(&mut self.histogram.log_scale, "Log");
            }
        });

        ui.separator();

        // Status bar
        ui.horizontal(|ui| {
            if self.width > 0 {
                ui.label(format!("{}x{} @ {}bit", self.width, self.height, self.bit_depth));
                ui.separator();
                ui.label(format!("Frame: {}", self.frame_count));
                ui.separator();
                ui.label(format!("{:.1} FPS", self.fps_counter.fps()));
            }

            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::RED, err);
            }
            if let Some(status) = &self.status {
                ui.colored_label(egui::Color32::YELLOW, status);
            }
        });

        ui.separator();

        // Image display area with optional statistics panel
        ui.horizontal(|ui| {
            // Calculate side panel width based on what's visible
            let has_roi_panel = self.show_roi_panel && self.roi_selector.roi().is_some();
            let has_histogram_panel = matches!(self.histogram_position, HistogramPosition::SidePanel);
            let stats_panel_width = if has_roi_panel || has_histogram_panel {
                180.0 // Slightly wider to accommodate histogram
            } else {
                0.0
            };
            let available_size = ui.available_size() - egui::vec2(stats_panel_width + 8.0, 0.0);

            if let Some(texture) = &self.texture {
                // Calculate fit zoom if needed
                if self.auto_fit && self.width > 0 && self.height > 0 {
                    let scale_x = available_size.x / self.width as f32;
                    let scale_y = available_size.y / self.height as f32;
                    self.zoom = scale_x.min(scale_y).min(1.0); // Don't upscale beyond 1:1
                    self.pan = egui::Vec2::ZERO;
                    self.auto_fit = false;
                }

                let image_size = egui::vec2(self.width as f32 * self.zoom, self.height as f32 * self.zoom);

                // Scrollable/pannable area
                egui::ScrollArea::both()
                    .id_salt("image_scroll")
                    .show(ui, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            available_size.max(image_size),
                            egui::Sense::click_and_drag(),
                        );

                        // Calculate image offset (centered)
                        let offset = (available_size - image_size) / 2.0 + self.pan;
                        let image_rect = egui::Rect::from_min_size(
                            rect.min + offset,
                            image_size,
                        );

                        // Handle ROI selection or pan depending on mode
                        if self.roi_selector.selection_mode {
                            // ROI selection mode
                            let roi_finalized = self.roi_selector.handle_input(
                                &response,
                                rect,
                                (self.width, self.height),
                                self.zoom,
                                self.pan,
                            );

                            // If ROI was finalized and we have frame data, compute statistics
                            if roi_finalized {
                                if let (Some(roi), Some(frame_data)) = (self.roi_selector.roi(), &self.last_frame_data) {
                                    self.roi_selector.set_roi_from_frame(
                                        *roi,
                                        frame_data,
                                        self.width,
                                        self.height,
                                        self.bit_depth,
                                    );
                                }
                            }
                        } else {
                            // Pan mode
                            if response.dragged() {
                                self.pan += response.drag_delta();
                            }
                        }

                        // Handle zoom with scroll wheel (always active)
                        if response.hovered() {
                            let scroll_delta = ui.input(|i| i.raw_scroll_delta.y);
                            if scroll_delta != 0.0 {
                                let zoom_factor = 1.0 + scroll_delta * 0.001;
                                self.zoom = (self.zoom * zoom_factor).clamp(0.1, 10.0);
                            }
                        }

                        // Draw the image
                        ui.painter().image(
                            texture.id(),
                            image_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );

                        // Draw ROI overlay
                        self.roi_selector.draw_overlay(
                            ui.painter(),
                            rect,
                            (self.width, self.height),
                            self.zoom,
                            self.pan,
                        );

                        // Draw histogram overlay if positioned on image
                        if self.histogram_position.is_overlay() {
                            let hist_size = egui::vec2(180.0, 80.0);
                            let hist_rect = self.histogram_position.overlay_rect(image_rect, hist_size);

                            // Create a child UI at the overlay position
                            let mut hist_ui = ui.new_child(egui::UiBuilder::new()
                                .max_rect(hist_rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Min)));
                            self.histogram.show_overlay(&mut hist_ui, hist_size);
                        }

                        // Show pixel coordinates on hover
                        if let Some(pos) = response.hover_pos() {
                            let image_pos = pos - rect.min - offset;
                            let pixel_x = (image_pos.x / self.zoom) as i32;
                            let pixel_y = (image_pos.y / self.zoom) as i32;
                            if pixel_x >= 0 && pixel_x < self.width as i32
                                && pixel_y >= 0 && pixel_y < self.height as i32
                            {
                                response.on_hover_text(format!("({}, {})", pixel_x, pixel_y));
                            }
                        }
                    });
            } else {
                // No image - show placeholder
                ui.centered_and_justified(|ui| {
                    ui.label("No image. Select a camera device and start streaming.");
                });
            }

            // Side panels (ROI stats and/or histogram)
            let show_side_panel = self.show_roi_panel || matches!(self.histogram_position, HistogramPosition::SidePanel);
            if show_side_panel {
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_width(stats_panel_width);

                    // ROI statistics
                    if self.show_roi_panel {
                        self.roi_selector.show_statistics_panel(ui);
                    }

                    // Histogram in side panel
                    if matches!(self.histogram_position, HistogramPosition::SidePanel) {
                        if self.show_roi_panel && self.roi_selector.roi().is_some() {
                            ui.add_space(8.0);
                        }
                        self.histogram.show_panel(ui);
                    }
                });
            }
        });
    }

    /// Set the device to stream from (for external control)
    #[allow(dead_code)]
    pub fn set_device(&mut self, device_id: &str, client: &mut DaqClient, runtime: &Runtime) {
        self.start_stream(device_id, client, runtime);
    }

    /// Check if currently streaming
    #[allow(dead_code)]
    pub fn is_streaming(&self) -> bool {
        self.subscription.is_some()
    }

    /// Get current device ID
    #[allow(dead_code)]
    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }
}
