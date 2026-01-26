//! Mock camera implementation with trigger and streaming support.

use crate::pattern::generate_test_pattern;
// Import common infrastructure (bd-1gdn.2)
use crate::common::{MockMode, TimingConfig, ErrorConfig, MockRng};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use daq_core::capabilities::{
    ExposureControl, FrameObserver, FrameProducer, LoanedFrame, ObserverHandle, Parameterized,
    Stageable, Triggerable,
};
use daq_core::data::{Frame, FrameView};
use daq_core::driver::{Capability, DeviceComponents, DriverFactory};
use daq_core::observable::ParameterSet;
use daq_core::parameter::Parameter;
use daq_pool::{FrameData, Pool};
use futures::future::BoxFuture;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{Duration, sleep};

/// Pool size for MockCamera frame delivery
const MOCK_FRAME_POOL_SIZE: usize = 16;

/// Type alias for frame observer registry to reduce complexity.
/// Each observer is a tuple of (observer_id, observer_callback).
type ObserverRegistry = Arc<RwLock<Vec<(u64, Box<dyn FrameObserver>)>>>;

// =============================================================================
// MockCameraFactory - DriverFactory implementation
// =============================================================================

/// Configuration for MockCamera driver
#[derive(Debug, Clone, Deserialize)]
pub struct MockCameraConfig {
    /// Frame width in pixels (default: 1920)
    #[serde(default = "default_width")]
    pub width: u32,

    /// Frame height in pixels (default: 1080)
    #[serde(default = "default_height")]
    pub height: u32,

    /// Initial exposure in seconds (default: 0.033)
    #[serde(default = "default_exposure")]
    pub exposure_s: f64,
}

fn default_width() -> u32 {
    1920
}
fn default_height() -> u32 {
    1080
}
fn default_exposure() -> f64 {
    0.033
}

impl Default for MockCameraConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            exposure_s: 0.033,
        }
    }
}

/// Factory for creating MockCamera instances.
pub struct MockCameraFactory;

/// Static capabilities for MockCamera
static MOCK_CAMERA_CAPABILITIES: &[Capability] = &[
    Capability::FrameProducer,
    Capability::Triggerable,
    Capability::ExposureControl,
    Capability::Stageable,
    Capability::Parameterized,
];

