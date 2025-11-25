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

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use crate::hardware::capabilities::{
    ExposureControl, FrameProducer, Movable, Readable, Triggerable,
};

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
pub struct MockStage {
    position: Arc<RwLock<f64>>,
    speed_mm_per_sec: f64,
}

impl MockStage {
    /// Create new mock stage at position 0.0mm
    pub fn new() -> Self {
        Self {
            position: Arc::new(RwLock::new(0.0)),
            speed_mm_per_sec: 10.0, // 10mm/sec
        }
    }

    /// Create new mock stage at specified initial position
    ///
    /// # Arguments
    /// * `initial_position` - Starting position in mm
    pub fn with_position(initial_position: f64) -> Self {
        Self {
            position: Arc::new(RwLock::new(initial_position)),
            speed_mm_per_sec: 10.0, // 10mm/sec
        }
    }

    /// Create mock stage with custom speed
    ///
    /// # Arguments
    /// * `speed_mm_per_sec` - Motion speed in mm/sec
    pub fn with_speed(speed_mm_per_sec: f64) -> Self {
        Self {
            position: Arc::new(RwLock::new(0.0)),
            speed_mm_per_sec,
        }
    }
}

impl Default for MockStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Movable for MockStage {
    async fn move_abs(&self, target: f64) -> Result<()> {
        let current = *self.position.read().await;
        let distance = (target - current).abs();
        let delay_ms = (distance / self.speed_mm_per_sec * 1000.0) as u64;

        println!(
            "MockStage: Moving from {:.2}mm to {:.2}mm ({}ms)",
            current, target, delay_ms
        );

        // CRITICAL: Use tokio::time::sleep, NOT std::thread::sleep
        sleep(Duration::from_millis(delay_ms)).await;

        *self.position.write().await = target;
        println!("MockStage: Reached {:.2}mm", target);
        Ok(())
    }

    async fn move_rel(&self, distance: f64) -> Result<()> {
        let current = *self.position.read().await;
        self.move_abs(current + distance).await
    }

    async fn position(&self) -> Result<f64> {
        Ok(*self.position.read().await)
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
/// Simulates a triggered camera with:
/// - Configurable resolution
/// - 33ms frame readout time (30fps)
/// - Arm/trigger lifecycle
/// - Frame counting for diagnostics
///
/// # Example
///
/// ```rust,ignore
/// let camera = MockCamera::new(1920, 1080);
/// camera.arm().await?;
/// camera.trigger().await?; // Takes ~33ms
/// assert_eq!(camera.resolution(), (1920, 1080));
/// ```
pub struct MockCamera {
    resolution: (u32, u32),
    frame_count: Arc<RwLock<u32>>,
    armed: Arc<RwLock<bool>>,
    streaming: Arc<RwLock<bool>>,
    exposure_s: Arc<RwLock<f64>>,
}

impl MockCamera {
    /// Create new mock camera with specified resolution
    ///
    /// # Arguments
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            resolution: (width, height),
            frame_count: Arc::new(RwLock::new(0)),
            armed: Arc::new(RwLock::new(false)),
            streaming: Arc::new(RwLock::new(false)),
            exposure_s: Arc::new(RwLock::new(0.033)),
        }
    }

    /// Get total number of frames captured
    pub async fn frame_count(&self) -> u32 {
        *self.frame_count.read().await
    }

    /// Check if camera is currently armed
    pub async fn is_armed(&self) -> bool {
        *self.armed.read().await
    }

    /// Check if camera is streaming
    pub async fn is_streaming(&self) -> bool {
        *self.streaming.read().await
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
        let already_armed = *self.armed.read().await;
        if already_armed {
            println!("MockCamera: Already armed (re-arming)");
        } else {
            println!("MockCamera: Armed");
        }
        *self.armed.write().await = true;
        Ok(())
    }

    async fn trigger(&self) -> Result<()> {
        // Check if armed
        if !*self.armed.read().await {
            anyhow::bail!("MockCamera: Cannot trigger - not armed");
        }

        let mut count = self.frame_count.write().await;
        *count += 1;
        println!("MockCamera: Triggered frame #{}", *count);

        // Simulate 30fps frame readout time
        sleep(Duration::from_millis(33)).await;

        println!("MockCamera: Frame #{} readout complete", *count);
        Ok(())
    }

    async fn is_armed(&self) -> Result<bool> {
        Ok(*self.armed.read().await)
    }
}

#[async_trait]
impl ExposureControl for MockCamera {
    async fn set_exposure(&self, seconds: f64) -> Result<()> {
        if seconds <= 0.0 {
            return Err(anyhow!("MockCamera: Exposure must be positive"));
        }
        *self.exposure_s.write().await = seconds;
        Ok(())
    }

    async fn get_exposure(&self) -> Result<f64> {
        Ok(*self.exposure_s.read().await)
    }
}

#[async_trait]
impl FrameProducer for MockCamera {
    async fn start_stream(&self) -> Result<()> {
        let already_streaming = *self.streaming.read().await;
        if already_streaming {
            anyhow::bail!("MockCamera: Already streaming");
        }

        println!("MockCamera: Stream started");
        *self.streaming.write().await = true;
        Ok(())
    }

    async fn stop_stream(&self) -> Result<()> {
        let was_streaming = *self.streaming.read().await;
        if !was_streaming {
            println!("MockCamera: Stream already stopped");
        } else {
            println!("MockCamera: Stream stopped");
        }

        *self.streaming.write().await = false;
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        self.resolution
    }

    async fn is_streaming(&self) -> Result<bool> {
        Ok(*self.streaming.read().await)
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
    base_power: Arc<RwLock<f64>>,
}

impl MockPowerMeter {
    /// Create new mock power meter with specified base power (Watts)
    ///
    /// # Arguments
    /// * `base_power` - Base power reading in Watts
    pub fn new(base_power: f64) -> Self {
        Self {
            base_power: Arc::new(RwLock::new(base_power)),
        }
    }

    /// Set the base power reading
    pub async fn set_base_power(&self, power: f64) {
        *self.base_power.write().await = power;
    }

    /// Get the current base power setting
    pub async fn get_base_power(&self) -> f64 {
        *self.base_power.read().await
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
        let base = *self.base_power.read().await;

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
        println!("MockPowerMeter: Read {:.6}W", reading);
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
    async fn test_mock_camera_trigger() {
        let camera = MockCamera::new(1920, 1080);

        // Should fail if not armed
        let result = camera.trigger().await;
        assert!(result.is_err());

        // Arm and trigger
        camera.arm().await.unwrap();
        assert!(camera.is_armed().await);

        camera.trigger().await.unwrap();
        assert_eq!(camera.frame_count().await, 1);

        // Trigger again (should still work, camera stays armed)
        camera.trigger().await.unwrap();
        assert_eq!(camera.frame_count().await, 2);
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
        meter.set_base_power(5.0).await;
        assert_eq!(meter.get_base_power().await, 5.0);

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
        assert_eq!(meter.get_base_power().await, 1.0);
    }
}
