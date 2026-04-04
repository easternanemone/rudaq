//! Workflow coordinators that compose multiple device capabilities.
//!
//! Coordinators implement [`CompositeCapability`] to orchestrate multi-device
//! operations. The most common pattern is the camera acquisition workflow:
//! position a stage, trigger the camera, and (optionally) read back results.
//!
//! # Example
//!
//! ```rust,ignore
//! use experiment::coordinator::AcquisitionCoordinator;
//!
//! let coord = AcquisitionCoordinator::new("stage_x", "camera", "camera");
//!
//! // Verify all required devices are available
//! coord.validate(&registry)?;
//!
//! // Execute a single acquisition at position 10.5 mm
//! coord.acquire_at(&registry, 10.5).await?;
//!
//! // Or scan across positions
//! for pos in [0.0, 1.0, 2.0, 3.0] {
//!     coord.acquire_at(&registry, pos).await?;
//! }
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::instrument;

use common::capabilities::{CapabilityProvider, CompositeCapability};
use common::driver::Capability;

/// Coordinates a camera acquisition workflow: move -> trigger -> read.
///
/// This is the most common 3-device pattern in DAQ: position a stage,
/// trigger camera acquisition, and collect the resulting frame.
///
/// The `stage_id` and `trigger_id` may refer to different physical devices
/// (e.g., an ELL14 stage and a PVCAM camera), or the same device if it
/// implements both capabilities. The `frame_producer_id` is often the same
/// as `trigger_id` but is tracked separately so callers can compose freely.
pub struct AcquisitionCoordinator {
    /// Stage device ID (must implement `Movable`)
    stage_id: String,
    /// Camera/trigger device ID (must implement `Triggerable`)
    trigger_id: String,
    /// Frame producer device ID (must implement `FrameProducer`)
    frame_producer_id: String,
}

impl AcquisitionCoordinator {
    /// Create a new acquisition coordinator.
    ///
    /// # Arguments
    ///
    /// * `stage_id` - Device that will be moved to each position
    /// * `trigger_id` - Device that will be armed and triggered
    /// * `frame_producer_id` - Device that produces frames (often same as `trigger_id`)
    pub fn new(
        stage_id: impl Into<String>,
        trigger_id: impl Into<String>,
        frame_producer_id: impl Into<String>,
    ) -> Self {
        Self {
            stage_id: stage_id.into(),
            trigger_id: trigger_id.into(),
            frame_producer_id: frame_producer_id.into(),
        }
    }

    /// Validate that all required devices and capabilities are present.
    ///
    /// Call this before starting a scan to get clear error messages about
    /// missing devices rather than failing mid-acquisition.
    pub fn validate(&self, provider: &dyn CapabilityProvider) -> Result<()> {
        provider
            .get_movable(&self.stage_id)
            .with_context(|| format!("stage '{}' not found or not Movable", self.stage_id))?;

        provider
            .get_triggerable(&self.trigger_id)
            .with_context(|| {
                format!(
                    "trigger device '{}' not found or not Triggerable",
                    self.trigger_id
                )
            })?;

        provider
            .get_frame_producer(&self.frame_producer_id)
            .with_context(|| {
                format!(
                    "frame producer '{}' not found or not FrameProducer",
                    self.frame_producer_id
                )
            })?;

        Ok(())
    }

    /// Execute a single acquisition at the given position.
    ///
    /// Sequence: move stage -> wait settled -> arm camera -> trigger.
    ///
    /// This does NOT read frames back; callers should register a
    /// `FrameObserver` or tap the ring buffer independently.
    #[instrument(skip(self, provider), fields(stage = %self.stage_id, trigger = %self.trigger_id))]
    pub async fn acquire_at(&self, provider: &dyn CapabilityProvider, position: f64) -> Result<()> {
        let movable = provider
            .get_movable(&self.stage_id)
            .with_context(|| format!("stage '{}' not found or not Movable", self.stage_id))?;

        let triggerable = provider
            .get_triggerable(&self.trigger_id)
            .with_context(|| {
                format!(
                    "trigger device '{}' not found or not Triggerable",
                    self.trigger_id
                )
            })?;

        // 1. Move stage to target position
        movable
            .move_abs(position)
            .await
            .context("failed to move stage")?;

        // 2. Wait for stage to settle before triggering
        movable
            .wait_settled()
            .await
            .context("stage failed to settle")?;

        // 3. Arm and trigger the camera
        triggerable
            .arm()
            .await
            .context("failed to arm trigger device")?;

        triggerable
            .trigger()
            .await
            .context("failed to trigger acquisition")?;

        Ok(())
    }

