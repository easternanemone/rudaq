//! Mock Hardware Implementations
//!
//! Provides simulated hardware devices for testing without physical hardware.
//! All mock devices use async-safe operations (tokio::time::sleep, not std::thread::sleep).
//!
//! # Available Mocks
//!
//! - `MockStage` - Simulated motion stage with realistic timing
//! - `MockCamera` - Simulated camera with trigger and streaming support
//!
//! # Performance Characteristics
//!
//! - MockStage: 10mm/sec motion speed, 50ms settling time
//! - MockCamera: 33ms frame readout (30fps simulation)

use crate::capabilities::{
    ExposureControl, FrameProducer, Movable, Parameterized, Readable, Stageable, Triggerable,
};
use crate::Frame;
use anyhow::anyhow;
use anyhow::Result;
use async_trait::async_trait;
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, Duration};

// =============================================================================
// Test Pattern Generator
// =============================================================================

/// Simple pseudo-random number generator (LCG) for reproducible noise.
/// Uses the same algorithm as glibc for predictable cross-platform behavior.
#[inline]
fn prng(seed: u64) -> u64 {
    seed.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fffffff
}

/// Generates a diagnostic test pattern for camera validation.
///
/// The pattern includes:
/// - Checkerboard background for pixel alignment verification
/// - Corner markers (different shapes) for orientation detection
/// - Center crosshair for centering verification
/// - Gradient regions for colormap/intensity testing
/// - Frame number encoded in the pattern
/// - **Dynamic elements:**
///   - Background noise (varies each frame)
///   - Moving Gaussian hotspot that orbits the center
///   - Pulsing center ring intensity
///
/// # Arguments
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `frame_num` - Frame number (for animation/identification)
///
/// # Returns
/// A Vec<u16> containing the test pattern pixel data
fn generate_test_pattern(width: u32, height: u32, frame_num: u64) -> Vec<u16> {
    let mut buffer = vec![0u16; (width * height) as usize];
    let w = width as usize;
    let h = height as usize;

    // Size parameters scaled to image dimensions
    let checker_size = (width.min(height) / 32) as usize; // ~20 pixels for 640x480
    let corner_size = (width.min(height) / 8) as usize; // ~60 pixels for 640x480
    let crosshair_thickness = 3usize;
    let crosshair_length = (width.min(height) / 6) as usize; // ~80 pixels for 640x480
    let gradient_height = (height / 10) as usize; // 10% of height for gradient bars

    // Center coordinates
    let cx = w / 2;
    let cy = h / 2;

    // === Dynamic elements ===
    // Moving hotspot: orbits around center with period of ~120 frames (~4 sec at 30fps)
    let orbit_radius = (width.min(height) / 5) as f64;
    let angle = (frame_num as f64 * 0.05) % (2.0 * std::f64::consts::PI);
    let hotspot_x = cx as f64 + orbit_radius * angle.cos();
    let hotspot_y = cy as f64 + orbit_radius * angle.sin();
    let hotspot_radius = 30.0f64; // Gaussian sigma

    // Pulsing intensity for center ring (oscillates between 70% and 100%)
    let pulse_phase = (frame_num as f64 * 0.15).sin(); // ~0.5 Hz at 30fps
    let ring_intensity = (0.85 + 0.15 * pulse_phase) * 65535.0;

    // Noise seed based on frame number
    let frame_seed = frame_num.wrapping_mul(2654435761);

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;

            // Per-pixel noise seed (combines frame and position for spatial variation)
            let noise_seed = prng(frame_seed ^ (idx as u64));
            let noise_value = ((noise_seed & 0xFFF) as i32 - 2048) as i16; // Range: -2048 to +2047

            // Layer 1: Checkerboard background (alternating ~25% and ~30% intensity)
            let checker_x = x / checker_size;
            let checker_y = y / checker_size;
            let base_value: i32 = if (checker_x + checker_y).is_multiple_of(2) {
                16384 // ~25% of 65535
            } else {
                19660 // ~30% of 65535
            };

            // Add noise to base value (small amplitude: ~3% of full scale)
            let mut pixel_value: u16 = (base_value + noise_value as i32).clamp(0, 65535) as u16;

            // Layer 2: Gradient regions at top and bottom
            // Top gradient: 0% to 100% intensity (left to right)
            if y < gradient_height {
                pixel_value = ((x as u32 * 65535) / width) as u16;
            }
            // Bottom gradient: 100% to 0% intensity (left to right)
            if y >= h - gradient_height {
                pixel_value = (((w - 1 - x) as u32 * 65535) / width) as u16;
            }

            // Layer 3: Corner markers for orientation detection
            // Top-left: Solid bright triangle (identifies origin)
            if x < corner_size && y < corner_size && x + y < corner_size {
                pixel_value = 65535; // Full white
            }
            // Top-right: Hollow rectangle outline
            if x >= w - corner_size && y < corner_size {
                let local_x = x - (w - corner_size);
                let local_y = y;
                let border = 5;
                if local_x < border
                    || local_x >= corner_size - border
                    || local_y < border
                    || local_y >= corner_size - border
                {
                    pixel_value = 52428; // ~80% intensity
                }
            }
            // Bottom-left: Filled circle
            if x < corner_size && y >= h - corner_size {
                let local_x = x as i32;
                let local_y = (y - (h - corner_size)) as i32;
                let center = (corner_size / 2) as i32;
                let radius = (corner_size / 3) as i32;
                let dx = local_x - center;
                let dy = local_y - center;
                if dx * dx + dy * dy <= radius * radius {
                    pixel_value = 45875; // ~70% intensity
                }
            }
            // Bottom-right: X mark
            if x >= w - corner_size && y >= h - corner_size {
                let local_x = x - (w - corner_size);
                let local_y = y - (h - corner_size);
                let thickness = 6;
                // Diagonal from top-left to bottom-right
                let diff1 = (local_x as i32 - local_y as i32).unsigned_abs() as usize;
                // Diagonal from top-right to bottom-left
                let diff2 = (local_x as i32 - (corner_size as i32 - 1 - local_y as i32))
                    .unsigned_abs() as usize;
                if diff1 < thickness || diff2 < thickness {
                    pixel_value = 39321; // ~60% intensity
                }
            }

            // Layer 4: Center crosshair
            let in_horizontal = y >= cy - crosshair_thickness / 2
                && y <= cy + crosshair_thickness / 2
                && x >= cx - crosshair_length
                && x <= cx + crosshair_length;
            let in_vertical = x >= cx - crosshair_thickness / 2
                && x <= cx + crosshair_thickness / 2
                && y >= cy - crosshair_length
                && y <= cy + crosshair_length;
            if in_horizontal || in_vertical {
                pixel_value = 65535; // Full white
            }

            // Layer 5: Center circle (distinguishable marker) - PULSING
            let dx_center = (x as i32 - cx as i32).abs();
            let dy_center = (y as i32 - cy as i32).abs();
            let dist_sq_center = dx_center * dx_center + dy_center * dy_center;
            let inner_radius = (crosshair_length / 3) as i32;
            let outer_radius = inner_radius + 4;
            if dist_sq_center >= inner_radius * inner_radius
                && dist_sq_center <= outer_radius * outer_radius
            {
                pixel_value = ring_intensity as u16; // Pulsing intensity
            }

            // Layer 6: Frame number indicator (small dots in top-left area below corner marker)
            // Encode low 4 bits of frame_num as 4 dots
            let dot_y_start = corner_size + 10;
            let dot_spacing = 15usize;
            let dot_radius = 5i32;
            if y >= dot_y_start && y < dot_y_start + 20 && x < corner_size + 10 {
                for bit in 0usize..4 {
                    let dot_x = (10 + bit * dot_spacing) as i32;
                    let dot_y = (dot_y_start + 10) as i32;
                    let dx = (x as i32 - dot_x).abs();
                    let dy = (y as i32 - dot_y).abs();
                    if dx * dx + dy * dy <= dot_radius * dot_radius {
                        if (frame_num >> bit) & 1 == 1 {
                            pixel_value = 65535; // On = white
                        } else {
                            pixel_value = 6553; // Off = ~10% (visible but dim)
                        }
                    }
                }
            }

            // Layer 7: Intensity test patches (stepped grayscale) on right edge
            let patch_height = h / 8;
            let patch_width = 40;
            if x >= w - patch_width && y >= gradient_height && y < h - gradient_height {
                let patch_idx = (y - gradient_height) / patch_height;
                // 8 levels from ~12.5% to 100%
                let intensity = ((patch_idx + 1) as u32 * 65535) / 8;
                pixel_value = intensity as u16;
            }

            // Layer 8: Moving Gaussian hotspot (orbits around center)
            let dx_hotspot = x as f64 - hotspot_x;
            let dy_hotspot = y as f64 - hotspot_y;
            let dist_sq_hotspot = dx_hotspot * dx_hotspot + dy_hotspot * dy_hotspot;
            let gaussian = (-dist_sq_hotspot / (2.0 * hotspot_radius * hotspot_radius)).exp();
            // Add hotspot intensity (additive blend, max 50% of full scale)
            let hotspot_contribution = (gaussian * 32768.0) as u32;
            pixel_value = (pixel_value as u32 + hotspot_contribution).min(65535) as u16;

            buffer[idx] = pixel_value;
        }
    }

    buffer
}

