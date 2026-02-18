#![cfg(not(target_arch = "wasm32"))]
//! Comprehensive mock mode coverage tests for Andor SDK3 driver [bd-u92f]
//!
//! Exercises all capability trait implementations through the mock backend:
//! - `AndorCamera` (mock mode): FrameProducer, Triggerable, ExposureControl, Parameterized
//! - `AndorSpectrograph` (mock mode): WavelengthTunable, ShutterControl, Parameterized
//! - `MockCamera` (standalone): Frame generation, observer pattern, parameter introspection
//! - `MockSpectrograph` (standalone): Wavelength calibration, grating control
//!
//! # Running
//!
//! ```bash
//! cargo nextest run -p driver-andor-sdk3 --test mock_coverage
//! ```

use common::capabilities::{
    ExposureControl, FrameProducer, Parameterized, ShutterControl, Triggerable, WavelengthTunable,
};
use driver_andor_sdk3::mock::{MockCamera, MockSpectrograph};
use driver_andor_sdk3::{AndorCamera, AndorSpectrograph};

// =============================================================================
// MockCamera — Standalone Unit Tests
// =============================================================================

mod mock_camera_unit {
    use super::*;

    #[tokio::test]
    async fn creation_returns_valid_defaults() {
        let cam = MockCamera::new();

        let (w, h) = cam.resolution();
        assert_eq!(w, 2048, "default sensor width");
        assert_eq!(h, 2048, "default sensor height");
    }

    #[tokio::test]
    async fn exposure_roundtrip() {
        let cam = MockCamera::new();

        cam.set_exposure(0.05).await.unwrap();
        let exp = cam.get_exposure().await.unwrap();
        assert!((exp - 0.05).abs() < 1e-9, "exposure should round-trip");
    }

    #[tokio::test]
    async fn exposure_multiple_updates() {
        let cam = MockCamera::new();

        for &val in &[0.001, 0.01, 0.1, 1.0, 10.0] {
            cam.set_exposure(val).await.unwrap();
            let got = cam.get_exposure().await.unwrap();
            assert!(
                (got - val).abs() < 1e-9,
                "exposure {val} should round-trip, got {got}"
            );
        }
    }

    #[tokio::test]
    async fn arm_and_trigger_lifecycle() {
        let cam = MockCamera::new();

        // Initially not armed
        assert!(!cam.is_armed().await.unwrap(), "should start unarmed");

        // Arm
        cam.arm().await.unwrap();
        assert!(cam.is_armed().await.unwrap(), "should be armed after arm()");

        // Trigger succeeds when armed
        cam.trigger().await.unwrap();
    }

    #[tokio::test]
    async fn trigger_without_arm_fails() {
        let cam = MockCamera::new();

        let result = cam.trigger().await;
        assert!(result.is_err(), "trigger without arm should fail");
        assert!(
            result.unwrap_err().to_string().contains("not armed"),
            "error should mention 'not armed'"
        );
    }

    #[tokio::test]
    async fn stream_start_stop() {
        let cam = MockCamera::new();
        cam.set_exposure(10.0).await.unwrap(); // long exposure to prevent runaway

        cam.start_stream().await.unwrap();
        // Stop should succeed
        cam.stop_stream().await.unwrap();
    }

    #[tokio::test]
    async fn observer_register_unregister() {
        use common::capabilities::FrameObserver;
        use common::data::FrameView;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        struct CountingObserver {
            count: Arc<AtomicU64>,
        }

        impl FrameObserver for CountingObserver {
            fn on_frame(&self, _frame: &FrameView<'_>) {
                self.count.fetch_add(1, Ordering::Relaxed);
            }
            fn name(&self) -> &'static str {
                "counting_observer"
            }
        }

        let cam = MockCamera::new();
        let count = Arc::new(AtomicU64::new(0));

        // Register
        let handle = cam
            .register_observer(Box::new(CountingObserver {
                count: count.clone(),
            }))
            .await
            .unwrap();

        assert_ne!(handle.id(), 0, "handle should have non-zero id");

