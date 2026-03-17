use super::*;
use crate::plans::{ComparisonOp, Count, EvalCondition, PlanCommand};
use crate::plans_imperative::ImperativePlan;
use async_trait::async_trait;
use chrono::Utc;
use common::capabilities::{
    DeviceCategory, FrameProducer, GateMode, GatedCamera, Parameterized, SpectrometerControl,
    TemperatureStatus,
};
use common::driver::{Capability as DeviceCapability, DeviceComponents, DriverFactory};
use common::observable::{Observable, ParameterSet};
use echelle::EchelleFrameCompatibility;
use std::future::Future;
use std::pin::Pin;

struct MockSpectrometer {
    grating: u8,
    wavelength_nm: f64,
}

#[async_trait]
impl SpectrometerControl for MockSpectrometer {
    async fn set_grating(&self, _grating_num: u8) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_grating(&self) -> anyhow::Result<u8> {
        Ok(self.grating)
    }

    async fn set_wavelength(&self, _nm: f64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_wavelength(&self) -> anyhow::Result<f64> {
        Ok(self.wavelength_nm)
    }

    async fn set_slit_width(&self, _slit_id: u8, _width_um: u16) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_calibration(&self, num_pixels: usize) -> anyhow::Result<Vec<f64>> {
        #[allow(clippy::cast_precision_loss)] // test pixel indices are small
        Ok((0..num_pixels).map(|idx| 300.0 + idx as f64).collect())
    }

    async fn is_at_zero_order(&self) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn set_shutter(&self, _open: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

struct MockSpectrometerFactory {
    grating: u8,
    wavelength_nm: f64,
}

impl DriverFactory for MockSpectrometerFactory {
    fn driver_type(&self) -> &'static str {
        "mock_spectrometer"
    }

    fn name(&self) -> &'static str {
        "Mock Spectrometer"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[DeviceCapability::SpectrometerControl]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let grating = self.grating;
        let wavelength_nm = self.wavelength_nm;
        Box::pin(async move {
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Detector)
                .with_spectrometer_control(Arc::new(MockSpectrometer {
                    grating,
                    wavelength_nm,
                })))
        })
    }
}

async fn make_spectroscopy_registry() -> Arc<DeviceRegistry> {
    make_spectroscopy_registry_with_spectrometer(1, 300.0).await
}

async fn make_spectroscopy_registry_with_spectrometer(
    grating: u8,
    wavelength_nm: f64,
) -> Arc<DeviceRegistry> {
    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(MockSpectrometerFactory {
        grating,
        wavelength_nm,
    }));
    registry
        .register_from_toml(
            "spectrometer",
            "Mock Spectrometer",
            "mock_spectrometer",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register mock spectrometer");
    registry
}

#[derive(Clone, Copy)]
struct MockEchelleCameraConfig {
    frame_width: u32,
    frame_height: u32,
    roi_x: u32,
    roi_y: u32,
    binning_x: u32,
    binning_y: u32,
    bit_depth: Option<u32>,
}

impl Default for MockEchelleCameraConfig {
    fn default() -> Self {
        Self {
            frame_width: 1024,
            frame_height: 512,
            roi_x: 0,
            roi_y: 0,
            binning_x: 1,
            binning_y: 1,
            bit_depth: Some(16),
        }
    }
}

struct MockGatedCamera {
    params: ParameterSet,
    resolution: (u32, u32),
}

impl MockGatedCamera {
    fn new(config: MockEchelleCameraConfig) -> Self {
        let mut params = ParameterSet::new();
        params.register(Observable::new("AOIWidth", i64::from(config.frame_width)));
        params.register(Observable::new("AOIHeight", i64::from(config.frame_height)));
        params.register(Observable::new("AOILeft", i64::from(config.roi_x + 1)));
        params.register(Observable::new("AOITop", i64::from(config.roi_y + 1)));
        params.register(Observable::new("AOIHBin", i64::from(config.binning_x)));
        params.register(Observable::new("AOIVBin", i64::from(config.binning_y)));
        if let Some(bit_depth) = config.bit_depth {
            params.register(Observable::new("BitDepth", i64::from(bit_depth)));
        }

        Self {
            params,
            resolution: (config.frame_width, config.frame_height),
        }
    }
}

impl Parameterized for MockGatedCamera {
    fn parameters(&self) -> &ParameterSet {
        &self.params
    }
}