// =============================================================================
// MockStage - Simulated Motion Stage
// =============================================================================

/// Mock motion stage with realistic timing
///
/// Simulates a linear stage with:
/// - 10mm/sec motion speed
/// - 50ms settling time after motion
/// - Thread-safe position tracking
///
/// # Example
///
/// ```rust,ignore
/// let stage = MockStage::new();
/// stage.move_abs(10.0).await?; // Takes ~1 second
/// assert_eq!(stage.position().await?, 10.0);
/// ```
#[allow(dead_code)]
pub struct MockStage {
    position: Parameter<f64>,
    position_state: Arc<RwLock<f64>>,
    speed_mm_per_sec: f64,
    params: ParameterSet,
}

impl MockStage {
    /// Create new mock stage at position 0.0mm
    pub fn new() -> Self {
        let mut params = ParameterSet::new();
        let position_state = Arc::new(RwLock::new(0.0));
        let position = Parameter::new("position", 0.0)
            .with_description("Stage position")
            .with_unit("mm");

        let position = Self::attach_stage_callbacks(position, position_state.clone(), 10.0);

        params.register(position.clone());

        Self {
            position,
            position_state,
            speed_mm_per_sec: 10.0, // 10mm/sec
            params,
        }
    }

    /// Create new mock stage at specified initial position
    ///
    /// # Arguments
    /// * `initial_position` - Starting position in mm
    pub fn with_position(initial_position: f64) -> Self {
        let mut params = ParameterSet::new();
        let position_state = Arc::new(RwLock::new(initial_position));
        let position = Parameter::new("position", initial_position)
            .with_description("Stage position")
            .with_unit("mm");

        let position = Self::attach_stage_callbacks(position, position_state.clone(), 10.0);

        params.register(position.clone());

        Self {
            position,
            position_state,
            speed_mm_per_sec: 10.0, // 10mm/sec
            params,
        }
    }