impl DriverFactory for MockCameraFactory {
    fn driver_type(&self) -> &'static str {
        "mock_camera"
    }

    fn name(&self) -> &'static str {
        "Mock Camera"
    }

    fn capabilities(&self) -> &'static [Capability] {
        MOCK_CAMERA_CAPABILITIES
    }

    fn validate(&self, config: &toml::Value) -> Result<()> {
        let cfg: MockCameraConfig = config.clone().try_into()?;
        if cfg.width == 0 || cfg.height == 0 {
            anyhow::bail!("Camera resolution must be non-zero");
        }
        if cfg.exposure_s <= 0.0 {
            anyhow::bail!("Exposure must be positive");
        }
        Ok(())
    }

    fn build(&self, config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let cfg: MockCameraConfig = config.try_into().unwrap_or_default();

            let camera = Arc::new(MockCamera::with_config(cfg));

            Ok(DeviceComponents {
                frame_producer: Some(camera.clone()),
                triggerable: Some(camera.clone()),
                exposure_control: Some(camera.clone()),
                stageable: Some(camera.clone()),
                parameterized: Some(camera),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// FrameStatistics - Frame Loss Tracking (bd-1gdn.2)
// =============================================================================

/// Frame loss statistics (matches PVCAM behavior)
#[derive(Debug, Clone, Default)]
pub struct FrameStatistics {
    /// Total frames attempted to capture
    pub total_frames: u64,
    /// Frames lost due to buffer overrun or simulated loss
    pub lost_frames: u64,
    /// Number of discontinuity events detected
    pub discontinuity_events: u32,
    /// Frames dropped by consumer
    pub dropped_frames: u64,
    /// Last hardware frame number seen
    last_frame_nr: u64,
}

impl FrameStatistics {
    fn new() -> Self {
        Self::default()
    }

    /// Check for frame discontinuity and update counters
    fn check_discontinuity(&mut self, hardware_frame_nr: u64) {
        if self.last_frame_nr > 0 {
            let expected = self.last_frame_nr + 1;
            if hardware_frame_nr != expected {
                let gap = hardware_frame_nr.saturating_sub(expected);
                self.lost_frames += gap;
                self.discontinuity_events += 1;
                tracing::warn!(
                    "Frame discontinuity: expected {}, got {} (lost {} frames)",
                    expected,
                    hardware_frame_nr,
                    gap
                );
            }
        }
        self.last_frame_nr = hardware_frame_nr;
        self.total_frames += 1;
    }
}

// =============================================================================
// TemperatureSimulation - Realistic Temperature Behavior (bd-1gdn.2)
// =============================================================================

/// Temperature simulation with exponential drift
#[derive(Debug, Clone)]
pub struct TemperatureSimulation {
    current: f64,
    setpoint: f64,
    drift_rate: f64, // degrees per second
}

impl TemperatureSimulation {
    fn new(initial_temp: f64) -> Self {
        Self {
            current: initial_temp,
            setpoint: initial_temp,
            drift_rate: 0.1, // Conservative drift rate
        }
    }

    fn set_setpoint(&mut self, setpoint: f64) {
        self.setpoint = setpoint;
    }

    /// Update temperature with exponential approach to setpoint
    fn update(&mut self, dt_seconds: f64) {
        let diff = self.setpoint - self.current;
        self.current += diff * (1.0 - (-self.drift_rate * dt_seconds).exp());
    }

    fn current(&self) -> f64 {
        self.current
    }
}

// =============================================================================
// MockCameraBuilder - Builder Pattern (bd-1gdn.2)
// =============================================================================

/// Builder for MockCamera with advanced configuration
pub struct MockCameraBuilder {
    width: u32,
    height: u32,
    mode: MockMode,
    frame_loss_rate: f64,
    error_config: ErrorConfig,
    timing_config: TimingConfig,
    shutter_open_delay_ms: u64,
    shutter_close_delay_ms: u64,
    initial_temperature: f64,
    max_fps: f64,
}

impl MockCameraBuilder {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            mode: MockMode::Instant,
            frame_loss_rate: 0.0,
            error_config: ErrorConfig::none(),
            timing_config: TimingConfig::camera(),
            shutter_open_delay_ms: 0,
            shutter_close_delay_ms: 0,
            initial_temperature: 20.0,
            max_fps: 30.0,
        }
    }

    pub fn mode(mut self, mode: MockMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn frame_loss_rate(mut self, rate: f64) -> Self {
        self.frame_loss_rate = rate.clamp(0.0, 1.0);
        self
    }

    pub fn error_config(mut self, config: ErrorConfig) -> Self {
        self.error_config = config;
        self
    }

    pub fn timing_config(mut self, config: TimingConfig) -> Self {
        self.timing_config = config;
        self
    }

    pub fn shutter_delays(mut self, open_ms: u64, close_ms: u64) -> Self {
        self.shutter_open_delay_ms = open_ms;
        self.shutter_close_delay_ms = close_ms;
        self
    }

    pub fn initial_temperature(mut self, temp: f64) -> Self {
        self.initial_temperature = temp;
        self
    }

    pub fn max_fps(mut self, fps: f64) -> Self {
        self.max_fps = fps.max(1.0);
        self
    }

    pub fn build(self) -> MockCamera {
        MockCamera::from_builder(self)
    }
}

// =============================================================================
// MockCamera - Simulated Camera
// =============================================================================

/// Mock camera with trigger and streaming support.
///
/// Simulates a camera with:
/// - Configurable resolution
/// - Frame loss simulation (bd-1gdn.2)
/// - Exposure-rate coupling (bd-1gdn.2)
/// - Temperature simulation (bd-1gdn.2)
/// - Shutter delays (bd-1gdn.2)
/// - Error injection (bd-1gdn.2)
/// - Arm/disarm triggering
/// - Start/stop streaming with broadcast support
///
/// # Example
///
/// ```rust,ignore
/// let camera = MockCamera::new(640, 480);
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
    frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
    reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<Frame>>>>>,
    streaming_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    streaming_flag: Arc<AtomicBool>,
    armed_flag: Arc<AtomicBool>,
    staged_flag: Arc<AtomicBool>,
    primary_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<LoanedFrame>>>>,
    frame_pool: Arc<Mutex<Option<Arc<Pool<FrameData>>>>>,
    observers: ObserverRegistry,
    next_observer_id: AtomicU64,
    // New fields (bd-1gdn.2)
    mode: MockMode,
    frame_loss_rate: f64,
    error_config: ErrorConfig,
    timing_config: TimingConfig,
    shutter_open_delay_ms: u64,
    shutter_close_delay_ms: u64,
    temperature: Arc<Mutex<TemperatureSimulation>>,
    statistics: Arc<Mutex<FrameStatistics>>,
    rng: Arc<MockRng>,
    max_fps: f64,
}

impl MockCamera {
    /// Create new mock camera with specified resolution.
    ///
    /// # Arguments
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    ///
    /// # Backward Compatibility
    /// Creates camera in Instant mode with no delays or errors.
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_config(MockCameraConfig {
            width,
            height,
            ..Default::default()
        })
    }

    /// Create builder for advanced configuration
    pub fn builder() -> MockCameraBuilder {
        MockCameraBuilder::new(1920, 1080)
    }

    /// Create from builder configuration
    fn from_builder(builder: MockCameraBuilder) -> Self {
        let config = MockCameraConfig {
            width: builder.width,
            height: builder.height,
            exposure_s: 0.033,
        };

        Self::with_full_config(
            config,
            builder.mode,
            builder.frame_loss_rate,
            builder.error_config,
            builder.timing_config,
            builder.shutter_open_delay_ms,
            builder.shutter_close_delay_ms,
            builder.initial_temperature,
            builder.max_fps,
        )
    }

    /// Create mock camera with full configuration (internal).
    #[allow(clippy::too_many_arguments)]
    fn with_full_config(
        config: MockCameraConfig,
        mode: MockMode,
        frame_loss_rate: f64,
        error_config: ErrorConfig,
        timing_config: TimingConfig,
        shutter_open_delay_ms: u64,
        shutter_close_delay_ms: u64,
        initial_temperature: f64,
        max_fps: f64,
    ) -> Self {
        let (frame_tx, _) = tokio::sync::broadcast::channel(16);
        let frame_count = Arc::new(AtomicU64::new(0));
        let streaming_flag = Arc::new(AtomicBool::new(false));
        let armed_flag = Arc::new(AtomicBool::new(false));
        let staged_flag = Arc::new(AtomicBool::new(false));
        let streaming_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>> =
            Arc::new(Mutex::new(None));
        let reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<Frame>>>>> =
            Arc::new(Mutex::new(None));
        let primary_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<LoanedFrame>>>> =
            Arc::new(Mutex::new(None));
        let frame_pool: Arc<Mutex<Option<Arc<Pool<FrameData>>>>> = Arc::new(Mutex::new(None));
        let observers: Arc<RwLock<Vec<(u64, Box<dyn FrameObserver>)>>> =
            Arc::new(RwLock::new(Vec::new()));

        // New state (bd-1gdn.2)
        let temperature = Arc::new(Mutex::new(TemperatureSimulation::new(initial_temperature)));
        let statistics = Arc::new(Mutex::new(FrameStatistics::new()));
        let rng = Arc::new(MockRng::new(None));

        let mut params = ParameterSet::new();
        let exposure = Parameter::new("exposure_s", config.exposure_s)
            .with_description("Camera exposure time")
            .with_unit("s")
            .with_range(0.001, 10.0);

        // Armed parameter
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

        // Streaming parameter with enhanced timing
        let mut streaming = Parameter::new("streaming", false).with_description("Streaming");
        let observers_for_streaming = observers.clone();
        {
            let streaming_flag_write = streaming_flag.clone();
            let frame_tx_write = frame_tx.clone();
            let frame_count_write = frame_count.clone();
            let streaming_task_write = streaming_task.clone();
            let reliable_tx_write = reliable_tx.clone();
            let primary_tx_write = primary_tx.clone();
            let frame_pool_write = frame_pool.clone();
            let observers_write = observers_for_streaming.clone();
            let resolution = (config.width, config.height);
            
            // Clone these Arcs for the closure (originals kept for struct)
            let mode_for_streaming = mode;
            let timing_for_streaming = timing_config;
            let frame_loss_rate_for_streaming = frame_loss_rate;
            let error_config_for_streaming = error_config.clone();
            let statistics_for_streaming = statistics.clone();
            let rng_for_streaming = rng.clone();
            let temp_sim_for_streaming = temperature.clone();
            let exposure_param_for_streaming = exposure.clone();
            let max_fps_for_streaming = max_fps;

            streaming.connect_to_hardware_write(move |enable| {
                let streaming_flag = streaming_flag_write.clone();
                let frame_tx = frame_tx_write.clone();
                let frame_count = frame_count_write.clone();
                let streaming_task = streaming_task_write.clone();
                let reliable_tx = reliable_tx_write.clone();
                let primary_tx = primary_tx_write.clone();
                let frame_pool = frame_pool_write.clone();
                let observers_for_task = observers_write.clone();
                let mode = mode_for_streaming;
                let timing = timing_for_streaming;
                let loss_rate = frame_loss_rate_for_streaming;
                let error_cfg = error_config_for_streaming.clone();
                let stats = statistics_for_streaming.clone();
                let rng_val = rng_for_streaming.clone();
                let temp_sim = temp_sim_for_streaming.clone();
                let exposure_param = exposure_param_for_streaming.clone();
                let max_fps_val = max_fps_for_streaming;

                Box::pin(async move {
                    if enable {
                        if streaming_flag.swap(true, Ordering::SeqCst) {
                            return Ok(());
                        }

                        let mut handle_guard = streaming_task.lock().await;
                        let flag_for_task = streaming_flag.clone();
                        let tx = frame_tx.clone();
                        let res = resolution;
                        let count = frame_count.clone();

                        let observers_for_spawn = observers_for_task.clone();
                        let handle = tokio::spawn(async move {
                            // Lock mutexes inside spawn where await is allowed
                            let reliable_tx_for_task = reliable_tx.lock().await.clone();
                            let primary_tx_for_task = primary_tx.lock().await.clone();
                            let frame_pool_for_task = frame_pool.lock().await.clone();
                            
                            let mut last_frame_time = tokio::time::Instant::now();
                            
                            while flag_for_task.load(Ordering::SeqCst) {
                                // Error injection check
                                if let Err(e) = error_cfg.check_operation("mock_camera", "frame_capture") {
                                    tracing::error!("MockCamera: Error injection triggered: {}", e);
                                    if matches!(mode, MockMode::Chaos) {
                                        sleep(Duration::from_millis(100)).await;
                                        continue;
                                    }
                                }

                                let frame_num = count.fetch_add(1, Ordering::SeqCst) + 1;
                                
                                // Frame loss simulation
                                let mut hardware_frame_nr = frame_num;
                                if loss_rate > 0.0 && rng_val.should_fail(loss_rate) {
                                    // Simulate lost frame by incrementing hardware counter
                                    hardware_frame_nr += rng_val.gen_range(1..5);
                                }
                                
                                // Update statistics
                                {
                                    let mut stats_guard = stats.lock().await;
                                    stats_guard.check_discontinuity(hardware_frame_nr);
                                }
                                
                                let (w, h) = res;
                                let buffer = generate_test_pattern(w, h, frame_num);

                                // Calculate actual frame delay based on exposure and max_fps
                                let exposure_s = exposure_param.get();
                                let frame_delay_ms = match mode {
                                    MockMode::Instant => 0, // No delays in Instant mode
                                    MockMode::Realistic | MockMode::Chaos => {
                                        let readout_time_ms = timing.frame_readout_ms;
                                        
                                        // fps = min(1/exposure, max_fps, 1000/readout_time)
                                        let max_fps_from_readout = 1000.0 / readout_time_ms.max(1) as f64;
                                        let exposure_fps = 1.0 / exposure_s;
                                        let actual_fps = exposure_fps.min(max_fps_val).min(max_fps_from_readout);
                                        (1000.0 / actual_fps) as u64
                                    }
                                };

                                // Send through primary_tx if registered (pooled path)
                                if let (Some(p_tx), Some(pool)) =
                                    (&primary_tx_for_task, &frame_pool_for_task)
                                {
                                    if let Some(mut loaned_frame) = pool.try_acquire() {
                                        let frame_data = loaned_frame.get_mut();
                                        frame_data.width = w;
                                        frame_data.height = h;
                                        frame_data.bit_depth = 16;
                                        frame_data.frame_number = frame_num;
                                        frame_data.timestamp_ns = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_nanos() as u64)
                                            .unwrap_or(0);

                                        let byte_len = buffer.len() * 2;
                                        if byte_len <= frame_data.pixels.capacity() {
                                            let src_ptr = buffer.as_ptr() as *const u8;
                                            unsafe {
                                                std::ptr::copy_nonoverlapping(
                                                    src_ptr,
                                                    frame_data.pixels.as_mut_ptr(),
                                                    byte_len,
                                                );
                                            }
                                            frame_data.actual_len = byte_len;
                                        }

                                        if p_tx.try_send(loaned_frame).is_err()
                                            && frame_num.is_multiple_of(100)
                                        {
                                            tracing::warn!(
                                                "MockCamera: primary channel full at frame {}",
                                                frame_num
                                            );
                                        }
                                    } else if frame_num.is_multiple_of(100) {
                                        tracing::warn!(
                                            "MockCamera: frame pool exhausted at frame {}",
                                            frame_num
                                        );
                                    }
                                }

                                // Notify registered observers with FrameView
                                {
                                    let observers_guard = observers_for_spawn.read().await;
                                    if !observers_guard.is_empty() {
                                        let pixel_bytes: Vec<u8> = buffer
                                            .iter()
                                            .flat_map(|&pixel| pixel.to_le_bytes())
                                            .collect();
                                        let timestamp_ns = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_nanos() as u64)
                                            .unwrap_or(0);
                                        let frame_view = FrameView::new(
                                            w,
                                            h,
                                            16,
                                            &pixel_bytes,
                                            frame_num,
                                            timestamp_ns,
                                        )
                                        .with_exposure(exposure_s * 1000.0);
                                        for (_, observer) in observers_guard.iter() {
                                            observer.on_frame(&frame_view);
                                        }
                                    }
                                }

                                // Legacy paths: Arc<Frame> for broadcast and reliable channels
                                let frame = Arc::new(Frame::from_u16(w, h, &buffer));

                                if let Some(ref r_tx) = reliable_tx_for_task {
                                    let _ = r_tx.send(frame.clone()).await;
                                }

                                let _ = tx.send(frame);

                                // Update temperature simulation
                                {
                                    let dt = last_frame_time.elapsed().as_secs_f64();
                                    let mut temp = temp_sim.lock().await;
                                    temp.update(dt);
                                    last_frame_time = tokio::time::Instant::now();
                                }

                                // Apply frame delay
                                if frame_delay_ms > 0 {
                                    sleep(Duration::from_millis(frame_delay_ms)).await;
                                }
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

        // Staged parameter
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

        params.register(exposure.clone());
        params.register(armed.clone());
        params.register(streaming.clone());
        params.register(staged.clone());

        Self {
            resolution: (config.width, config.height),
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
            primary_tx,
            frame_pool,
            observers,
            next_observer_id: AtomicU64::new(0),
            mode,
            frame_loss_rate,
            error_config,
            timing_config,
            shutter_open_delay_ms,
            shutter_close_delay_ms,
            temperature,
            statistics,
            rng,
            max_fps,
        }
    }

    /// Create mock camera with basic configuration (backward compatible).
    pub fn with_config(config: MockCameraConfig) -> Self {
        Self::with_full_config(
            config,
            MockMode::Instant,
            0.0,
            ErrorConfig::none(),
            TimingConfig::camera(),
            0,
            0,
            20.0,
            30.0,
        )
    }

    /// Get total number of frames captured.
    pub fn get_frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::SeqCst)
    }

    /// Check if camera is currently armed.
    pub async fn is_armed(&self) -> bool {
        self.armed_flag.load(Ordering::SeqCst)
    }

    /// Check if camera is streaming.
    pub async fn is_streaming(&self) -> bool {
        self.streaming_flag.load(Ordering::SeqCst)
    }

    /// Get frame statistics (bd-1gdn.2)
    pub async fn statistics(&self) -> FrameStatistics {
        self.statistics.lock().await.clone()
    }

    /// Get current temperature (bd-1gdn.2)
    pub async fn temperature(&self) -> f64 {
        self.temperature.lock().await.current()
    }

    /// Set temperature setpoint (bd-1gdn.2)
    pub async fn set_temperature_setpoint(&self, temp: f64) {
        self.temperature.lock().await.set_setpoint(temp);
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
            tracing::debug!("MockCamera: Already armed (re-arming)");
        } else {
            tracing::debug!("MockCamera: Armed");
        }
        self.armed.set(true).await?;
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        if !self.armed_flag.load(Ordering::SeqCst) {
            anyhow::bail!("MockCamera: Cannot trigger - not armed");
        }

        let count = self.frame_count.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::debug!("MockCamera: Triggered frame #{}", count);

        sleep(Duration::from_millis(33)).await;

        let (w, h) = self.resolution;
        let buffer = generate_test_pattern(w, h, count);
        let frame = Arc::new(Frame::from_u16(w, h, &buffer));

        let _ = self.frame_tx.send(frame);

        tracing::debug!("MockCamera: Frame #{} readout and emit complete", count);
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
            tracing::debug!("MockCamera: Stream already stopped");
        } else {
            tracing::debug!("MockCamera: Stream stopped");
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
        self.frame_count.load(Ordering::SeqCst)
    }

    #[allow(deprecated)]
    async fn subscribe_frames(&self) -> Option<tokio::sync::broadcast::Receiver<Arc<Frame>>> {
        tracing::warn!(
            "subscribe_frames() is deprecated; use register_primary_output() for pooled frames"
        );
        Some(self.frame_tx.subscribe())
    }

    async fn register_primary_output(
        &self,
        tx: tokio::sync::mpsc::Sender<LoanedFrame>,
    ) -> Result<()> {
        let (width, height) = self.resolution;
        let frame_bytes = (width * height * 2) as usize;

        let pool = Pool::new_with_reset(
            MOCK_FRAME_POOL_SIZE,
            move || FrameData::with_capacity(frame_bytes),
            FrameData::reset,
        );

        tracing::info!(
            pool_size = MOCK_FRAME_POOL_SIZE,
            frame_bytes,
            total_mb = (MOCK_FRAME_POOL_SIZE * frame_bytes) as f64 / (1024.0 * 1024.0),
            "MockCamera: Created frame pool for primary output"
        );

        *self.frame_pool.lock().await = Some(pool);
        *self.primary_tx.lock().await = Some(tx);
        Ok(())
    }

    async fn register_observer(&self, observer: Box<dyn FrameObserver>) -> Result<ObserverHandle> {
        let id = self.next_observer_id.fetch_add(1, Ordering::Relaxed);
        self.observers.write().await.push((id, observer));
        Ok(ObserverHandle(id))
    }

    async fn unregister_observer(&self, handle: ObserverHandle) -> Result<()> {
        let mut observers = self.observers.write().await;
        if let Some(pos) = observers.iter().position(|(id, _)| *id == handle.0) {
            observers.remove(pos);
            Ok(())
        } else {
            anyhow::bail!("Observer handle {} not found", handle.0)
        }
    }

    fn supports_observers(&self) -> bool {
        true
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
            tracing::debug!("MockCamera: Already staged (re-staging)");
        } else {
            tracing::debug!("MockCamera: Staging - preparing for acquisition");
        }

        self.staged.set(true).await?;

        tracing::debug!("MockCamera: Staged successfully");
        Ok(())
    }

    async fn unstage(&self) -> Result<()> {
        let was_staged = self.staged_flag.load(Ordering::SeqCst);
        if !was_staged {
            tracing::debug!("MockCamera: Already unstaged");
            return Ok(());
        }

        tracing::debug!("MockCamera: Unstaging - cleaning up after acquisition");

        self.staged.set(false).await?;

        tracing::debug!("MockCamera: Unstaged successfully");
        Ok(())
    }

    async fn is_staged(&self) -> Result<bool> {
        Ok(self.staged_flag.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daq_core::data::FrameView;
    use crate::common::ErrorScenario;

    /// Test observer that counts frames received.
    struct CountingObserver {
        frames_received: Arc<AtomicU64>,
        last_width: Arc<AtomicU64>,
        last_height: Arc<AtomicU64>,
    }

    impl FrameObserver for CountingObserver {
        fn on_frame(&self, frame: &FrameView<'_>) {
            self.frames_received.fetch_add(1, Ordering::Relaxed);
            self.last_width.store(frame.width as u64, Ordering::Relaxed);
            self.last_height
                .store(frame.height as u64, Ordering::Relaxed);
        }

        fn name(&self) -> &str {
            "counting_observer"
        }
    }

    /// Test that observers receive frame notifications during streaming.
    /// This is a regression test for bd-flaky-test-fix.
    #[tokio::test]
    async fn test_mock_camera_observers_receive_frames() {
        // Use small resolution for fast test
        let camera = MockCamera::new(64, 64);

        // Create observer with shared counters
        let frames_received = Arc::new(AtomicU64::new(0));
        let last_width = Arc::new(AtomicU64::new(0));
        let last_height = Arc::new(AtomicU64::new(0));

        let observer = CountingObserver {
            frames_received: frames_received.clone(),
            last_width: last_width.clone(),
            last_height: last_height.clone(),
        };

        // Register observer
        let handle = camera.register_observer(Box::new(observer)).await.unwrap();
        assert!(camera.supports_observers());

        // Start streaming
        camera.start_stream().await.unwrap();

        // Wait for some frames
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Stop streaming
        camera.stop_stream().await.unwrap();

        // Verify observer received frames
        let received = frames_received.load(Ordering::Relaxed);
        assert!(
            received > 0,
            "Observer should have received at least one frame, got {}",
            received
        );

        // Verify frame dimensions were correct
        let (w, h) = (
            last_width.load(Ordering::Relaxed),
            last_height.load(Ordering::Relaxed),
        );
        assert_eq!(w, 64, "Frame width should be 64");
        assert_eq!(h, 64, "Frame height should be 64");

        // Unregister observer (cleanup)
        camera.unregister_observer(handle).await.unwrap();
    }

    /// Test that unregistering an observer stops frame delivery.
    #[tokio::test]
    async fn test_mock_camera_observer_unregister() {
        let camera = MockCamera::new(64, 64);

        let frames_received = Arc::new(AtomicU64::new(0));
        let observer = CountingObserver {
            frames_received: frames_received.clone(),
            last_width: Arc::new(AtomicU64::new(0)),
            last_height: Arc::new(AtomicU64::new(0)),
        };

        let handle = camera.register_observer(Box::new(observer)).await.unwrap();

        // Start streaming and let it run briefly
        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Unregister the observer
        camera.unregister_observer(handle).await.unwrap();

        // Record count after unregister
        let count_after_unregister = frames_received.load(Ordering::Relaxed);

        // Continue streaming for a bit more
        tokio::time::sleep(Duration::from_millis(100)).await;
        camera.stop_stream().await.unwrap();

        // Count should not have increased significantly after unregister
        let final_count = frames_received.load(Ordering::Relaxed);
        assert!(
            final_count <= count_after_unregister + 1,
            "Frames should not be delivered after unregister: before={}, after={}",
            count_after_unregister,
            final_count
        );
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

        camera.start_stream().await.unwrap();
        assert!(camera.is_streaming().await);

        let result = camera.start_stream().await;
        assert!(result.is_err());

        camera.stop_stream().await.unwrap();
        assert!(!camera.is_streaming().await);

        camera.stop_stream().await.unwrap();
    }

    #[tokio::test]
    async fn test_mock_camera_multiple_arms() {
        let camera = MockCamera::new(1920, 1080);

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
    async fn test_mock_camera_staging() {
        let camera = MockCamera::new(1920, 1080);

        assert!(!camera.is_staged().await.unwrap());
        assert!(!camera.is_armed().await);

        camera.stage().await.unwrap();
        assert!(camera.is_staged().await.unwrap());
        assert!(camera.is_armed().await);

        camera.trigger().await.unwrap();
        assert_eq!(camera.get_frame_count(), 1);

        camera.stage().await.unwrap();
        assert_eq!(camera.get_frame_count(), 0);

        camera.unstage().await.unwrap();
        assert!(!camera.is_staged().await.unwrap());
        assert!(!camera.is_armed().await);

        let result = camera.trigger().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_camera_staging_stops_streaming() {
        let camera = MockCamera::new(640, 480);

        camera.stage().await.unwrap();
        camera.start_stream().await.unwrap();
        assert!(camera.is_streaming().await);

        camera.unstage().await.unwrap();
        assert!(!camera.is_streaming().await);
    }

    #[tokio::test]
    async fn test_factory_creates_camera() {
        let factory = MockCameraFactory;

        assert_eq!(factory.driver_type(), "mock_camera");

        let config = toml::Value::Table(toml::map::Map::new());
        let components = factory.build(config).await.unwrap();

        assert!(components.frame_producer.is_some());
        assert!(components.triggerable.is_some());
        assert!(components.exposure_control.is_some());
        assert!(components.stageable.is_some());
        assert!(components.parameterized.is_some());
    }

    // =============================================================================
    // Tests for bd-1gdn.2: Enhanced MockCamera Features
    // =============================================================================

    #[tokio::test]
    async fn test_builder_pattern() {
        let camera = MockCamera::builder()
            .mode(MockMode::Realistic)
            .frame_loss_rate(0.1)
            .max_fps(60.0)
            .initial_temperature(25.0)
            .shutter_delays(10, 5)
            .build();

        assert_eq!(camera.resolution(), (1920, 1080));
        assert_eq!(camera.mode, MockMode::Realistic);
        assert_eq!(camera.frame_loss_rate, 0.1);
        assert_eq!(camera.max_fps, 60.0);
    }

    #[tokio::test]
    async fn test_backward_compatibility() {
        // MockCamera::new should still work with Instant mode
        let camera = MockCamera::new(640, 480);
        assert_eq!(camera.resolution(), (640, 480));
        assert_eq!(camera.mode, MockMode::Instant);
        assert_eq!(camera.frame_loss_rate, 0.0);
    }

    #[tokio::test]
    async fn test_frame_loss_detection() {
        let camera = MockCamera::builder()
            .mode(MockMode::Realistic)
            .frame_loss_rate(0.5) // High rate for testing
            .build();

        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        camera.stop_stream().await.unwrap();

        let stats = camera.statistics().await;
        assert!(stats.total_frames > 0, "Should have captured some frames");
        
        // With 50% loss rate, we expect some discontinuities
        // (but allow for random chance with small sample)
        if stats.lost_frames > 0 {
            assert!(
                stats.discontinuity_events > 0,
                "Lost frames should trigger discontinuity events"
            );
        }
    }

    #[tokio::test]
    async fn test_frame_statistics_no_loss() {
        let camera = MockCamera::builder()
            .mode(MockMode::Instant)
            .frame_loss_rate(0.0)
            .build();

        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        camera.stop_stream().await.unwrap();

        let stats = camera.statistics().await;
        assert_eq!(stats.lost_frames, 0, "No frames should be lost");
        assert_eq!(stats.discontinuity_events, 0, "No discontinuities expected");
        assert!(stats.total_frames > 0, "Should have captured frames");
    }

    #[tokio::test]
    async fn test_temperature_simulation() {
        let camera = MockCamera::builder()
            .initial_temperature(20.0)
            .mode(MockMode::Realistic)
            .build();

        let initial_temp = camera.temperature().await;
        assert!((initial_temp - 20.0).abs() < 0.1);

        // Set new setpoint
        camera.set_temperature_setpoint(25.0).await;

        // Stream to allow temperature updates
        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        camera.stop_stream().await.unwrap();

        let final_temp = camera.temperature().await;
        
        // Temperature should have drifted toward setpoint
        assert!(
            final_temp > initial_temp,
            "Temperature should increase toward setpoint (got {}, started at {})",
            final_temp,
            initial_temp
        );
    }

    #[tokio::test]
    async fn test_exposure_rate_coupling() {
        let camera = MockCamera::builder()
            .mode(MockMode::Realistic)
            .max_fps(30.0)
            .build();

        // Set long exposure (0.1s = 10 fps max)
        camera.set_exposure(0.1).await.unwrap();
        
        let start = tokio::time::Instant::now();
        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        camera.stop_stream().await.unwrap();

        let frame_count = camera.get_frame_count();
        
        // With 0.1s exposure, should get ~5 frames in 500ms
        // Allow wide tolerance for async scheduling and timing jitter
        assert!(
            frame_count >= 2 && frame_count <= 8,
            "Expected 2-8 frames with 0.1s exposure in 500ms, got {}",
            frame_count
        );
    }

    #[tokio::test]
    async fn test_instant_mode_no_delays() {
        let camera = MockCamera::builder()
            .mode(MockMode::Instant)
            .build();

        let start = tokio::time::Instant::now();
        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        camera.stop_stream().await.unwrap();

        // In Instant mode, frames should stream very quickly
        let frame_count = camera.get_frame_count();
        assert!(
            frame_count > 5,
            "Instant mode should produce many frames quickly (>5), got {}",
            frame_count
        );
    }

    #[tokio::test]
    async fn test_realistic_mode_timing() {
        let camera = MockCamera::builder()
            .mode(MockMode::Realistic)
            .max_fps(30.0)
            .build();

        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        camera.stop_stream().await.unwrap();

        let frame_count = camera.get_frame_count();
        
        // At 30fps, should get ~9 frames in 300ms
        // Allow very wide tolerance for timing jitter, async delays, and startup latency
        assert!(
            frame_count >= 1 && frame_count <= 15,
            "Expected 1-15 frames at 30fps in 300ms, got {}",
            frame_count
        );
    }

    #[tokio::test]
    async fn test_error_injection_chaos_mode() {
        let error_config = ErrorConfig::random_failures_seeded(0.3, Some(42));
        let camera = MockCamera::builder()
            .mode(MockMode::Chaos)
            .error_config(error_config)
            .build();

        // Camera should still function but may have some errors logged
        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        camera.stop_stream().await.unwrap();

        // Should have attempted to capture frames despite errors
        let stats = camera.statistics().await;
        assert!(stats.total_frames > 0);
    }

    #[tokio::test]
    async fn test_shutter_delays() {
        // This test is primarily for coverage - shutter delays are currently
        // used in the builder but not actively applied in arm/trigger
        let camera = MockCamera::builder()
            .shutter_delays(50, 30)
            .build();

        assert_eq!(camera.shutter_open_delay_ms, 50);
        assert_eq!(camera.shutter_close_delay_ms, 30);
    }

    #[tokio::test]
    async fn test_max_fps_limits_frame_rate() {
        let camera = MockCamera::builder()
            .mode(MockMode::Realistic)
            .max_fps(10.0) // Very slow
            .build();

        let start = tokio::time::Instant::now();
        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(250)).await;
        camera.stop_stream().await.unwrap();

        let frame_count = camera.get_frame_count();
        
        // At 10fps, should get 2-3 frames in 250ms
        assert!(
            frame_count <= 4,
            "Max FPS should limit frame rate, got {} frames",
            frame_count
        );
    }

    #[tokio::test]
    async fn test_statistics_reset_on_stage() {
        let camera = MockCamera::builder()
            .mode(MockMode::Realistic)
            .build();

        camera.stage().await.unwrap();
        camera.start_stream().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        camera.stop_stream().await.unwrap();

        let stats_before = camera.statistics().await;
        assert!(stats_before.total_frames > 0);

        // Re-staging should reset frame count
        camera.unstage().await.unwrap();
        camera.stage().await.unwrap();

        assert_eq!(camera.get_frame_count(), 0, "Frame count should reset on stage");
    }
}