        // Unregister
        cam.unregister_observer(handle).await.unwrap();
    }

    #[tokio::test]
    async fn multiple_observers_unique_handles() {
        use common::capabilities::FrameObserver;
        use common::data::FrameView;

        struct NoopObserver;
        impl FrameObserver for NoopObserver {
            fn on_frame(&self, _frame: &FrameView<'_>) {}
            fn name(&self) -> &'static str {
                "noop"
            }
        }

        let cam = MockCamera::new();

        let h1 = cam.register_observer(Box::new(NoopObserver)).await.unwrap();
        let h2 = cam.register_observer(Box::new(NoopObserver)).await.unwrap();

        assert_ne!(h1, h2, "observer handles must be unique");

        // Cleanup
        cam.unregister_observer(h1).await.unwrap();
        cam.unregister_observer(h2).await.unwrap();
    }

    #[tokio::test]
    async fn parameter_introspection() {
        let cam = MockCamera::new();
        let params = cam.parameters();

        let names = params.names();
        assert!(
            names.contains(&"exposure_s"),
            "should have exposure_s parameter, got: {:?}",
            names
        );
        assert!(
            names.contains(&"mcp_gain"),
            "should have mcp_gain parameter, got: {:?}",
            names
        );
    }

    #[tokio::test]
    async fn parameter_json_get_set() {
        let cam = MockCamera::new();
        let params = cam.parameters();

        // Get exposure via JSON
        let param = params.get("exposure_s").expect("exposure_s should exist");
        let json = param.get_json().unwrap();
        assert!(json.is_number(), "exposure should be a number");

        // Set via JSON
        param.set_json(serde_json::json!(0.25)).unwrap();
        let updated = param.get_json().unwrap();
        assert_eq!(updated, serde_json::json!(0.25));
    }

    #[tokio::test]
    async fn parameter_metadata() {
        let cam = MockCamera::new();
        let params = cam.parameters();

        let param = params.get("exposure_s").expect("exposure_s should exist");
        let meta = param.metadata();
        assert_eq!(
            meta.units.as_deref(),
            Some("s"),
            "exposure units should be s"
        );
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let cam1 = MockCamera::new();
        let cam2 = cam1.clone();

        cam1.set_exposure(0.123).await.unwrap();
        let exp2 = cam2.get_exposure().await.unwrap();
        assert!(
            (exp2 - 0.123).abs() < 1e-9,
            "cloned camera should share exposure state"
        );
    }
}

// =============================================================================
// MockSpectrograph — Standalone Unit Tests
// =============================================================================

mod mock_spectrograph_unit {
    use super::*;

    #[tokio::test]
    async fn creation_returns_valid_defaults() {
        let spec = MockSpectrograph::new();

        let wl = spec.get_wavelength().await.unwrap();
        assert!(
            (wl - 310.0).abs() < 1e-6,
            "default wavelength should be 310nm"
        );
    }

    #[tokio::test]
    async fn wavelength_set_get_roundtrip() {
        let spec = MockSpectrograph::new();

        spec.set_wavelength(500.0).await.unwrap();
        let wl = spec.get_wavelength().await.unwrap();
        assert!(
            (wl - 500.0).abs() < 1e-6,
            "wavelength should round-trip to 500nm"
        );
    }

    #[tokio::test]
    async fn wavelength_range() {
        let spec = MockSpectrograph::new();

        let (min, max) = spec.wavelength_range();
        assert_eq!(min, 200.0, "min wavelength should be 200nm");
        assert_eq!(max, 1000.0, "max wavelength should be 1000nm");
    }

    #[tokio::test]
    async fn shutter_lifecycle() {
        let spec = MockSpectrograph::new();

        // Initially closed
        assert!(
            !spec.is_shutter_open().await.unwrap(),
            "shutter should start closed"
        );

        // Open
        spec.open_shutter().await.unwrap();
        assert!(
            spec.is_shutter_open().await.unwrap(),
            "shutter should be open after open_shutter()"
        );

        // Close
        spec.close_shutter().await.unwrap();
        assert!(
            !spec.is_shutter_open().await.unwrap(),
            "shutter should be closed after close_shutter()"
        );
    }

    #[tokio::test]
    async fn shutter_double_open_is_idempotent() {
        let spec = MockSpectrograph::new();

        spec.open_shutter().await.unwrap();
        spec.open_shutter().await.unwrap();
        assert!(spec.is_shutter_open().await.unwrap());

        spec.close_shutter().await.unwrap();
        assert!(!spec.is_shutter_open().await.unwrap());
    }