    /// Create mock stage with custom speed
    ///
    /// # Arguments
    /// * `speed_mm_per_sec` - Motion speed in mm/sec
    pub fn with_speed(speed_mm_per_sec: f64) -> Self {
        let mut params = ParameterSet::new();
        let position_state = Arc::new(RwLock::new(0.0));
        let position = Parameter::new("position", 0.0)
            .with_description("Stage position")
            .with_unit("mm");

        let position =
            Self::attach_stage_callbacks(position, position_state.clone(), speed_mm_per_sec);

        params.register(position.clone());

        Self {
            position,
            position_state,
            speed_mm_per_sec,
            params,
        }
    }

    fn attach_stage_callbacks(
        mut position: Parameter<f64>,
        state: Arc<RwLock<f64>>,
        speed_mm_per_sec: f64,
    ) -> Parameter<f64> {
        let state_for_write = state.clone();
        position.connect_to_hardware_write(move |target| {
            let state_for_write = state_for_write.clone();
            Box::pin(async move {
                let current = *state_for_write.read().await;
                let distance = (target - current).abs();
                let delay_ms = (distance / speed_mm_per_sec * 1000.0) as u64;

                println!(
                    "MockStage: Moving from {:.2}mm to {:.2}mm ({}ms)",
                    current, target, delay_ms
                );

                sleep(Duration::from_millis(delay_ms)).await;
                *state_for_write.write().await = target;
                println!("MockStage: Reached {:.2}mm", target);
                Ok(())
            })
        });

        let state_for_read = state.clone();
        position.connect_to_hardware_read(move || {
            let state_for_read = state_for_read.clone();
            Box::pin(async move { Ok(*state_for_read.read().await) })
        });

        position
    }
}