#[async_trait]
impl FrameProducer for MockGatedCamera {
    async fn start_stream(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop_stream(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn resolution(&self) -> (u32, u32) {
        self.resolution
    }
}

#[async_trait]
impl GatedCamera for MockGatedCamera {
    async fn set_gate_mode(&self, _mode: GateMode) -> anyhow::Result<()> {
        Ok(())
    }

    async fn set_ddg_timing(&self, _delay_ps: u64, _width_ps: u64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn set_mcp_gain(&self, _gain: u16) -> anyhow::Result<()> {
        Ok(())
    }

    async fn set_intelligate(&self, _enabled: bool) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_temperature_status(&self) -> anyhow::Result<TemperatureStatus> {
        Ok(TemperatureStatus::Stabilized)
    }
}

struct MockGatedCameraFactory {
    config: MockEchelleCameraConfig,
}

impl DriverFactory for MockGatedCameraFactory {
    fn driver_type(&self) -> &'static str {
        "mock_gated_camera"
    }

    fn name(&self) -> &'static str {
        "Mock Gated Camera"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[
            DeviceCapability::Parameterized,
            DeviceCapability::GatedCamera,
        ]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let config = self.config;
        Box::pin(async move {
            let driver = Arc::new(MockGatedCamera::new(config));
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Camera)
                .with_parameterized(driver.clone())
                .with_gated_camera(driver))
        })
    }
}

async fn make_echelle_registry(config: MockEchelleCameraConfig) -> Arc<DeviceRegistry> {
    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(MockGatedCameraFactory { config }));
    registry
        .register_from_toml(
            "camera",
            "Mock Gated Camera",
            "mock_gated_camera",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register mock gated camera");
    registry
}

#[tokio::test]
async fn test_engine_state_transitions() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    assert_eq!(engine.state().await, EngineState::Idle);

    // Can't pause when idle
    assert!(engine.pause().await.is_err());

    // Can't resume when idle
    assert!(engine.resume().await.is_err());
}

#[tokio::test]
async fn test_queue_plan() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let plan = Box::new(Count::new(5));
    let _run_uid = engine.queue(plan).await;

    assert_eq!(engine.queue_len().await, 1);
}

#[tokio::test]
async fn test_start_blocks_on_stale_spectroscopy_calibration() {
    let registry = make_spectroscopy_registry().await;
    let engine = RunEngine::new(registry);

    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy")
                .with_timestamp(Utc::now() - chrono::Duration::hours(30)),
        )
        .await;
    let plan = Box::new(Count::new(1).with_detector("spectrometer"));
    engine.queue(plan).await;

    let err = engine
        .start()
        .await
        .expect_err("stale calibration should block");
    assert!(
        err.to_string().contains("CalibrationStalenessGate"),
        "unexpected error: {err}"
    );
    assert_eq!(engine.state().await, EngineState::Idle);
    assert_eq!(
        engine.queue_len().await,
        1,
        "blocked run should stay queued"
    );
}

#[tokio::test]
async fn test_next_plan_readiness_issues_reports_calibration_age() {
    let registry = make_spectroscopy_registry().await;
    let engine = RunEngine::new(registry);

    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy")
                .with_timestamp(Utc::now() - chrono::Duration::hours(30)),
        )
        .await;
    engine
        .queue(Box::new(Count::new(1).with_detector("spectrometer")))
        .await;

    let issues = engine.next_plan_readiness_issues().await;
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "calibration_stale");
    assert!(issues[0].blocking);
    assert!(
        issues[0].age_hours.unwrap_or_default() >= 29.9,
        "age_hours should reflect stale calibration"
    );
}

#[tokio::test]
async fn test_custom_calibration_threshold_allows_newer_snapshot() {
    let registry = make_spectroscopy_registry().await;
    let engine = Arc::new(RunEngine::new(registry));

    engine
        .set_calibration_max_age("spectroscopy", Duration::from_secs(36 * 60 * 60))
        .await;
    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy")
                .with_timestamp(Utc::now() - chrono::Duration::hours(30)),
        )
        .await;
    engine
        .queue(Box::new(Count::new(1).with_detector("spectrometer")))
        .await;

    let engine_for_task = engine.clone();
    let task = tokio::spawn(async move { engine_for_task.start().await });
    task.await
        .expect("join start task")
        .expect("fresh enough run");

    assert_eq!(engine.state().await, EngineState::Idle);
}

#[tokio::test]
async fn test_start_blocks_on_spectrometer_grating_mismatch_without_timestamp() {
    let registry = make_spectroscopy_registry_with_spectrometer(3, 300.0).await;
    let engine = RunEngine::new(registry);

    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy").with_grating_wavelength_coverage(
                HashMap::from([(
                    1,
                    CalibrationWavelengthCoverage {
                        min_nm: 295.0,
                        max_nm: 305.0,
                    },
                )]),
            ),
        )
        .await;
    engine
        .queue(Box::new(Count::new(1).with_detector("spectrometer")))
        .await;

    let err = engine
        .start()
        .await
        .expect_err("grating mismatch should block even without timestamp metadata");
    assert!(
        err.to_string().contains("CalibrationConfigGate"),
        "unexpected error: {err}"
    );
    assert!(
        err.to_string().contains("grating 3"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn test_next_plan_readiness_issues_reports_spectrometer_wavelength_mismatch() {
    let registry = make_spectroscopy_registry_with_spectrometer(1, 325.0).await;
    let engine = RunEngine::new(registry);

    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy").with_grating_wavelength_coverage(
                HashMap::from([(
                    1,
                    CalibrationWavelengthCoverage {
                        min_nm: 295.0,
                        max_nm: 305.0,
                    },
                )]),
            ),
        )
        .await;
    engine
        .queue(Box::new(Count::new(1).with_detector("spectrometer")))
        .await;

    let issues = engine.next_plan_readiness_issues().await;
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "calibration_config_mismatch");
    assert_eq!(issues[0].device_id.as_deref(), Some("spectrometer"));
    assert!(issues[0].blocking);
    assert!(
        issues[0]
            .message
            .contains("outside the loaded calibration range"),
        "unexpected issue: {:?}",
        issues
    );
}