    #[tokio::test]
    async fn parameter_introspection() {
        let spec = MockSpectrograph::new();
        let params = spec.parameters();

        let names = params.names();
        assert!(
            names.contains(&"wavelength_nm"),
            "should have wavelength_nm parameter, got: {:?}",
            names
        );
        assert!(
            names.contains(&"grating"),
            "should have grating parameter, got: {:?}",
            names
        );
    }

    #[tokio::test]
    async fn parameter_json_roundtrip() {
        let spec = MockSpectrograph::new();
        let params = spec.parameters();

        let param = params
            .get("wavelength_nm")
            .expect("wavelength_nm should exist");
        let meta = param.metadata();
        assert_eq!(
            meta.units.as_deref(),
            Some("nm"),
            "wavelength units should be nm"
        );

        // Set and verify via JSON
        param.set_json(serde_json::json!(400.0)).unwrap();
        let updated = param.get_json().unwrap();
        assert_eq!(updated, serde_json::json!(400.0));
    }

    #[tokio::test]
    async fn wavelength_calibration() {
        let spec = MockSpectrograph::new();

        let cal = spec.get_wavelength_calibration(2048).await.unwrap();
        let wavelengths = cal.wavelengths_nm;
        assert_eq!(
            wavelengths.len(),
            2048,
            "calibration should have 2048 entries"
        );

        // Wavelengths should be monotonically increasing
        for pair in wavelengths.windows(2) {
            assert!(
                pair[1] > pair[0],
                "wavelengths should be monotonically increasing"
            );
        }

        // Center pixel should be close to center wavelength (310nm default)
        let center_wl = wavelengths[1024];
        assert!(
            (center_wl - 310.0).abs() < 5.0,
            "center wavelength should be near 310nm, got {}",
            center_wl
        );
    }

    #[tokio::test]
    async fn wavelength_change_updates_calibration() {
        let spec = MockSpectrograph::new();

        // Set to 500nm
        spec.set_wavelength(500.0).await.unwrap();

        let cal = spec.get_wavelength_calibration(1024).await.unwrap();
        let wavelengths = cal.wavelengths_nm;
        assert_eq!(wavelengths.len(), 1024);

        // Center should be near 500nm
        let center_wl = wavelengths[512];
        assert!(
            (center_wl - 500.0).abs() < 5.0,
            "calibration center should track wavelength, got {}",
            center_wl
        );
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let spec1 = MockSpectrograph::new();
        let spec2 = spec1.clone();

        // Shutter state should be shared
        spec1.open_shutter().await.unwrap();
        assert!(
            spec2.is_shutter_open().await.unwrap(),
            "cloned spectrograph should share shutter state"
        );
    }
}

// =============================================================================
// AndorCamera — Mock Mode Integration Tests
// =============================================================================

mod andor_camera_mock {
    use super::*;

    #[tokio::test]
    async fn new_mock_succeeds() {
        let camera = AndorCamera::new_mock().await;
        assert!(camera.is_ok(), "new_mock() should succeed");
    }