impl Default for MockStage {
    fn default() -> Self {
        Self::new()
    }
}

impl Parameterized for MockStage {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl Movable for MockStage {
    async fn move_abs(&self, target: f64) -> Result<()> {
        println!(
            "MockStage: command move to {:.2}mm at {:.2} mm/s",
            target, self.speed_mm_per_sec
        );
        self.position.set(target).await
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = self.position.get();
        self.move_abs(current + distance).await
    }

    async fn position(&self) -> Result<f64> {
        Ok(self.position.get())
    }

    async fn wait_settled(&self) -> Result<()> {
        println!("MockStage: Settling...");
        sleep(Duration::from_millis(50)).await; // 50ms settling time
        println!("MockStage: Settled");
        Ok(())
    }
}

// =============================================================================
// MockCamera - Simulated Camera
// =============================================================================

/// Mock camera with trigger and streaming support
///
/// Simulates a camera with:
/// - Configurable resolution
/// - 33ms frame readout (30fps)
/// - Arm/disarm triggering
/// - Start/stop streaming with broadcast support
///
/// # Example
///
/// ```rust,ignore
/// let (camera, params) = MockCamera::new(640, 480);
/// camera.arm().await?;
/// camera.trigger().await?;
/// ```
pub struct MockCamera {
    resolution: (u32, u32),
    frame_count: Arc<AtomicU64>,
    armed: Parameter<bool>,
    streaming: Parameter<bool>,
    staged: Parameter<bool>,
    exposure_s: Parameter<f64>,
    params: ParameterSet,
    /// Broadcast channel for frame streaming
    frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
    /// Reliable channel for lossless data transmission (optional)
    reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<Frame>>>>>,
    #[allow(dead_code)]
    streaming_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    streaming_flag: Arc<AtomicBool>,
    armed_flag: Arc<AtomicBool>,
    staged_flag: Arc<AtomicBool>,
}

impl MockCamera {
    /// Create new mock camera with specified resolution
    ///
    /// # Arguments
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    pub fn new(width: u32, height: u32) -> Self {
        let (frame_tx, _) = tokio::sync::broadcast::channel(16);
        let frame_count = Arc::new(AtomicU64::new(0));
        let streaming_flag = Arc::new(AtomicBool::new(false));
        let armed_flag = Arc::new(AtomicBool::new(false));
        let staged_flag = Arc::new(AtomicBool::new(false));
        let streaming_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>> =
            Arc::new(Mutex::new(None));
        let reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<Frame>>>>> =
            Arc::new(Mutex::new(None));

        // Create exposure parameter with validation and metadata
        let mut params = ParameterSet::new();
        let exposure = Parameter::new("exposure_s", 0.033)
            .with_description("Camera exposure time")
            .with_unit("s")
            .with_range(0.001, 10.0); // 1ms to 10s range

        // Armed parameter (controls trigger readiness)
        let mut armed = Parameter::new("armed", false).with_description("Camera armed");
        {
            let armed_flag_write = armed_flag.clone();
            armed.connect_to_hardware_write(move |value| {
                let armed_flag = armed_flag_write.clone();
                Box::pin(async move {
                    armed_flag.store(value, Ordering::SeqCst);
                    Ok(())
                })
            });

            let armed_flag_read = armed_flag.clone();
            armed.connect_to_hardware_read(move || {
                let armed_flag = armed_flag_read.clone();
                Box::pin(async move { Ok(armed_flag.load(Ordering::SeqCst)) })
            });
        }

        // Streaming parameter (controls frame generation loop)
        let mut streaming = Parameter::new("streaming", false).with_description("Streaming");
        {
            let streaming_flag_write = streaming_flag.clone();
            let frame_tx_write = frame_tx.clone();
            let frame_count_write = frame_count.clone();
            let streaming_task_write = streaming_task.clone();
            let reliable_tx_write = reliable_tx.clone();
            let resolution = (width, height);

            streaming.connect_to_hardware_write(move |enable| {
                let streaming_flag = streaming_flag_write.clone();
                let frame_tx = frame_tx_write.clone();
                let frame_count = frame_count_write.clone();
                let streaming_task = streaming_task_write.clone();
                let reliable_tx = reliable_tx_write.clone();

                Box::pin(async move {
                    if enable {
                        // Only start if not already streaming
                        if streaming_flag.swap(true, Ordering::SeqCst) {
                            return Ok(());
                        }

                        let mut handle_guard = streaming_task.lock().await;
                        let flag_for_task = streaming_flag.clone();
                        let tx = frame_tx.clone();
                        let reliable_tx_for_task = reliable_tx.lock().await.clone();
                        let res = resolution;
                        let count = frame_count.clone();

                        let handle = tokio::spawn(async move {
                            while flag_for_task.load(Ordering::SeqCst) {
                                let frame_num = count.fetch_add(1, Ordering::SeqCst) + 1;
                                let (w, h) = res;
                                let buffer = generate_test_pattern(w, h, frame_num);

                                let frame = Arc::new(Frame::from_u16(w, h, &buffer));

                                // Reliable Path
                                if let Some(ref r_tx) = reliable_tx_for_task {
                                    let _ = r_tx.send(frame.clone()).await;
                                }

                                // Lossy Path
                                let _ = tx.send(frame);

                                sleep(Duration::from_millis(33)).await; // ~30fps
                            }
                        });

                        *handle_guard = Some(handle);
                    } else {
                        streaming_flag.store(false, Ordering::SeqCst);
                        if let Some(handle) = streaming_task.lock().await.take() {
                            handle.abort();
                        }
                    }

                    Ok(())
                })
            });

            let streaming_flag_read = streaming_flag.clone();
            streaming.connect_to_hardware_read(move || {
                let streaming_flag = streaming_flag_read.clone();
                Box::pin(async move { Ok(streaming_flag.load(Ordering::SeqCst)) })
            });
        }

        // Staged parameter (controls readiness lifecycle)
        let mut staged = Parameter::new("staged", false).with_description("Camera staged");
        {
            let staged_flag_write = staged_flag.clone();
            let frame_count_write = frame_count.clone();
            let streaming_param_write = streaming.clone();
            let armed_param_write = armed.clone();

            staged.connect_to_hardware_write(move |is_staged| {
                let staged_flag = staged_flag_write.clone();
                let frame_count = frame_count_write.clone();
                let streaming_param = streaming_param_write.clone();
                let armed_param = armed_param_write.clone();

                Box::pin(async move {
                    staged_flag.store(is_staged, Ordering::SeqCst);

                    if is_staged {
                        frame_count.store(0, Ordering::SeqCst);
                        let _ = armed_param.set(true).await;
                    } else {
                        // Ensure streaming stops when unstaging
                        let _ = streaming_param.set(false).await;
                        let _ = armed_param.set(false).await;
                    }

                    Ok(())
                })
            });

            let staged_flag_read = staged_flag.clone();
            staged.connect_to_hardware_read(move || {
                let staged_flag = staged_flag_read.clone();
                Box::pin(async move { Ok(staged_flag.load(Ordering::SeqCst)) })
            });
        }

        // Register parameters in the parameter set
        params.register(exposure.clone());
        params.register(armed.clone());
        params.register(streaming.clone());
        params.register(staged.clone());

        Self {
            resolution: (width, height),
            frame_count,
            armed,
            streaming,
            staged,
            exposure_s: exposure,
            params,
            frame_tx,
            reliable_tx,
            streaming_task,
            streaming_flag,
            armed_flag,
            staged_flag,
        }
    }