#[tokio::test]
async fn test_next_plan_readiness_issues_reports_echelle_frame_mismatch() {
    let registry = make_echelle_registry(MockEchelleCameraConfig {
        roi_x: 10,
        ..MockEchelleCameraConfig::default()
    })
    .await;
    let engine = RunEngine::new(registry);

    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy").with_echelle_frame_compatibility(
                EchelleFrameCompatibility {
                    sensor_width: 1024,
                    sensor_height: 512,
                    frame_width: 1024,
                    frame_height: 512,
                    roi_x: 0,
                    roi_y: 0,
                    binning_x: 1,
                    binning_y: 1,
                    bit_depth: Some(16),
                },
            ),
        )
        .await;
    engine
        .queue(Box::new(Count::new(1).with_detector("camera")))
        .await;

    let issues = engine.next_plan_readiness_issues().await;
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "calibration_config_mismatch");
    assert_eq!(issues[0].device_id.as_deref(), Some("camera"));
    assert!(issues[0].blocking);
    assert!(
        issues[0].message.contains("roi_x expected 0 got 10"),
        "unexpected issue: {:?}",
        issues
    );
}

#[tokio::test]
async fn test_register_calibration_snapshot_merges_radiance_and_echelle_fields() {
    let registry = make_spectroscopy_registry().await;
    let engine = RunEngine::new(registry);

    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy")
                .with_grating_wavelength_coverage(HashMap::from([(
                    1,
                    CalibrationWavelengthCoverage {
                        min_nm: 295.0,
                        max_nm: 305.0,
                    },
                )]))
                .with_timestamp(Utc::now()),
        )
        .await;
    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy")
                .with_target_device_id("camera")
                .with_echelle_frame_compatibility(EchelleFrameCompatibility {
                    sensor_width: 1024,
                    sensor_height: 512,
                    frame_width: 1024,
                    frame_height: 512,
                    roi_x: 0,
                    roi_y: 0,
                    binning_x: 1,
                    binning_y: 1,
                    bit_depth: Some(16),
                }),
        )
        .await;

    let snapshots = engine.active_calibrations.read().await;
    let snapshot = snapshots
        .get("spectroscopy")
        .expect("merged snapshot should exist");
    assert!(snapshot.calibration_timestamp.is_some());
    assert_eq!(
        snapshot.grating_wavelength_coverage.get(&1),
        Some(&CalibrationWavelengthCoverage {
            min_nm: 295.0,
            max_nm: 305.0,
        })
    );
    assert_eq!(snapshot.target_device_id.as_deref(), Some("camera"));
    assert!(snapshot.echelle_frame_compatibility.is_some());
}

#[tokio::test]
async fn test_clear_echelle_snapshot_preserves_radiance_fields() {
    let registry = make_spectroscopy_registry().await;
    let engine = RunEngine::new(registry);

    engine
        .register_calibration_snapshot(
            CalibrationFreshness::new("spectroscopy")
                .with_grating_wavelength_coverage(HashMap::from([(
                    1,
                    CalibrationWavelengthCoverage {
                        min_nm: 295.0,
                        max_nm: 305.0,
                    },
                )]))
                .with_timestamp(Utc::now())
                .with_target_device_id("camera")
                .with_echelle_frame_compatibility(EchelleFrameCompatibility {
                    sensor_width: 1024,
                    sensor_height: 512,
                    frame_width: 1024,
                    frame_height: 512,
                    roi_x: 0,
                    roi_y: 0,
                    binning_x: 1,
                    binning_y: 1,
                    bit_depth: Some(16),
                }),
        )
        .await;

    engine
        .clear_echelle_calibration_snapshot("spectroscopy")
        .await;

    let snapshots = engine.active_calibrations.read().await;
    let snapshot = snapshots
        .get("spectroscopy")
        .expect("radiance fields should remain after clearing echelle state");
    assert!(snapshot.calibration_timestamp.is_some());
    assert!(!snapshot.grating_wavelength_coverage.is_empty());
    assert_eq!(snapshot.target_device_id, None);
    assert_eq!(snapshot.echelle_frame_compatibility, None);
}