    /// Execute acquisitions at multiple positions in sequence.
    ///
    /// This is a convenience wrapper over [`acquire_at`](Self::acquire_at).
    /// For more sophisticated scan patterns (snake, grid, adaptive), use
    /// the `Plan` trait and `RunEngine` instead.
    #[instrument(skip(self, provider, positions), fields(stage = %self.stage_id, n_positions))]
    pub async fn acquire_scan(
        &self,
        provider: &dyn CapabilityProvider,
        positions: &[f64],
    ) -> Result<()> {
        tracing::Span::current().record("n_positions", positions.len());

        self.validate(provider)
            .context("scan pre-validation failed")?;

        for (i, &pos) in positions.iter().enumerate() {
            self.acquire_at(provider, pos)
                .await
                .with_context(|| format!("acquisition failed at step {i}, position {pos}"))?;
        }

        Ok(())
    }

    /// Get the stage device ID.
    pub fn stage_id(&self) -> &str {
        &self.stage_id
    }

    /// Get the trigger device ID.
    pub fn trigger_id(&self) -> &str {
        &self.trigger_id
    }

    /// Get the frame producer device ID.
    pub fn frame_producer_id(&self) -> &str {
        &self.frame_producer_id
    }
}

#[async_trait]
impl CompositeCapability for AcquisitionCoordinator {
    fn name(&self) -> &str {
        "acquisition_coordinator"
    }

    fn required_capabilities(&self) -> Vec<(String, Capability)> {
        vec![
            (self.stage_id.clone(), Capability::Movable),
            (self.trigger_id.clone(), Capability::Triggerable),
            (self.frame_producer_id.clone(), Capability::FrameProducer),
        ]
    }