    /// Get total number of frames captured
    pub fn get_frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::SeqCst)
    }

    /// Check if camera is currently armed
    pub async fn is_armed(&self) -> bool {
        self.armed_flag.load(Ordering::SeqCst)
    }

    /// Check if camera is streaming
    pub async fn is_streaming(&self) -> bool {
        self.streaming_flag.load(Ordering::SeqCst)
    }
}

impl Parameterized for MockCamera {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

impl Default for MockCamera {
    fn default() -> Self {
        Self::new(1920, 1080)
    }
}

#[async_trait]
impl Triggerable for MockCamera {
    async fn arm(&self) -> Result<()> {
        let already_armed = self.armed_flag.load(Ordering::SeqCst);
        if already_armed {
            println!("MockCamera: Already armed (re-arming)");
        } else {
            println!("MockCamera: Armed");
        }
        self.armed.set(true).await?;
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        // Check if armed
        if !self.armed_flag.load(Ordering::SeqCst) {
            anyhow::bail!("MockCamera: Cannot trigger - not armed");
        }

        let count = self
            .frame_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        println!("MockCamera: Triggered frame #{}", count);

        // Simulate 30fps frame readout time
        sleep(Duration::from_millis(33)).await;

        // Generate and emit frame with diagnostic test pattern
        let (w, h) = self.resolution;
        let buffer = generate_test_pattern(w, h, count);
        let frame = Arc::new(Frame::from_u16(w, h, &buffer));

        let _ = self.frame_tx.send(frame);

        println!("MockCamera: Frame #{} readout and emit complete", count);
        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(self.armed_flag.load(Ordering::SeqCst))
    }
}

#[async_trait]
impl ExposureControl for MockCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        if seconds <= 0.0 {
            return Err(anyhow!("MockCamera: Exposure must be positive"));
        }
        self.exposure_s.set(seconds).await?;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(self.exposure_s.get())
    }
}