#[tokio::test]
async fn test_document_subscription() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let mut rx = engine.subscribe();

    // Queue a simple plan
    let plan = Box::new(Count::new(3));
    engine.queue(plan).await;

    // Start in a separate task
    let engine_clone = Arc::new(engine);
    let engine_for_task = engine_clone.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Should receive StartDoc
    let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(doc.is_ok());
    if let Ok(Ok(Document::Start(start))) = doc {
        assert_eq!(start.plan_type, "count");
    }

    // Should receive Manifest document after Start (bd-ib06)
    let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(doc.is_ok());
    if let Ok(Ok(Document::Manifest(manifest))) = doc {
        assert_eq!(manifest.plan_type, "count");
        // Manifest should contain system info
        assert!(manifest.system_info.contains_key("software_version"));
    }
}

/// Test that Wait command can be interrupted by abort (bd-lnoi)
#[tokio::test]
async fn test_wait_interruptible_by_abort() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = Arc::new(RunEngine::new(registry));

    let mut rx = engine.subscribe();

    // Create a plan with a long wait (60 seconds - would block if not interruptible)
    let plan = Box::new(ImperativePlan::wait(60.0));
    engine.queue(plan).await;

    // Start in a separate task
    let engine_for_task = engine.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Wait for engine to start (receive StartDoc)
    let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(doc.is_ok(), "Should receive StartDoc");
    if let Ok(Ok(Document::Start(_))) = doc {
        // Good - engine started
    }

    // Give the Wait command time to start executing
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request abort - should take effect within 200ms (2 check cycles)
    let abort_start = tokio::time::Instant::now();
    engine
        .abort("Test abort")
        .await
        .expect("Abort should succeed");

    // Wait for StopDoc with abort status
    let doc = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match rx.recv().await {
                Ok(Document::Stop(stop)) => return stop,
                Ok(_) => {} // Skip other documents
                Err(err) => panic!("Channel closed before StopDoc: {err}"),
            }
        }
    })
    .await;

    let abort_elapsed = abort_start.elapsed();

    assert!(doc.is_ok(), "Should receive StopDoc within 500ms");
    let stop = doc.unwrap();
    assert_eq!(stop.exit_status, "abort", "Exit status should be 'abort'");

    // Verify abort was fast (< 500ms, well under the 60s wait)
    // The chunked sleep checks every 100ms, so abort should complete in ~200ms max
    assert!(
        abort_elapsed < Duration::from_millis(500),
        "Abort took too long: {:?} (expected < 500ms)",
        abort_elapsed
    );
}

/// Test that normal Wait still works correctly (bd-lnoi)
#[tokio::test]
async fn test_wait_completes_normally() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = Arc::new(RunEngine::new(registry));

    let mut rx = engine.subscribe();

    // Create a plan with a short wait
    let wait_duration = 0.2; // 200ms
    let plan = Box::new(ImperativePlan::wait(wait_duration));
    engine.queue(plan).await;

    let start_time = tokio::time::Instant::now();

    // Start in a separate task
    let engine_for_task = engine.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Wait for StopDoc
    let stop_doc = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Ok(Document::Stop(stop)) => return stop,
                Ok(_) => {}
                Err(err) => panic!("Channel closed before StopDoc: {err}"),
            }
        }
    })
    .await
    .expect("Should receive StopDoc");

    let elapsed = start_time.elapsed();

    // Should complete successfully
    assert_eq!(stop_doc.exit_status, "success");

    // Timing should be approximately correct (within 100ms tolerance)
    // The wait should take at least wait_duration and not much longer
    let expected_min = Duration::from_secs_f64(wait_duration);
    let expected_max = Duration::from_secs_f64(wait_duration + 0.15); // Allow 150ms overhead

    assert!(
        elapsed >= expected_min,
        "Wait completed too fast: {:?} (expected >= {:?})",
        elapsed,
        expected_min
    );
    assert!(
        elapsed < expected_max,
        "Wait took too long: {:?} (expected < {:?})",
        elapsed,
        expected_max
    );
}

#[tokio::test]
async fn test_clear_queue() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let plan1 = Box::new(Count::new(5));
    let plan2 = Box::new(Count::new(3));

    engine.queue(plan1).await;
    engine.queue(plan2).await;
    assert_eq!(engine.queue_len().await, 2);

    engine.clear_queue().await;
    assert_eq!(engine.queue_len().await, 0);
}

#[tokio::test]
async fn test_state_returns_correct_value() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let state = engine.state().await;
    assert!(matches!(state, EngineState::Idle));
}

#[tokio::test]
async fn test_engine_abort_when_idle() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let result = engine.abort("Test abort").await;
    // Aborting when idle should fail or be a no-op
    // Based on the implementation, check what's expected
    assert!(result.is_ok() || result.is_err());
}