    #[tokio::test]
    async fn resolution_is_valid() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let (w, h) = camera.resolution();
        assert_eq!(w, 2048, "mock width should be 2048");
        assert_eq!(h, 2048, "mock height should be 2048");
    }

    #[tokio::test]
    async fn camera_info() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let info = camera.info();

        assert!(!info.model.is_empty(), "model should not be empty");
        assert!(!info.serial_number.is_empty(), "serial should not be empty");
        assert!(
            !info.firmware_version.is_empty(),
            "firmware version should not be empty"
        );
        assert!(info.sensor_width > 0);
        assert!(info.sensor_height > 0);
    }

    #[tokio::test]
    async fn exposure_set_get() {
        let camera = AndorCamera::new_mock().await.unwrap();

        camera.set_exposure(0.05).await.unwrap();
        let exp = camera.get_exposure().await.unwrap();
        assert!(
            (exp - 0.05).abs() < 1e-6,
            "exposure should round-trip to 50ms"
        );
    }

    #[tokio::test]
    async fn exposure_rejects_zero() {
        let camera = AndorCamera::new_mock().await.unwrap();

        let result = camera.set_exposure(0.0).await;
        assert!(result.is_err(), "zero exposure should be rejected");
    }

    #[tokio::test]
    async fn exposure_rejects_negative() {
        let camera = AndorCamera::new_mock().await.unwrap();

        let result = camera.set_exposure(-0.001).await;
        assert!(result.is_err(), "negative exposure should be rejected");
    }

    #[tokio::test]
    async fn trigger_lifecycle() {
        let camera = AndorCamera::new_mock().await.unwrap();

        // Not armed initially
        let armed = camera.is_armed().await.unwrap();
        assert!(!armed, "should not be armed initially");

        // Arm
        camera.arm().await.unwrap();
        let armed = camera.is_armed().await.unwrap();
        assert!(armed, "should be armed after arm()");

        // Trigger
        camera.trigger().await.unwrap();
    }

    #[tokio::test]
    async fn trigger_without_arm_errors() {
        let camera = AndorCamera::new_mock().await.unwrap();

        let result = camera.trigger().await;
        assert!(result.is_err(), "triggering unarmed camera should error");
    }

    #[tokio::test]
    async fn stream_start_stop() {
        let camera = AndorCamera::new_mock().await.unwrap();

        camera.start_stream().await.unwrap();
        camera.stop_stream().await.unwrap();
    }

    #[tokio::test]
    async fn parameter_set_names() {
        let camera = AndorCamera::new_mock().await.unwrap();
        let params = camera.parameters();
        let names = params.names();

        for expected in &[
            "exposure_s",
            "trigger_mode",
            "gate_mode",
            "mcp_gain",
            "roi",
            "binning",
            "temperature_c",
        ] {
            assert!(
                names.contains(expected),
                "missing parameter '{}', have: {:?}",
                expected,
                names
            );
        }
    }

    #[tokio::test]
    async fn mcp_gain_set_get() {
        let camera = AndorCamera::new_mock().await.unwrap();

        camera.set_mcp_gain(2000).await.unwrap();
        let gain = camera.get_mcp_gain().await.unwrap();
        assert_eq!(gain, 2000, "MCP gain should round-trip");
    }

    #[tokio::test]
    async fn mcp_gain_range_validation() {
        let camera = AndorCamera::new_mock().await.unwrap();

        // Valid range is 0-4095
        camera.set_mcp_gain(0).await.unwrap();
        camera.set_mcp_gain(4095).await.unwrap();

        // Out of range should fail
        let result = camera.set_mcp_gain(5000).await;
        assert!(result.is_err(), "MCP gain > 4095 should be rejected");
    }

    #[tokio::test]
    async fn ddg_timing_set() {
        let camera = AndorCamera::new_mock().await.unwrap();

        // Set DDG timing (delay and width in picoseconds)
        camera.set_ddg_output_delay(1_300_000).await.unwrap();
        camera.set_ddg_output_width(10_000_000).await.unwrap();
    }

    #[tokio::test]
    async fn trigger_mode_set() {
        let camera = AndorCamera::new_mock().await.unwrap();

        camera.set_trigger_mode("External").await.unwrap();
        let mode = camera.get_trigger_mode().await.unwrap();
        assert_eq!(mode, "External", "trigger mode should round-trip");
    }

    #[tokio::test]
    async fn gate_mode_set() {
        let camera = AndorCamera::new_mock().await.unwrap();

        camera.set_gate_mode("DDG").await.unwrap();
    }

    #[tokio::test]
    async fn cooling_control() {
        let camera = AndorCamera::new_mock().await.unwrap();

        // Enable/disable cooling should not error in mock mode
        camera.set_cooling(true).await.unwrap();
        camera.set_cooling(false).await.unwrap();
    }

    #[tokio::test]
    async fn temperature_read() {
        let camera = AndorCamera::new_mock().await.unwrap();

        let temp = camera.get_temperature().await.unwrap();
        // Mock returns a sensible default temperature
        assert!(
            temp > -50.0 && temp < 100.0,
            "temperature should be reasonable, got {temp}"
        );
    }

    #[tokio::test]
    async fn observer_register_unregister() {
        use common::capabilities::FrameObserver;
        use common::data::FrameView;

        struct TestObserver;
        impl FrameObserver for TestObserver {
            fn on_frame(&self, _frame: &FrameView<'_>) {}
            fn name(&self) -> &'static str {
                "test"
            }
        }

        let camera = AndorCamera::new_mock().await.unwrap();

        let handle = camera
            .register_observer(Box::new(TestObserver))
            .await
            .unwrap();

        camera.unregister_observer(handle).await.unwrap();
    }

    #[tokio::test]
    async fn full_libs_workflow() {
        // Simulate a typical LIBS acquisition workflow in mock mode
        let camera = AndorCamera::new_mock().await.unwrap();

        // 1. Configure acquisition parameters
        camera.set_trigger_mode("External").await.unwrap();
        camera.set_exposure(0.0015).await.unwrap(); // 1.5ms
        camera.set_gate_mode("DDG").await.unwrap();
        camera.set_mcp_gain(3600).await.unwrap();
        camera.set_ddg_output_delay(1_300_000).await.unwrap(); // 1.3µs
        camera.set_ddg_output_width(10_000_000).await.unwrap(); // 10µs

        // 2. Arm camera
        camera.arm().await.unwrap();
        assert!(camera.is_armed().await.unwrap());

        // 3. Trigger (simulates external trigger arriving)
        camera.trigger().await.unwrap();

        // 4. Verify camera info accessible throughout
        let info = camera.info();
        assert!(info.sensor_width > 0);
    }
}

