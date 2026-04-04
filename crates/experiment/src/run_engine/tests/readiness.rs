//! Tests for calibration readiness and config-match gating.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::time::Duration;

use super::super::*;
use super::helpers::{
    make_echelle_registry, make_spectroscopy_registry,
    make_spectroscopy_registry_with_spectrometer, MockEchelleCameraConfig,
};
use crate::plans::Count;
use echelle::EchelleFrameCompatibility;

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
        "unexpected issue: {issues:?}"
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
        "unexpected issue: {issues:?}"
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

    let snapshots = engine.readiness.active_calibrations.read().await;
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

    let snapshots = engine.readiness.active_calibrations.read().await;
    let snapshot = snapshots
        .get("spectroscopy")
        .expect("radiance fields should remain after clearing echelle state");
    assert!(snapshot.calibration_timestamp.is_some());
    assert!(!snapshot.grating_wavelength_coverage.is_empty());
    assert_eq!(snapshot.target_device_id, None);
    assert_eq!(snapshot.echelle_frame_compatibility, None);
}