#[async_trait]
impl FrameProducer for MockCamera {
    async fn start_stream(&self) -> Result<()> {
        let already_streaming = self.streaming_flag.load(Ordering::SeqCst);
        if already_streaming {
            anyhow::bail!("MockCamera: Already streaming");
        }

        self.streaming.set(true).await
    }

    async fn stop_stream(&self) -> Result<()> {
        let was_streaming = self.streaming_flag.load(Ordering::SeqCst);
        if !was_streaming {
            println!("MockCamera: Stream already stopped");
        } else {
            println!("MockCamera: Stream stopped");
        }

        self.streaming.set(false).await
    }

    fn resolution(&self) -> (u32, u32) {
        self.resolution
    }

    async fn is_streaming(&self) -> Result<bool> {
        Ok(self.streaming_flag.load(Ordering::SeqCst))
    }

    fn frame_count(&self) -> u64 {
        self.frame_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn subscribe_frames(&self) -> Option<tokio::sync::broadcast::Receiver<Arc<Frame>>> {
        Some(self.frame_tx.subscribe())
    }
}

#[async_trait]
impl daq_core::pipeline::MeasurementSource for MockCamera {
    type Output = Arc<Frame>;
    type Error = anyhow::Error;

    async fn register_output(
        &self,
        tx: tokio::sync::mpsc::Sender<Self::Output>,
    ) -> Result<(), Self::Error> {
        let mut reliable = self.reliable_tx.lock().await;
        *reliable = Some(tx);
        Ok(())
    }
}

#[async_trait]
impl Stageable for MockCamera {
    async fn stage(&self) -> Result<()> {
        let already_staged = self.staged_flag.load(Ordering::SeqCst);
        if already_staged {
            println!("MockCamera: Already staged (re-staging)");
        } else {
            println!("MockCamera: Staging - preparing for acquisition");
        }

        // Stage by toggling parameter (hardware writer resets counters & arms)
        self.staged.set(true).await?;

        println!("MockCamera: Staged successfully");
        Ok(())
    }

    async fn unstage(&self) -> Result<()> {
        let was_staged = self.staged_flag.load(Ordering::SeqCst);
        if !was_staged {
            println!("MockCamera: Already unstaged");
            return Ok(());
        }

        println!("MockCamera: Unstaging - cleaning up after acquisition");

        self.staged.set(false).await?;

        println!("MockCamera: Unstaged successfully");
        Ok(())
    }