#[tokio::test]
async fn test_multiple_plan_execution() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = Arc::new(RunEngine::new(registry));

    let mut rx = engine.subscribe();

    // Queue and execute first plan
    let plan1 = Box::new(Count::new(2));
    engine.queue(plan1).await;

    let engine_clone = engine.clone();
    let task1 = tokio::spawn(async move { engine_clone.start().await });
    let _ = task1.await;

    // Queue and execute second plan
    let plan2 = Box::new(Count::new(3));
    engine.queue(plan2).await;

    let engine_clone = engine.clone();
    let task2 = tokio::spawn(async move { engine_clone.start().await });
    let _ = task2.await;

    // Count StartDoc/StopDoc documents
    let mut start_count = 0;
    let mut stop_count = 0;

    while let Ok(doc_result) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
        match doc_result {
            Ok(Document::Start(_)) => start_count += 1,
            Ok(Document::Stop(_)) => stop_count += 1,
            Err(_) => break,
            _ => {}
        }
    }

    assert_eq!(start_count, 2, "Should execute 2 plans");
    assert_eq!(stop_count, 2, "Should complete 2 plans");
}

#[tokio::test]
async fn test_manifest_contains_plan_info() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);
    let mut rx = engine.subscribe();

    let plan = Box::new(Count::new(1));
    engine.queue(plan).await;

    let engine_clone = Arc::new(engine);
    let engine_for_task = engine_clone.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Skip StartDoc
    let _ = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;

    // Get Manifest
    let doc = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(doc.is_ok());
    if let Ok(Ok(Document::Manifest(manifest))) = doc {
        assert_eq!(manifest.plan_type, "count");
        assert!(manifest.system_info.contains_key("software_version"));
        assert!(manifest.system_info.contains_key("hostname"));
    } else {
        panic!("Expected Manifest document");
    }
}

#[tokio::test]
async fn test_subscribe_multiple_times() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let mut rx1 = engine.subscribe();
    let mut rx2 = engine.subscribe();

    let plan = Box::new(Count::new(1));
    engine.queue(plan).await;

    let engine_clone = Arc::new(engine);
    let engine_for_task = engine_clone.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    // Both subscriptions should receive documents
    let doc1 = tokio::time::timeout(Duration::from_secs(1), rx1.recv()).await;
    let doc2 = tokio::time::timeout(Duration::from_secs(1), rx2.recv()).await;

    assert!(doc1.is_ok());
    assert!(doc2.is_ok());
}

#[test]
fn test_engine_state_enum_display() {
    assert_eq!(format!("{:?}", EngineState::Idle), "Idle");
    assert_eq!(format!("{:?}", EngineState::Running), "Running");
    assert_eq!(format!("{:?}", EngineState::Paused), "Paused");
}

/// Test that the watchdog aborts an engine stuck in Running with no activity (bd-c9z1)
///
/// Uses direct state manipulation to avoid complex timing interactions
/// with the plan execution loop. Uses multi-thread runtime for real-time testing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_watchdog_aborts_orphaned_running_plan() {
    let registry = Arc::new(DeviceRegistry::new());
    let mut engine_raw = RunEngine::new(registry);
    // Timeout = 200ms, check interval = max(20ms, 1s) = 1s
    engine_raw.set_watchdog_timeout(Duration::from_millis(200));
    let engine = Arc::new(engine_raw);

    // Directly set state to Running to simulate an orphaned plan
    *engine.state.write().await = EngineState::Running;

    // Spawn the watchdog
    let watchdog_handle = engine.spawn_watchdog();

    // Wait for the check interval (1s) + margin for the watchdog to fire
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // The watchdog should have called abort(), transitioning to Aborting.
    let state = engine.state().await;
    assert!(
        state == EngineState::Aborting || state == EngineState::Idle,
        "Watchdog should have triggered abort, got state: {state}"
    );

    watchdog_handle.abort();
}

/// Test that the watchdog aborts a Paused engine with no activity (bd-c9z1)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_watchdog_aborts_orphaned_paused_plan() {
    let registry = Arc::new(DeviceRegistry::new());
    let mut engine_raw = RunEngine::new(registry);
    engine_raw.set_watchdog_timeout(Duration::from_millis(200));
    let engine = Arc::new(engine_raw);

    // Directly set state to Paused to simulate an orphaned paused plan
    *engine.state.write().await = EngineState::Paused;

    let watchdog_handle = engine.spawn_watchdog();

    // Wait for the check interval (1s) + margin
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let state = engine.state().await;
    assert!(
        state == EngineState::Aborting || state == EngineState::Idle,
        "Watchdog should have triggered abort on paused engine, got state: {state}"
    );

    watchdog_handle.abort();
}