    /// Default execution: arm and trigger at the current position (no move).
    async fn execute(&self, provider: &dyn CapabilityProvider) -> Result<()> {
        let triggerable = provider
            .get_triggerable(&self.trigger_id)
            .with_context(|| {
                format!(
                    "trigger device '{}' not found or not Triggerable",
                    self.trigger_id
                )
            })?;

        triggerable
            .arm()
            .await
            .context("failed to arm trigger device")?;

        triggerable
            .trigger()
            .await
            .context("failed to trigger acquisition")?;

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use common::capabilities::{
        EmissionControl, ExposureControl, FrameProducer, Movable, Readable, Settable,
        ShutterControl, Triggerable, WavelengthTunable,
    };

    // ---- Mock Devices ----

    struct MockStage {
        position: std::sync::Mutex<f64>,
        move_count: AtomicU64,
    }

    impl MockStage {
        fn new(initial_pos: f64) -> Self {
            Self {
                position: std::sync::Mutex::new(initial_pos),
                move_count: AtomicU64::new(0),
            }
        }
    }

    #[async_trait]
    impl Movable for MockStage {
        async fn move_abs(&self, position: f64) -> Result<()> {
            *self.position.lock().expect("lock poisoned") = position;
            self.move_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn move_rel(&self, distance: f64) -> Result<()> {
            *self.position.lock().expect("lock poisoned") += distance;
            self.move_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn position(&self) -> Result<f64> {
            Ok(*self.position.lock().expect("lock poisoned"))
        }

        async fn wait_settled(&self) -> Result<()> {
            Ok(())
        }
    }

    struct MockCamera {
        armed: AtomicBool,
        trigger_count: AtomicU64,
    }

    impl MockCamera {
        fn new() -> Self {
            Self {
                armed: AtomicBool::new(false),
                trigger_count: AtomicU64::new(0),
            }
        }
    }

    #[async_trait]
    impl Triggerable for MockCamera {
        async fn arm(&self) -> Result<()> {
            self.armed.store(true, Ordering::Relaxed);
            Ok(())
        }

        async fn trigger(&self) -> Result<()> {
            if !self.armed.load(Ordering::Relaxed) {
                anyhow::bail!("camera not armed");
            }
            self.trigger_count.fetch_add(1, Ordering::Relaxed);
            self.armed.store(false, Ordering::Relaxed);
            Ok(())
        }

        async fn is_armed(&self) -> Result<bool> {
            Ok(self.armed.load(Ordering::Relaxed))
        }
    }

    #[async_trait]
    impl FrameProducer for MockCamera {
        async fn start_stream(&self) -> Result<()> {
            Ok(())
        }
        async fn stop_stream(&self) -> Result<()> {
            Ok(())
        }
        fn resolution(&self) -> (u32, u32) {
            (64, 64)
        }
    }

    // ---- Mock CapabilityProvider ----

    struct MockProvider {
        stage: Arc<MockStage>,
        camera: Arc<MockCamera>,
    }

    impl MockProvider {
        fn new() -> Self {
            Self {
                stage: Arc::new(MockStage::new(0.0)),
                camera: Arc::new(MockCamera::new()),
            }
        }
    }

    impl CapabilityProvider for MockProvider {
        fn get_movable(&self, id: &str) -> Option<Arc<dyn Movable>> {
            if id == "stage" {
                Some(self.stage.clone())
            } else {
                None
            }
        }

        fn get_readable(&self, _id: &str) -> Option<Arc<dyn Readable>> {
            None
        }

        fn get_triggerable(&self, id: &str) -> Option<Arc<dyn Triggerable>> {
            if id == "camera" {
                Some(self.camera.clone())
            } else {
                None
            }
        }

        fn get_frame_producer(&self, id: &str) -> Option<Arc<dyn FrameProducer>> {
            if id == "camera" {
                Some(self.camera.clone())
            } else {
                None
            }
        }

        fn get_exposure_control(&self, _id: &str) -> Option<Arc<dyn ExposureControl>> {
            None
        }

        fn get_shutter_control(&self, _id: &str) -> Option<Arc<dyn ShutterControl>> {
            None
        }

        fn get_wavelength_tunable(&self, _id: &str) -> Option<Arc<dyn WavelengthTunable>> {
            None
        }

        fn get_emission_control(&self, _id: &str) -> Option<Arc<dyn EmissionControl>> {
            None
        }

        fn get_settable(&self, _id: &str) -> Option<Arc<dyn Settable>> {
            None
        }

        fn get_counter_configurable(
            &self,
            _id: &str,
        ) -> Option<Arc<dyn common::capabilities::CounterConfigurable>> {
            None
        }

        fn get_range_introspectable(
            &self,
            _id: &str,
        ) -> Option<Arc<dyn common::capabilities::RangeIntrospectable>> {
            None
        }

        fn get_device_introspection(
            &self,
            _id: &str,
        ) -> Option<Arc<dyn common::capabilities::DeviceIntrospection>> {
            None
        }

        fn get_readable_with_metadata(
            &self,
            _id: &str,
        ) -> Option<Arc<dyn common::capabilities::ReadableWithMetadata>> {
            None
        }

        fn get_spectrum_readable(
            &self,
            _id: &str,
        ) -> Option<Arc<dyn common::capabilities::SpectrumReadable>> {
            None
        }
    }

    // ---- Tests ----

    #[tokio::test]
    async fn test_acquire_at_moves_then_triggers() {
        let provider = MockProvider::new();
        let coord = AcquisitionCoordinator::new("stage", "camera", "camera");

        coord.acquire_at(&provider, 42.0).await.unwrap();

        // Stage should have moved to 42.0
        let pos = provider.stage.position().await.unwrap();
        assert!((pos - 42.0).abs() < f64::EPSILON);
        assert_eq!(provider.stage.move_count.load(Ordering::Relaxed), 1);

        // Camera should have been triggered once
        assert_eq!(provider.camera.trigger_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_acquire_scan_visits_all_positions() {
        let provider = MockProvider::new();
        let coord = AcquisitionCoordinator::new("stage", "camera", "camera");

        let positions = [0.0, 5.0, 10.0, 15.0, 20.0];
        coord.acquire_scan(&provider, &positions).await.unwrap();

        // Should have moved 5 times
        assert_eq!(provider.stage.move_count.load(Ordering::Relaxed), 5);
        // Should have triggered 5 times
        assert_eq!(provider.camera.trigger_count.load(Ordering::Relaxed), 5);
        // Final position should be 20.0
        let pos = provider.stage.position().await.unwrap();
        assert!((pos - 20.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_validate_detects_missing_stage() {
        let provider = MockProvider::new();
        let coord = AcquisitionCoordinator::new("nonexistent_stage", "camera", "camera");

        let err = coord.validate(&provider).unwrap_err();
        assert!(
            err.to_string().contains("nonexistent_stage"),
            "error should mention the missing device: {err}"
        );
    }

    #[tokio::test]
    async fn test_validate_detects_missing_trigger() {
        let provider = MockProvider::new();
        let coord = AcquisitionCoordinator::new("stage", "nonexistent_cam", "camera");

        let err = coord.validate(&provider).unwrap_err();
        assert!(
            err.to_string().contains("nonexistent_cam"),
            "error should mention the missing device: {err}"
        );
    }

    #[tokio::test]
    async fn test_validate_detects_missing_frame_producer() {
        let provider = MockProvider::new();
        let coord = AcquisitionCoordinator::new("stage", "camera", "nonexistent_fp");

        let err = coord.validate(&provider).unwrap_err();
        assert!(
            err.to_string().contains("nonexistent_fp"),
            "error should mention the missing device: {err}"
        );
    }

    #[tokio::test]
    async fn test_composite_capability_impl() {
        let coord = AcquisitionCoordinator::new("stage", "camera", "camera");

        assert_eq!(coord.name(), "acquisition_coordinator");

        let required = coord.required_capabilities();
        assert_eq!(required.len(), 3);
        assert_eq!(required[0], ("stage".to_string(), Capability::Movable));
        assert_eq!(required[1], ("camera".to_string(), Capability::Triggerable));
        assert_eq!(
            required[2],
            ("camera".to_string(), Capability::FrameProducer)
        );
    }

    #[tokio::test]
    async fn test_execute_triggers_without_moving() {
        let provider = MockProvider::new();
        // Pre-position stage at 99.0
        provider.stage.move_abs(99.0).await.unwrap();
        provider.stage.move_count.store(0, Ordering::Relaxed);

        let coord = AcquisitionCoordinator::new("stage", "camera", "camera");
        coord.execute(&provider).await.unwrap();

        // execute() should NOT move the stage
        assert_eq!(provider.stage.move_count.load(Ordering::Relaxed), 0);
        // But should trigger the camera
        assert_eq!(provider.camera.trigger_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_accessor_methods() {
        let coord = AcquisitionCoordinator::new("s1", "t1", "fp1");
        assert_eq!(coord.stage_id(), "s1");
        assert_eq!(coord.trigger_id(), "t1");
        assert_eq!(coord.frame_producer_id(), "fp1");
    }

    #[tokio::test]
    async fn test_acquire_scan_empty_positions() {
        let provider = MockProvider::new();
        let coord = AcquisitionCoordinator::new("stage", "camera", "camera");

        // Empty positions should succeed with zero moves and zero triggers
        coord.acquire_scan(&provider, &[]).await.unwrap();

        assert_eq!(provider.stage.move_count.load(Ordering::Relaxed), 0);
        assert_eq!(provider.camera.trigger_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_acquire_at_missing_stage_returns_error() {
        let provider = MockProvider::new();
        // Stage ID does not exist in provider
        let coord = AcquisitionCoordinator::new("nonexistent", "camera", "camera");

        let err = coord.acquire_at(&provider, 1.0).await.unwrap_err();
        assert!(
            err.to_string().contains("nonexistent"),
            "error should mention missing stage device: {err}"
        );
    }

    #[tokio::test]
    async fn test_acquire_at_missing_trigger_returns_error() {
        let provider = MockProvider::new();
        // Trigger ID does not exist in provider
        let coord = AcquisitionCoordinator::new("stage", "nonexistent", "camera");

        let err = coord.acquire_at(&provider, 1.0).await.unwrap_err();
        // Stage move succeeds but trigger lookup fails
        assert!(
            err.to_string().contains("nonexistent"),
            "error should mention missing trigger device: {err}"
        );
    }

    #[tokio::test]
    async fn test_acquire_scan_fails_validation_missing_device() {
        let provider = MockProvider::new();
        let coord = AcquisitionCoordinator::new("nonexistent", "camera", "camera");

        let err = coord
            .acquire_scan(&provider, &[1.0, 2.0])
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("pre-validation"),
            "scan should fail at pre-validation: {err}"
        );
    }
}