// =============================================================================
// AndorSpectrograph — Mock Mode Integration Tests
// =============================================================================

mod andor_spectrograph_mock {
    use super::*;

    #[tokio::test]
    async fn new_mock_succeeds() {
        let spec = AndorSpectrograph::new_mock().await;
        assert!(spec.is_ok(), "new_mock() should succeed");
    }

    #[tokio::test]
    async fn spectrograph_info() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();
        let info = spec.info();

        assert!(!info.model.is_empty(), "model should not be empty");
        assert!(!info.serial_number.is_empty(), "serial should not be empty");
        assert!(info.num_gratings > 0, "should have at least one grating");
    }

    #[tokio::test]
    async fn wavelength_set_get() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();

        spec.set_wavelength(500.0).await.unwrap();
        let wl = spec.get_wavelength().await.unwrap();
        assert!(
            (wl - 500.0).abs() < 1e-6,
            "wavelength should round-trip to 500nm"
        );
    }

    #[tokio::test]
    async fn wavelength_range() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();

        let (min, max) = spec.wavelength_range();
        assert!(min > 0.0, "min wavelength should be positive");
        assert!(max > min, "max should be greater than min");
    }

    #[tokio::test]
    async fn shutter_control() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();

        // Initially closed
        assert!(
            !spec.is_shutter_open().await.unwrap(),
            "shutter should start closed"
        );

        // Open
        spec.open_shutter().await.unwrap();
        assert!(
            spec.is_shutter_open().await.unwrap(),
            "shutter should be open"
        );

        // Close
        spec.close_shutter().await.unwrap();
        assert!(
            !spec.is_shutter_open().await.unwrap(),
            "shutter should be closed"
        );
    }

    #[tokio::test]
    async fn parameter_set_names() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();
        let params = spec.parameters();
        let names = params.names();

        for expected in &[
            "wavelength_nm",
            "grating",
            "slit_width_um",
            "flipper_mirror",
        ] {
            assert!(
                names.contains(expected),
                "missing parameter '{}', have: {:?}",
                expected,
                names
            );
        }
    }

    #[tokio::test]
    async fn grating_set_get() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();

        spec.set_grating(1).await.unwrap();
        let grating = spec.get_grating().await.unwrap();
        assert_eq!(grating, 1, "grating should round-trip to 1");
    }

    #[tokio::test]
    async fn grating_info() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();

        let info = spec.get_grating_info(1).await.unwrap();
        assert!(info.lines_per_mm > 0.0, "lines/mm should be positive");
        assert!(
            info.blaze_wavelength_nm > 0.0,
            "blaze wavelength should be positive"
        );
    }

    #[tokio::test]
    async fn slit_width_set_get() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();

        spec.set_slit_width(2, 200.0).await.unwrap();
        let width = spec.get_slit_width(2).await.unwrap();
        assert!(
            (width - 200.0).abs() < 1e-6,
            "slit width should round-trip to 200µm"
        );
    }

    #[tokio::test]
    async fn wavelength_calibration() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();

        let cal = spec.get_wavelength_calibration(2048).await.unwrap();
        let wavelengths = cal.wavelengths_nm;
        assert_eq!(
            wavelengths.len(),
            2048,
            "calibration should have 2048 entries"
        );

        // Should be monotonically increasing
        for pair in wavelengths.windows(2) {
            assert!(pair[1] > pair[0], "wavelengths should increase");
        }
    }

    #[tokio::test]
    async fn full_spectroscopy_workflow() {
        // Simulate typical LIBS spectroscopy workflow in mock mode
        let spec = AndorSpectrograph::new_mock().await.unwrap();

        // 1. Select grating
        spec.set_grating(2).await.unwrap();

        // 2. Set wavelength
        spec.set_wavelength(310.0).await.unwrap();

        // 3. Configure slit
        spec.set_slit_width(2, 150.0).await.unwrap();

        // 4. Open shutter
        spec.open_shutter().await.unwrap();
        assert!(spec.is_shutter_open().await.unwrap());

        // 5. Get calibration for camera pixels
        let cal = spec.get_wavelength_calibration(2048).await.unwrap();
        assert_eq!(cal.wavelengths_nm.len(), 2048);

        // 6. Close shutter after acquisition
        spec.close_shutter().await.unwrap();
        assert!(!spec.is_shutter_open().await.unwrap());
    }
}