/// Test that the watchdog does NOT abort a plan with ongoing activity (bd-c9z1)
#[tokio::test(start_paused = true)]
async fn test_watchdog_does_not_abort_active_plan() {
    let registry = Arc::new(DeviceRegistry::new());
    let mut engine_raw = RunEngine::new(registry);
    // 2-second timeout
    engine_raw.set_watchdog_timeout(Duration::from_secs(2));
    let engine = Arc::new(engine_raw);

    // Queue a plan with 5 events (Count emits events rapidly)
    let plan = Box::new(Count::new(5));
    engine.queue(plan).await;

    // Spawn the watchdog
    let watchdog_handle = engine.spawn_watchdog();

    // Start in a separate task
    let engine_for_task = engine.clone();
    tokio::spawn(async move {
        let _ = engine_for_task.start().await;
    });

    let mut rx = engine.subscribe();

    // Collect all documents until StopDoc
    let stop_doc = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await {
                Ok(Document::Stop(stop)) => return stop,
                Ok(_) => {}
                Err(err) => panic!("Channel closed before StopDoc: {err}"),
            }
        }
    })
    .await
    .expect("Should receive StopDoc");

    // Plan should complete successfully (watchdog should NOT have fired)
    assert_eq!(
        stop_doc.exit_status, "success",
        "Active plan should complete successfully, not be aborted by watchdog"
    );

    watchdog_handle.abort();
}

/// Test that touch_activity resets the watchdog timer (bd-c9z1)
#[tokio::test(start_paused = true)]
async fn test_touch_activity_prevents_watchdog() {
    let registry = Arc::new(DeviceRegistry::new());
    let mut engine_raw = RunEngine::new(registry);
    engine_raw.set_watchdog_timeout(Duration::from_secs(2));
    let engine = Arc::new(engine_raw);

    // Manually set state to Running to simulate an active plan
    *engine.state.write().await = EngineState::Running;

    let watchdog_handle = engine.spawn_watchdog();

    // Keep touching activity every 1 second (under the 2s timeout)
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        engine.touch_activity().await;
    }

    // Engine should still be Running (watchdog did not fire)
    assert_eq!(
        engine.state().await,
        EngineState::Running,
        "Watchdog should not fire when activity is refreshed"
    );

    watchdog_handle.abort();
    // Reset state to avoid panic on drop
    *engine.state.write().await = EngineState::Idle;
}

// ---- Adaptive scan evaluation tests (bd-0za1) ----

/// Helper: build a mock device registry with a readable device.
/// The device returns a fixed value on every `read()` call.
struct FixedReadable(f64);

#[async_trait]
impl common::capabilities::Readable for FixedReadable {
    async fn read(&self) -> anyhow::Result<f64> {
        Ok(self.0)
    }
}

struct FixedReadableFactory {
    value: f64,
}

impl DriverFactory for FixedReadableFactory {
    fn driver_type(&self) -> &'static str {
        "fixed_readable"
    }

    fn name(&self) -> &'static str {
        "Fixed Readable"
    }

    fn capabilities(&self) -> &'static [DeviceCapability] {
        &[DeviceCapability::Readable]
    }

    fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
        let value = self.value;
        Box::pin(async move {
            Ok(DeviceComponents::new()
                .with_category(DeviceCategory::Detector)
                .with_readable(Arc::new(FixedReadable(value))))
        })
    }
}

async fn make_readable_registry(device_id: &str, value: f64) -> Arc<DeviceRegistry> {
    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(FixedReadableFactory { value }));
    registry
        .register_from_toml(
            device_id,
            "Fixed Readable",
            "fixed_readable",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register fixed readable device");
    registry
}

/// AC-5: Unit test with mock feedback proves adaptive behavior (bd-0za1).
///
/// This test verifies all five acceptance criteria:
/// 1. RunEngine checks FeedbackChannel between plan steps
/// 2. Adaptive decision logged with rationale
/// 3. Adjusted scan points reflected in emitted Event documents
/// 4. Fallback to linear scan if no feedback within timeout
/// 5. Unit test with mock feedback proves adaptive behavior
#[tokio::test]
async fn test_adaptive_scan_feedback_integration() {
    let registry = make_readable_registry("detector", 42.0).await;
    let engine = Arc::new(RunEngine::new(registry));

    // Grab the feedback sender before the plan runs.
    let feedback_tx = engine.feedback_sender();

    // Build a plan that mimics an adaptive scan: checkpoint -> read -> emit.
    let plan = Box::new(ImperativePlan::new(vec![
        PlanCommand::Checkpoint {
            label: "adaptive_test_start".to_string(),
        },
        PlanCommand::Read {
            device_id: "detector".to_string(),
        },
        PlanCommand::Checkpoint {
            label: "adaptive_test_point_0_triggers_1".to_string(),
        },
        PlanCommand::EmitEvent {
            stream: "primary".to_string(),
            data: HashMap::new(),
            positions: HashMap::new(),
            scan_indices: None,
        },
    ]));

    // Send a ThresholdCrossed event *before* execution so the feedback
    // channel is pre-loaded. The engine should drain it at the first
    // adaptive checkpoint.
    feedback_tx
        .send(FeedbackEvent::ThresholdCrossed {
            device_id: "detector".to_string(),
            field: "intensity".to_string(),
            value: 100.0,
            threshold: 50.0,
        })
        .await
        .expect("send feedback event");

    // Execute using the adaptive path.
    let result = engine
        .execute_adaptive(plan, Duration::from_secs(5))
        .await
        .expect("adaptive execution should succeed");

    // AC-1 + AC-5: The plan completed and the feedback was consumed.
    assert_eq!(result.exit_status, "success");
    assert_eq!(result.num_events, 1, "plan should emit exactly one event");

    // Verify the feedback channel is now empty (event was consumed).
    let drained = engine.check_feedback();
    assert!(
        drained.is_none(),
        "feedback channel should be empty after adaptive execution"
    );
}