    async fn is_staged(&self) -> Result<bool> {
        Ok(self.staged_flag.load(Ordering::SeqCst))
    }
}

// =============================================================================
// MockPowerMeter - Simulated Power Meter
// =============================================================================

/// Mock power meter with simulated readings
///
/// Simulates a power meter with:
/// - Configurable base power value
/// - Small random noise simulation
/// - Units in Watts
///
/// # Example
///
/// ```rust,ignore
/// let meter = MockPowerMeter::new(2.5);
/// let reading = meter.read().await?;
/// assert!((reading - 2.5).abs() < 0.1);
/// ```
pub struct MockPowerMeter {
    base_power: Parameter<f64>,
    params: ParameterSet,
}

impl MockPowerMeter {
    /// Create new mock power meter with specified base power (Watts)
    ///
    /// # Arguments
    /// * `base_power` - Base power reading in Watts
    pub fn new(base_power: f64) -> Self {
        // Create base_power parameter with validation and metadata
        let mut params = ParameterSet::new();
        let power_param = Parameter::new("base_power", base_power)
            .with_description("Base power reading for simulated measurements")
            .with_unit("W")
            .with_range(0.0, 10.0); // 0 to 10W range

        // Register parameter in the parameter set
        params.register(power_param.clone());

        Self {
            base_power: power_param,
            params,
        }
    }

    /// Set the base power reading
    pub async fn set_base_power(&self, power: f64) -> Result<()> {
        // Just delegate to parameter - no hardware for mock
        self.base_power.set(power).await
    }