// =============================================================================
// Cross-Driver Trait Compatibility Tests
// =============================================================================

mod trait_compatibility {
    use super::*;

    /// Verify MockCamera can be used as a generic FrameProducer
    #[tokio::test]
    async fn mock_camera_as_frame_producer() {
        let cam = MockCamera::new();
        exercise_frame_producer(&cam).await;
    }

    /// Verify AndorCamera (mock) can be used as a generic FrameProducer
    #[tokio::test]
    async fn andor_camera_as_frame_producer() {
        let cam = AndorCamera::new_mock().await.unwrap();
        exercise_frame_producer(&cam).await;
    }

    async fn exercise_frame_producer(fp: &(impl FrameProducer + ExposureControl)) {
        fp.set_exposure(10.0).await.unwrap(); // long exposure for safety
        let (w, h) = fp.resolution();
        assert!(w > 0 && h > 0, "resolution should be positive");
        fp.start_stream().await.unwrap();
        fp.stop_stream().await.unwrap();
    }

    /// Verify MockCamera can be used as a generic Triggerable
    #[tokio::test]
    async fn mock_camera_as_triggerable() {
        let cam = MockCamera::new();
        exercise_triggerable(&cam).await;
    }

    /// Verify AndorCamera (mock) can be used as a generic Triggerable
    #[tokio::test]
    async fn andor_camera_as_triggerable() {
        let cam = AndorCamera::new_mock().await.unwrap();
        exercise_triggerable(&cam).await;
    }

    async fn exercise_triggerable(t: &impl Triggerable) {
        t.arm().await.unwrap();
        assert!(t.is_armed().await.unwrap());
        t.trigger().await.unwrap();
    }

    /// Verify MockSpectrograph can be used as a generic WavelengthTunable
    #[tokio::test]
    async fn mock_spectrograph_as_wavelength_tunable() {
        let spec = MockSpectrograph::new();
        exercise_wavelength_tunable(&spec).await;
    }

    /// Verify AndorSpectrograph (mock) can be used as a generic WavelengthTunable
    #[tokio::test]
    async fn andor_spectrograph_as_wavelength_tunable() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();
        exercise_wavelength_tunable(&spec).await;
    }

    async fn exercise_wavelength_tunable(wt: &impl WavelengthTunable) {
        let (min, max) = wt.wavelength_range();
        let mid = f64::midpoint(min, max);

        wt.set_wavelength(mid).await.unwrap();
        let actual = wt.get_wavelength().await.unwrap();
        assert!(
            (actual - mid).abs() < 1.0,
            "wavelength should be near {mid}, got {actual}"
        );
    }

    /// Verify MockSpectrograph can be used as a generic ShutterControl
    #[tokio::test]
    async fn mock_spectrograph_as_shutter_control() {
        let spec = MockSpectrograph::new();
        exercise_shutter_control(&spec).await;
    }

    /// Verify AndorSpectrograph (mock) can be used as a generic ShutterControl
    #[tokio::test]
    async fn andor_spectrograph_as_shutter_control() {
        let spec = AndorSpectrograph::new_mock().await.unwrap();
        exercise_shutter_control(&spec).await;
    }

    async fn exercise_shutter_control(sc: &impl ShutterControl) {
        sc.open_shutter().await.unwrap();
        assert!(sc.is_shutter_open().await.unwrap());
        sc.close_shutter().await.unwrap();
        assert!(!sc.is_shutter_open().await.unwrap());
    }
}