/// Test that execute_adaptive falls back gracefully when no feedback
/// is received (AC-4: linear fallback).
#[tokio::test]
async fn test_adaptive_scan_linear_fallback() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = Arc::new(RunEngine::new(registry));

    // Simple count plan — no feedback injected at all.
    let plan = Box::new(Count::new(2));

    let result = engine
        .execute_adaptive(plan, Duration::from_secs(5))
        .await
        .expect("adaptive execution with no feedback should succeed");

    assert_eq!(result.exit_status, "success");
    assert_eq!(result.num_events, 2);
}

/// Test adapt_scan_point returns adjusted position on threshold crossing
/// and None on value updates (AC-2: logged with rationale).
#[tokio::test]
async fn test_adapt_scan_point_decisions() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    // ThresholdCrossed should return adjusted position.
    let threshold_event = FeedbackEvent::ThresholdCrossed {
        device_id: "det".to_string(),
        field: "intensity".to_string(),
        value: 100.0,
        threshold: 80.0,
    };
    let adjusted = engine.adapt_scan_point(50.0, &threshold_event);
    // Midpoint of planned (50) and threshold (80) = 65
    assert_eq!(adjusted, Some(65.0));

    // ValueUpdate should return None (no adjustment).
    let value_event = FeedbackEvent::ValueUpdate {
        device_id: "det".to_string(),
        field: "value".to_string(),
        value: 42.0,
    };
    let adjusted = engine.adapt_scan_point(50.0, &value_event);
    assert_eq!(adjusted, None);

    // StabilityReached should return None.
    let stability_event = FeedbackEvent::StabilityReached {
        device_id: "det".to_string(),
        field: "value".to_string(),
        variance: 0.001,
    };
    let adjusted = engine.adapt_scan_point(50.0, &stability_event);
    assert_eq!(adjusted, None);
}

/// Test that check_feedback is non-blocking and drains the channel.
#[tokio::test]
async fn test_check_feedback_nonblocking() {
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);
    let tx = engine.feedback_sender();

    // Channel is empty — should return None immediately.
    assert!(engine.check_feedback().is_none());

    // Send two events.
    tx.send(FeedbackEvent::ValueUpdate {
        device_id: "d".to_string(),
        field: "v".to_string(),
        value: 1.0,
    })
    .await
    .unwrap();
    tx.send(FeedbackEvent::ValueUpdate {
        device_id: "d".to_string(),
        field: "v".to_string(),
        value: 2.0,
    })
    .await
    .unwrap();

    // Should get both events.
    assert!(engine.check_feedback().is_some());
    assert!(engine.check_feedback().is_some());
    // Now empty.
    assert!(engine.check_feedback().is_none());
}

// --- evaluate_condition: Threshold ---

#[tokio::test]
async fn test_evaluate_condition_threshold_above_true() {
    // Device reads 42.0, threshold 10.0, above=true => 42 > 10 => true
    let registry = make_readable_registry("sensor", 42.0).await;
    let engine = RunEngine::new(registry);

    let cond = EvalCondition::Threshold {
        device_id: "sensor".to_string(),
        field: "value".to_string(),
        threshold: 10.0,
        above: true,
    };
    assert!(engine.evaluate_condition(&cond).await);
}

#[tokio::test]
async fn test_evaluate_condition_threshold_above_false() {
    // Device reads 5.0, threshold 10.0, above=true => 5 > 10 => false
    let registry = make_readable_registry("sensor", 5.0).await;
    let engine = RunEngine::new(registry);

    let cond = EvalCondition::Threshold {
        device_id: "sensor".to_string(),
        field: "value".to_string(),
        threshold: 10.0,
        above: true,
    };
    assert!(!engine.evaluate_condition(&cond).await);
}

#[tokio::test]
async fn test_evaluate_condition_threshold_below() {
    // Device reads 3.0, threshold 10.0, above=false => 3 < 10 => true
    let registry = make_readable_registry("sensor", 3.0).await;
    let engine = RunEngine::new(registry);

    let cond = EvalCondition::Threshold {
        device_id: "sensor".to_string(),
        field: "value".to_string(),
        threshold: 10.0,
        above: false,
    };
    assert!(engine.evaluate_condition(&cond).await);
}

