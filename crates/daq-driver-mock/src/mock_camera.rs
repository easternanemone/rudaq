//! Mock camera implementation with trigger and streaming support.

use crate::pattern::generate_test_pattern;
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
// MockCamera - Simulated Camera
// =============================================================================

/// Mock camera with trigger and streaming support.
///
/// Simulates a camera with:
/// - Configurable resolution
/// - 33ms frame readout (~30fps)
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
    /// Broadcast channel for frame streaming (deprecated)
    frame_tx: tokio::sync::broadcast::Sender<Arc<Frame>>,
    /// Reliable channel for lossless data transmission (optional)
    reliable_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Arc<Frame>>>>>,
    #[allow(dead_code)]
    streaming_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    streaming_flag: Arc<AtomicBool>,
    armed_flag: Arc<AtomicBool>,
    staged_flag: Arc<AtomicBool>,
    /// Primary output channel for pooled frames
    primary_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<LoanedFrame>>>>,
    /// Frame pool for zero-allocation LoanedFrame delivery
    frame_pool: Arc<Mutex<Option<Arc<Pool<FrameData>>>>>,
    /// Registered frame observers
    #[allow(clippy::type_complexity)]
    observers: Arc<RwLock<Vec<(u64, Box<dyn FrameObserver>)>>>,
    /// Counter for generating unique observer IDs
    next_observer_id: AtomicU64,
}

impl MockCamera {
    /// Create new mock camera with specified resolution.
    ///
    /// # Arguments
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_config(MockCameraConfig {
            width,
            height,
            ..Default::default()
        })
    }

    /// Create mock camera with full configuration.
    pub fn with_config(config: MockCameraConfig) -> Self {
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

        // Streaming parameter
        let mut streaming = Parameter::new("streaming", false).with_description("Streaming");
        let observers_for_streaming = observers.clone(); // Clone the observers Arc for capture
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

            streaming.connect_to_hardware_write(move |enable| {
                let streaming_flag = streaming_flag_write.clone();
                let frame_tx = frame_tx_write.clone();
                let frame_count = frame_count_write.clone();
                let streaming_task = streaming_task_write.clone();
                let reliable_tx = reliable_tx_write.clone();
                let primary_tx = primary_tx_write.clone();
                let frame_pool = frame_pool_write.clone();
                let observers_for_task = observers_write.clone();

                Box::pin(async move {
                    if enable {
                        if streaming_flag.swap(true, Ordering::SeqCst) {
                            return Ok(());
                        }

                        let mut handle_guard = streaming_task.lock().await;
                        let flag_for_task = streaming_flag.clone();
                        let tx = frame_tx.clone();
                        let reliable_tx_for_task = reliable_tx.lock().await.clone();
                        let primary_tx_for_task = primary_tx.lock().await.clone();
                        let frame_pool_for_task = frame_pool.lock().await.clone();
                        let res = resolution;
                        let count = frame_count.clone();

                        let observers_for_spawn = observers_for_task.clone();
                        let handle = tokio::spawn(async move {
                            while flag_for_task.load(Ordering::SeqCst) {
                                let frame_num = count.fetch_add(1, Ordering::SeqCst) + 1;
                                let (w, h) = res;
                                let buffer = generate_test_pattern(w, h, frame_num);

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

                                // Notify registered observers with FrameView (bd-flaky-test-fix)
                                // Only allocate pixel_bytes if there are observers registered
                                {
                                    let observers_guard = observers_for_spawn.read().await;
                                    if !observers_guard.is_empty() {
                                        // Convert u16 buffer to bytes for FrameView
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
                                        .with_exposure(33.0);
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
        }
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

        // Wait for some frames (MockCamera streams at ~30fps, so 150ms should give ~4-5 frames)
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
        // (allowing for 1 frame that might have been in-flight)
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
}