    /// Get the current base power setting
    pub fn get_base_power(&self) -> f64 {
        self.base_power.get()
    }
}

impl Parameterized for MockPowerMeter {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

impl Default for MockPowerMeter {
    fn default() -> Self {
        Self::new(1.0)
    }
}

#[async_trait]
impl Readable for MockPowerMeter {
    async fn read(&self) -> Result<f64> {
        let base = self.base_power.get();

        // Add small noise (~1% variation) for realism
        // Use simple deterministic noise based on time
        let noise_factor = 1.0
            + (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                % 200) as f64
                / 10000.0
            - 0.01;

        let reading = base * noise_factor;
        Ok(reading)
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_stage_absolute_move() {
        let stage = MockStage::new();

        // Initial position should be 0
        assert_eq!(stage.position().await.unwrap(), 0.0);

        // Move to 10mm
        stage.move_abs(10.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 10.0);

        // Move to 25mm
        stage.move_abs(25.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 25.0);
    }

    #[tokio::test]
    async fn test_mock_stage_relative_move() {
        let stage = MockStage::new();

        // Move +5mm
        stage.move_rel(5.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 5.0);

        // Move +10mm
        stage.move_rel(10.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 15.0);

        // Move -3mm
        stage.move_rel(-3.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 12.0);
    }

    #[tokio::test]
    async fn test_mock_stage_settle() {
        let stage = MockStage::new();

        stage.move_abs(10.0).await.unwrap();
        stage.wait_settled().await.unwrap(); // Should not panic
    }

    #[tokio::test]
    async fn test_mock_stage_custom_speed() {
        let stage = MockStage::with_speed(20.0); // 20mm/sec

        stage.move_abs(20.0).await.unwrap();
        assert_eq!(stage.position().await.unwrap(), 20.0);
    }

    #[tokio::test]
    async fn test_mock_stage_parameter_set_moves_stage() {
        let stage = MockStage::new();
        let params = stage.parameters();

        let position_param = params
            .get_typed::<Parameter<f64>>("position")
            .expect("position parameter registered");

        position_param.set(7.5).await.unwrap();

        assert_eq!(stage.position().await.unwrap(), 7.5);
    }

    #[tokio::test]
    async fn test_mock_camera_trigger() {
        let camera = MockCamera::new(1920, 1080);

        // Should fail if not armed
        let result = camera.trigger().await;
        assert!(result.is_err());

        // Arm and trigger
        camera.arm().await.unwrap();
        assert!(camera.is_armed().await);

        camera.trigger().await.unwrap();
        assert_eq!(camera.get_frame_count(), 1);

        // Trigger again (should still work, camera stays armed)
        camera.trigger().await.unwrap();
        assert_eq!(camera.get_frame_count(), 2);
    }

    #[tokio::test]
    async fn test_mock_camera_resolution() {
        let camera = MockCamera::new(1920, 1080);
        assert_eq!(camera.resolution(), (1920, 1080));

        let camera2 = MockCamera::new(640, 480);
        assert_eq!(camera2.resolution(), (640, 480));
    }

    #[tokio::test]
    async fn test_mock_camera_streaming() {
        let camera = MockCamera::new(1920, 1080);

        // Start streaming
        camera.start_stream().await.unwrap();
        assert!(camera.is_streaming().await);

        // Cannot start twice
        let result = camera.start_stream().await;
        assert!(result.is_err());

        // Stop streaming
        camera.stop_stream().await.unwrap();
        assert!(!camera.is_streaming().await);

        // Can stop multiple times (idempotent)
        camera.stop_stream().await.unwrap();
    }

    #[tokio::test]
    async fn test_mock_camera_multiple_arms() {
        let camera = MockCamera::new(1920, 1080);

        // Can re-arm multiple times
        camera.arm().await.unwrap();
        camera.arm().await.unwrap();
        camera.arm().await.unwrap();

        assert!(camera.is_armed().await);
    }

    #[tokio::test]
    async fn test_mock_camera_parameter_set_controls_state() {
        let camera = MockCamera::new(320, 240);
        let params = camera.parameters();

        let streaming_param = params
            .get_typed::<Parameter<bool>>("streaming")
            .expect("streaming parameter registered");

        streaming_param.set(true).await.unwrap();
        assert!(camera.is_streaming().await);

        streaming_param.set(false).await.unwrap();
        assert!(!camera.is_streaming().await);

        let exposure_param = params
            .get_typed::<Parameter<f64>>("exposure_s")
            .expect("exposure parameter registered");
        exposure_param.set(0.05).await.unwrap();

        assert_eq!(camera.get_exposure().await.unwrap(), 0.05);
    }

    #[tokio::test]
    async fn test_mock_power_meter_read() {
        let meter = MockPowerMeter::new(2.5);

        // Read should return approximately the base value
        let reading = meter.read().await.unwrap();
        assert!(
            reading > 2.4 && reading < 2.6,
            "Reading {} not in expected range",
            reading
        );
    }

    #[tokio::test]
    async fn test_mock_power_meter_set_power() {
        let meter = MockPowerMeter::new(1.0);

        // Initial reading around 1.0
        let reading1 = meter.read().await.unwrap();
        assert!(reading1 > 0.9 && reading1 < 1.1);

        // Change base power
        meter.set_base_power(5.0).await.unwrap();
        assert_eq!(meter.get_base_power(), 5.0);

        // Reading should now be around 5.0
        let reading2 = meter.read().await.unwrap();
        assert!(
            reading2 > 4.9 && reading2 < 5.1,
            "Reading {} not in expected range",
            reading2
        );
    }

    #[tokio::test]
    async fn test_mock_power_meter_default() {
        let meter = MockPowerMeter::default();
        assert_eq!(meter.get_base_power(), 1.0);
    }

    #[tokio::test]
    async fn test_mock_camera_staging() {
        use crate::capabilities::Stageable;

        let camera = MockCamera::new(1920, 1080);

        // Initially not staged
        assert!(!camera.is_staged().await.unwrap());
        assert!(!camera.is_armed().await);

        // Stage the camera
        camera.stage().await.unwrap();
        assert!(camera.is_staged().await.unwrap());
        assert!(camera.is_armed().await); // staging arms the camera

        // Trigger should work after staging (since staging arms)
        camera.trigger().await.unwrap();
        assert_eq!(camera.get_frame_count(), 1);

        // Re-staging should reset frame count
        camera.stage().await.unwrap();
        assert_eq!(camera.get_frame_count(), 0);

        // Unstage the camera
        camera.unstage().await.unwrap();
        assert!(!camera.is_staged().await.unwrap());
        assert!(!camera.is_armed().await);

        // Trigger should fail after unstaging
        let result = camera.trigger().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_camera_staging_stops_streaming() {
        use crate::capabilities::Stageable;

        let camera = MockCamera::new(640, 480);

        // Stage and start streaming
        camera.stage().await.unwrap();
        camera.start_stream().await.unwrap();
        assert!(camera.is_streaming().await);

        // Unstaging should stop streaming
        camera.unstage().await.unwrap();
        assert!(!camera.is_streaming().await);
    }
}