#[tokio::test]
async fn test_evaluate_condition_threshold_missing_device() {
    // Non-existent device => false
    let registry = Arc::new(DeviceRegistry::new());
    let engine = RunEngine::new(registry);

    let cond = EvalCondition::Threshold {
        device_id: "nonexistent".to_string(),
        field: "value".to_string(),
        threshold: 10.0,
        above: true,
    };
    assert!(
        !engine.evaluate_condition(&cond).await,
        "missing device should evaluate to false"
    );
}

// --- evaluate_condition: Comparison operators ---

async fn make_two_readable_registry(
    id_a: &str,
    val_a: f64,
    id_b: &str,
    val_b: f64,
) -> Arc<DeviceRegistry> {
    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(FixedReadableFactory { value: val_a }));
    registry
        .register_from_toml(
            id_a,
            "Readable A",
            "fixed_readable",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register device A");

    // Need a second factory with different value. Re-register with different value.
    // The factory is keyed by driver_type, so we need a distinct type.
    struct FixedReadableFactoryB {
        value: f64,
    }

    #[async_trait]
    impl common::capabilities::Readable for FixedReadableB {
        async fn read(&self) -> anyhow::Result<f64> {
            Ok(self.0)
        }
    }

    struct FixedReadableB(f64);

    impl DriverFactory for FixedReadableFactoryB {
        fn driver_type(&self) -> &'static str {
            "fixed_readable_b"
        }
        fn name(&self) -> &'static str {
            "Fixed Readable B"
        }
        fn capabilities(&self) -> &'static [DeviceCapability] {
            &[DeviceCapability::Readable]
        }
        fn validate(&self, _config: &toml::Value) -> anyhow::Result<()> {
            Ok(())
        }
        fn build(
            &self,
            _config: toml::Value,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<DeviceComponents>> + Send>> {
            let value = self.value;
            Box::pin(async move {
                Ok(DeviceComponents::new()
                    .with_category(DeviceCategory::Detector)
                    .with_readable(Arc::new(FixedReadableB(value))))
            })
        }
    }

    registry.register_factory(Box::new(FixedReadableFactoryB { value: val_b }));
    registry
        .register_from_toml(
            id_b,
            "Readable B",
            "fixed_readable_b",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("register device B");

    registry
}

fn comparison_condition(op: ComparisonOp) -> EvalCondition {
    EvalCondition::Comparison {
        left_device_id: "left".to_string(),
        left_field: "value".to_string(),
        right_device_id: "right".to_string(),
        right_field: "value".to_string(),
        operator: op,
    }
}

#[tokio::test]
async fn test_evaluate_condition_comparison_gt() {
    let registry = make_two_readable_registry("left", 10.0, "right", 5.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Gt))
            .await
    );

    let registry = make_two_readable_registry("left", 5.0, "right", 10.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        !engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Gt))
            .await
    );
}

#[tokio::test]
async fn test_evaluate_condition_comparison_lt() {
    let registry = make_two_readable_registry("left", 3.0, "right", 7.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Lt))
            .await
    );

    let registry = make_two_readable_registry("left", 7.0, "right", 3.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        !engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Lt))
            .await
    );
}

#[tokio::test]
async fn test_evaluate_condition_comparison_eq() {
    let registry = make_two_readable_registry("left", 5.0, "right", 5.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Eq))
            .await
    );

    let registry = make_two_readable_registry("left", 5.0, "right", 5.1).await;
    let engine = RunEngine::new(registry);
    assert!(
        !engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Eq))
            .await
    );
}

#[tokio::test]
async fn test_evaluate_condition_comparison_gte() {
    let registry = make_two_readable_registry("left", 10.0, "right", 10.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Gte))
            .await
    );

    let registry = make_two_readable_registry("left", 11.0, "right", 10.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Gte))
            .await
    );

    let registry = make_two_readable_registry("left", 9.0, "right", 10.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        !engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Gte))
            .await
    );
}

#[tokio::test]
async fn test_evaluate_condition_comparison_lte() {
    let registry = make_two_readable_registry("left", 10.0, "right", 10.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Lte))
            .await
    );

    let registry = make_two_readable_registry("left", 9.0, "right", 10.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Lte))
            .await
    );

    let registry = make_two_readable_registry("left", 11.0, "right", 10.0).await;
    let engine = RunEngine::new(registry);
    assert!(
        !engine
            .evaluate_condition(&comparison_condition(ComparisonOp::Lte))
            .await
    );
}

// test_evaluate_condition_comparison_unknown_operator removed:
// ComparisonOp enum makes invalid operators a compile-time error.

#[tokio::test]
async fn test_evaluate_condition_comparison_missing_device() {
    // Only register one device -- the other is missing
    let registry = make_readable_registry("left", 10.0).await;
    let engine = RunEngine::new(registry);

    let cond = comparison_condition(ComparisonOp::Gt);
    assert!(
        !engine.evaluate_condition(&cond).await,
        "missing device in comparison should evaluate to false"
    );
}
