//! Comprehensive test suite for PVCAM driver features
//!
//! ## Test Categories
//!
//! 1. **Unit Tests (mock mode)**: Type conversions, enum mappings
//! 2. **Mock Integration Tests**: Feature functions with mock connection
//! 3. **Hardware Integration Tests**: Real hardware validation (requires `hardware_tests` feature)
//!
//! ## Running Tests
//!
//! ```bash
//! # Mock mode tests (no hardware required)
//! cargo test -p daq-driver-pvcam
//!
//! # Hardware tests (on remote machine with Prime BSI)
//! cargo test -p daq-driver-pvcam --features "pvcam_hardware,hardware_tests" -- --nocapture
//! ```

// Import all public types from the library root
use daq_driver_pvcam::{
    CameraInfo, CentroidsConfig, CentroidsMode, ClearMode, ExposeOutMode, ExposureMode, FanSpeed,
    GainMode, PPFeature, PPParam, ReadoutPort, ShutterMode, ShutterStatus, SmartStreamEntry,
    SmartStreamMode, SpeedMode,
};
// Import feature functions
use daq_driver_pvcam::components::features::PvcamFeatures;

// =============================================================================
// Unit Tests: Type Conversions (Mock Mode)
// =============================================================================

mod type_conversions {
    use super::*;

    #[test]
    fn fan_speed_display() {
        assert_eq!(format!("{:?}", FanSpeed::High), "High");
        assert_eq!(format!("{:?}", FanSpeed::Medium), "Medium");
        assert_eq!(format!("{:?}", FanSpeed::Low), "Low");
        assert_eq!(format!("{:?}", FanSpeed::Off), "Off");
    }

    #[test]
    fn shutter_mode_variants() {
        // Verify all variants exist and are distinct
        let modes = [
            ShutterMode::Normal,
            ShutterMode::Open,
            ShutterMode::Closed,
            ShutterMode::None,
            ShutterMode::PreOpen,
        ];

        for (i, mode) in modes.iter().enumerate() {
            for (j, other) in modes.iter().enumerate() {
                if i == j {
                    assert_eq!(mode, other);
                } else {
                    assert_ne!(mode, other);
                }
            }
        }
    }

    #[test]
    fn shutter_status_variants() {
        let statuses = [
            ShutterStatus::Closed,
            ShutterStatus::Open,
            ShutterStatus::Opening,
            ShutterStatus::Closing,
            ShutterStatus::Fault,
            ShutterStatus::Unknown,
        ];

        for (i, status) in statuses.iter().enumerate() {
            for (j, other) in statuses.iter().enumerate() {
                if i == j {
                    assert_eq!(status, other);
                } else {
                    assert_ne!(status, other);
                }
            }
        }
    }

    #[test]
    fn smart_stream_mode_variants() {
        assert_ne!(SmartStreamMode::Exposures, SmartStreamMode::Interleaved);
        assert_eq!(SmartStreamMode::Exposures, SmartStreamMode::Exposures);
    }

    #[test]
    fn smart_stream_entry_clone() {
        let entry = SmartStreamEntry { exposure_ms: 100 };
        let cloned = entry.clone();
        assert_eq!(entry.exposure_ms, cloned.exposure_ms);
    }

    #[test]
    fn exposure_mode_variants() {
        let modes = [
            ExposureMode::Timed,
            ExposureMode::Strobe,
            ExposureMode::Bulb,
            ExposureMode::TriggerFirst,
            ExposureMode::EdgeTrigger,
        ];

        // All variants should be distinct
        for (i, mode) in modes.iter().enumerate() {
            for (j, other) in modes.iter().enumerate() {
                if i == j {
                    assert_eq!(mode, other);
                } else {
                    assert_ne!(mode, other);
                }
            }
        }
    }

    #[test]
    fn clear_mode_variants() {
        let modes = [
            ClearMode::Never,
            ClearMode::PreExposure,
            ClearMode::PreSequence,
            ClearMode::PostSequence,
            ClearMode::PrePostSequence,
            ClearMode::PreExposurePostSequence,
        ];

        for (i, mode) in modes.iter().enumerate() {
            for (j, other) in modes.iter().enumerate() {
                if i == j {
                    assert_eq!(mode, other);
                } else {
                    assert_ne!(mode, other);
                }
            }
        }
    }

    #[test]
    fn expose_out_mode_variants() {
        let modes = [
            ExposeOutMode::FirstRow,
            ExposeOutMode::AllRows,
            ExposeOutMode::AnyRow,
            ExposeOutMode::RollingShutter,
            ExposeOutMode::LineOutput,
        ];

        for (i, mode) in modes.iter().enumerate() {
            for (j, other) in modes.iter().enumerate() {
                if i == j {
                    assert_eq!(mode, other);
                } else {
                    assert_ne!(mode, other);
                }
            }
        }
    }

    #[test]
    fn centroids_mode_variants() {
        let modes = [
            CentroidsMode::Locate,
            CentroidsMode::Track,
            CentroidsMode::Blob,
        ];

        for (i, mode) in modes.iter().enumerate() {
            for (j, other) in modes.iter().enumerate() {
                if i == j {
                    assert_eq!(mode, other);
                } else {
                    assert_ne!(mode, other);
                }
            }
        }
    }

    #[test]
    fn camera_info_structure() {
        let info = CameraInfo {
            serial_number: "TEST-001".to_string(),
            firmware_version: "1.0.0".to_string(),
            chip_name: "TestSensor".to_string(),
            temperature_c: -40.0,
            bit_depth: 16,
            pixel_time_ns: 10,
            pixel_size_nm: (6500, 6500),
            sensor_size: (2048, 2048),
            gain_name: "HDR".to_string(),
            speed_name: "100 MHz".to_string(),
            port_name: "Sensitivity".to_string(),
            gain_index: 0,
            speed_index: 0,
        };

        assert_eq!(info.serial_number, "TEST-001");
        assert_eq!(info.sensor_size, (2048, 2048));
        assert_eq!(info.bit_depth, 16);
    }

    #[test]
    fn gain_mode_structure() {
        let mode = GainMode {
            index: 0,
            name: "HDR".to_string(),
        };
        assert_eq!(mode.index, 0);
        assert_eq!(mode.name, "HDR");
    }

    #[test]
    fn speed_mode_structure() {
        let mode = SpeedMode {
            index: 0,
            name: "100 MHz".to_string(),
            pixel_time_ns: 10,
            bit_depth: 16,
            port_index: 0,
        };
        assert_eq!(mode.pixel_time_ns, 10);
        assert_eq!(mode.bit_depth, 16);
    }

    #[test]
    fn readout_port_structure() {
        let port = ReadoutPort {
            index: 0,
            name: "Sensitivity".to_string(),
        };
        assert_eq!(port.index, 0);
        assert_eq!(port.name, "Sensitivity");
    }

    #[test]
    fn pp_feature_structure() {
        let feature = PPFeature {
            index: 0,
            id: 1,
            name: "PrimeEnhance".to_string(),
        };
        assert_eq!(feature.id, 1);
        assert_eq!(feature.name, "PrimeEnhance");
    }

    #[test]
    fn pp_param_structure() {
        let param = PPParam {
            index: 0,
            id: 1,
            name: "Enabled".to_string(),
            value: 1,
        };
        assert_eq!(param.value, 1);
    }

    #[test]
    fn centroids_config_structure() {
        let config = CentroidsConfig {
            mode: CentroidsMode::Locate,
            radius: 3,
            max_count: 1000,
            threshold: 100,
        };
        assert_eq!(config.mode, CentroidsMode::Locate);
        assert_eq!(config.radius, 3);
    }
}

// =============================================================================
// Mock Integration Tests: Feature Functions
// =============================================================================

#[cfg(not(feature = "pvcam_hardware"))]
mod mock_features {
    use super::*;
    use daq_driver_pvcam::components::connection::PvcamConnection;

    fn mock_connection() -> PvcamConnection {
        PvcamConnection::new()
    }

    #[test]
    fn get_temperature_mock() {
        let conn = mock_connection();
        let temp = PvcamFeatures::get_temperature(&conn).unwrap();
        // MockCameraState defaults to 25.0 (room temperature)
        assert_eq!(
            temp, 25.0,
            "Mock temperature should match default mock state"
        );
    }

    #[test]
    fn set_temperature_setpoint_mock() {
        let conn = mock_connection();
        let result = PvcamFeatures::set_temperature_setpoint(&conn, -30.0);
        assert!(
            result.is_ok(),
            "Setting temperature setpoint should succeed in mock mode"
        );
    }

    #[test]
    fn get_camera_info_mock() {
        let conn = mock_connection();
        let info = PvcamFeatures::get_camera_info(&conn).unwrap();

        assert_eq!(info.serial_number, "MOCK-001");
        assert_eq!(info.firmware_version, "1.0.0");
        assert_eq!(info.chip_name, "MockSensor");
        assert_eq!(info.temperature_c, -40.0);
        assert_eq!(info.bit_depth, 16);
        assert_eq!(info.sensor_size, (2048, 2048));
    }

    #[test]
    fn get_serial_number_mock() {
        let conn = mock_connection();
        let serial = PvcamFeatures::get_serial_number(&conn).unwrap();
        assert_eq!(serial, "MOCK-001");
    }

    #[test]
    fn get_firmware_version_mock() {
        let conn = mock_connection();
        let version = PvcamFeatures::get_firmware_version(&conn).unwrap();
        assert_eq!(version, "1.0.0");
    }

    #[test]
    fn get_chip_name_mock() {
        let conn = mock_connection();
        let name = PvcamFeatures::get_chip_name(&conn).unwrap();
        assert_eq!(name, "MockSensor");
    }

    #[test]
    fn get_bit_depth_mock() {
        let conn = mock_connection();
        let depth = PvcamFeatures::get_bit_depth(&conn).unwrap();
        assert_eq!(depth, 16);
    }

    #[test]
    fn get_pixel_time_mock() {
        let conn = mock_connection();
        let time = PvcamFeatures::get_pixel_time(&conn).unwrap();
        assert_eq!(time, 10);
    }

    #[test]
    fn fan_speed_mock() {
        let conn = mock_connection();

        let speed = PvcamFeatures::get_fan_speed(&conn).unwrap();
        assert_eq!(speed, FanSpeed::High, "Mock fan speed should be High");

        let result = PvcamFeatures::set_fan_speed(&conn, FanSpeed::Low);
        assert!(
            result.is_ok(),
            "Setting fan speed should succeed in mock mode"
        );
    }

    #[test]
    fn speed_index_mock() {
        let conn = mock_connection();

        let index = PvcamFeatures::get_speed_index(&conn).unwrap();
        assert_eq!(index, 0);

        let result = PvcamFeatures::set_speed_index(&conn, 1);
        assert!(result.is_ok());
    }

    #[test]
    fn readout_port_mock() {
        let conn = mock_connection();

        let port = PvcamFeatures::get_readout_port(&conn).unwrap();
        assert_eq!(port, 0);

        let result = PvcamFeatures::set_readout_port(&conn, 1);
        assert!(result.is_ok());
    }

    #[test]
    fn list_speed_modes_mock() {
        let conn = mock_connection();
        let modes = PvcamFeatures::list_speed_modes(&conn).unwrap();

        assert_eq!(modes.len(), 2);
        assert_eq!(modes[0].name, "100 MHz");
        assert_eq!(modes[1].name, "50 MHz");
    }

    #[test]
    fn list_readout_ports_mock() {
        let conn = mock_connection();
        let ports = PvcamFeatures::list_readout_ports(&conn).unwrap();

        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].name, "Sensitivity");
        assert_eq!(ports[1].name, "Speed");
    }

    #[test]
    fn gain_index_mock() {
        let conn = mock_connection();

        let index = PvcamFeatures::get_gain_index(&conn).unwrap();
        assert_eq!(index, 0);

        let result = PvcamFeatures::set_gain_index(&conn, 1);
        assert!(result.is_ok());
    }

    #[test]
    fn list_gain_modes_mock() {
        let conn = mock_connection();
        let modes = PvcamFeatures::list_gain_modes(&conn).unwrap();

        assert_eq!(modes.len(), 2);
        assert_eq!(modes[0].name, "HDR");
        assert_eq!(modes[1].name, "CMS");
    }

    #[test]
    fn exposure_mode_mock() {
        let conn = mock_connection();

        let mode = PvcamFeatures::get_exposure_mode(&conn).unwrap();
        assert_eq!(mode, ExposureMode::Timed);

        let result = PvcamFeatures::set_exposure_mode(&conn, ExposureMode::Bulb);
        assert!(result.is_ok());
    }

    #[test]
    fn clear_mode_mock() {
        let conn = mock_connection();

        let mode = PvcamFeatures::get_clear_mode(&conn).unwrap();
        assert_eq!(mode, ClearMode::PreExposure);

        let result = PvcamFeatures::set_clear_mode(&conn, ClearMode::Never);
        assert!(result.is_ok());
    }

    #[test]
    fn expose_out_mode_mock() {
        let conn = mock_connection();

        let mode = PvcamFeatures::get_expose_out_mode(&conn).unwrap();
        assert_eq!(mode, ExposeOutMode::FirstRow);

        let result = PvcamFeatures::set_expose_out_mode(&conn, ExposeOutMode::AllRows);
        assert!(result.is_ok());
    }

    #[test]
    fn readout_timing_mock() {
        let conn = mock_connection();

        let readout = PvcamFeatures::get_readout_time_us(&conn).unwrap();
        assert_eq!(readout, 15000, "Mock readout time should be 15ms");

        let clearing = PvcamFeatures::get_clearing_time_us(&conn).unwrap();
        assert_eq!(clearing, 1000, "Mock clearing time should be 1ms");

        let pre_delay = PvcamFeatures::get_pre_trigger_delay_us(&conn).unwrap();
        assert_eq!(pre_delay, 0);

        let post_delay = PvcamFeatures::get_post_trigger_delay_us(&conn).unwrap();
        assert_eq!(post_delay, 0);
    }

    #[test]
    fn shutter_control_mock() {
        let conn = mock_connection();

        let status = PvcamFeatures::get_shutter_status(&conn).unwrap();
        assert_eq!(status, ShutterStatus::Closed);

        let mode = PvcamFeatures::get_shutter_mode(&conn).unwrap();
        assert_eq!(mode, ShutterMode::Normal);

        let result = PvcamFeatures::set_shutter_mode(&conn, ShutterMode::Open);
        assert!(result.is_ok());

        let open_delay = PvcamFeatures::get_shutter_open_delay_us(&conn).unwrap();
        assert_eq!(open_delay, 0);

        let close_delay = PvcamFeatures::get_shutter_close_delay_us(&conn).unwrap();
        assert_eq!(close_delay, 0);
    }

    #[test]
    fn smart_streaming_mock() {
        let conn = mock_connection();

        let enabled = PvcamFeatures::is_smart_stream_enabled(&conn).unwrap();
        assert!(
            !enabled,
            "Smart streaming should be disabled by default in mock"
        );

        let result = PvcamFeatures::set_smart_stream_enabled(&conn, true);
        assert!(result.is_ok());

        let mode = PvcamFeatures::get_smart_stream_mode(&conn).unwrap();
        assert_eq!(mode, SmartStreamMode::Exposures);

        let result = PvcamFeatures::set_smart_stream_mode(&conn, SmartStreamMode::Interleaved);
        assert!(result.is_ok());
    }

    #[test]
    fn list_pp_features_mock() {
        let conn = mock_connection();
        let features = PvcamFeatures::list_pp_features(&conn).unwrap();

        assert_eq!(features.len(), 2);
        assert_eq!(features[0].name, "PrimeEnhance");
        assert_eq!(features[1].name, "PrimeLocate");
    }

    #[test]
    fn list_pp_params_mock() {
        let conn = mock_connection();
        let params = PvcamFeatures::list_pp_params(&conn, 0).unwrap();

        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "Enabled");
        assert_eq!(params[1].name, "Threshold");
    }

    #[test]
    fn pp_param_get_set_mock() {
        let conn = mock_connection();

        let value = PvcamFeatures::get_pp_param(&conn, 0, 0).unwrap();
        assert_eq!(value, 0);

        let result = PvcamFeatures::set_pp_param(&conn, 0, 0, 1);
        assert!(result.is_ok());

        let result = PvcamFeatures::set_pp_feature_enabled(&conn, 0, true);
        assert!(result.is_ok());
    }

    #[test]
    fn binning_mock() {
        let conn = mock_connection();

        let (serial, parallel) = PvcamFeatures::get_binning(&conn).unwrap();
        assert_eq!(serial, 1);
        assert_eq!(parallel, 1);

        let serial_factors = PvcamFeatures::list_serial_binning(&conn).unwrap();
        assert_eq!(serial_factors, vec![1, 2, 4, 8]);

        let parallel_factors = PvcamFeatures::list_parallel_binning(&conn).unwrap();
        assert_eq!(parallel_factors, vec![1, 2, 4, 8]);
    }

    #[test]
    fn metadata_mock() {
        let conn = mock_connection();

        let enabled = PvcamFeatures::is_metadata_enabled(&conn).unwrap();
        assert!(!enabled);

        let result = PvcamFeatures::set_metadata_enabled(&conn, true);
        assert!(result.is_ok());
    }

    #[test]
    fn centroids_config_mock() {
        let conn = mock_connection();

        let config = PvcamFeatures::get_centroids_config(&conn).unwrap();
        assert_eq!(config.mode, CentroidsMode::Locate);
        assert_eq!(config.radius, 3);
        assert_eq!(config.max_count, 1000);
        assert_eq!(config.threshold, 100);

        let new_config = CentroidsConfig {
            mode: CentroidsMode::Track,
            radius: 5,
            max_count: 500,
            threshold: 200,
        };
        let result = PvcamFeatures::set_centroids_config(&conn, &new_config);
        assert!(result.is_ok());
    }
}

// =============================================================================
// Hardware Integration Tests (require pvcam_hardware + hardware_tests features)
// =============================================================================

#[cfg(all(feature = "pvcam_hardware", feature = "hardware_tests"))]
mod hardware_features {
    use super::*;
    use daq_driver_pvcam::components::connection::PvcamConnection;

    /// Opens a fresh camera connection for each test.
    /// This avoids mutex poisoning issues when tests panic.
    fn open_camera() -> PvcamConnection {
        let mut conn = PvcamConnection::new();
        conn.initialize().expect("Failed to initialize PVCAM SDK");
        conn.open("pvcamUSB_0").expect("Failed to open camera");
        conn
    }

    #[test]
    fn hardware_get_temperature() {
        let conn = open_camera();

        let temp = PvcamFeatures::get_temperature(&conn).unwrap();
        println!("Current temperature: {:.2}°C", temp);

        // Prime BSI typically operates between -50°C and +50°C
        assert!(
            temp > -60.0 && temp < 60.0,
            "Temperature {} out of expected range",
            temp
        );
    }

    #[test]
    fn hardware_get_camera_info() {
        let conn = open_camera();

        let info = PvcamFeatures::get_camera_info(&conn).unwrap();
        println!("Camera Info:");
        println!("  Serial: {}", info.serial_number);
        println!("  Firmware: {}", info.firmware_version);
        println!("  Chip: {}", info.chip_name);
        println!("  Temperature: {:.2}°C", info.temperature_c);
        println!("  Bit Depth: {}", info.bit_depth);
        println!("  Pixel Time: {} ns", info.pixel_time_ns);
        println!("  Pixel Size: {:?} nm", info.pixel_size_nm);
        println!("  Sensor Size: {:?}", info.sensor_size);
        println!("  Gain: {} (index {})", info.gain_name, info.gain_index);
        println!("  Speed: {} (index {})", info.speed_name, info.speed_index);
        println!("  Port: {}", info.port_name);

        // Prime BSI specific validations
        assert!(info.chip_name.contains("GS2020") || info.chip_name.len() > 0);
        assert_eq!(
            info.sensor_size,
            (2048, 2048),
            "Prime BSI should be 2048x2048"
        );
    }

    #[test]
    fn hardware_fan_speed_cycle() {
        let conn = open_camera();

        // Get initial fan speed
        let initial = PvcamFeatures::get_fan_speed(&conn).unwrap();
        println!("Initial fan speed: {:?}", initial);

        // Try setting different speeds - some may be restricted
        for speed in [FanSpeed::High, FanSpeed::Medium, FanSpeed::Low] {
            match PvcamFeatures::set_fan_speed(&conn, speed) {
                Ok(()) => {
                    let current = PvcamFeatures::get_fan_speed(&conn).unwrap();
                    println!("Set to {:?}, read back {:?}", speed, current);
                }
                Err(e) => {
                    println!("Could not set fan speed to {:?}: {}", speed, e);
                }
            }
        }

        // Try to restore initial
        let _ = PvcamFeatures::set_fan_speed(&conn, initial);
    }

    #[test]
    fn hardware_list_speed_modes() {
        let conn = open_camera();

        let modes = PvcamFeatures::list_speed_modes(&conn).unwrap();
        println!("Available speed modes:");
        for mode in &modes {
            println!(
                "  [{}] {} - {} ns/pixel, {} bit, port {}",
                mode.index, mode.name, mode.pixel_time_ns, mode.bit_depth, mode.port_index
            );
        }

        // May be empty if not enumerable
        println!("Total speed modes: {}", modes.len());
    }

    #[test]
    fn hardware_list_readout_ports() {
        let conn = open_camera();

        let ports = PvcamFeatures::list_readout_ports(&conn).unwrap();
        println!("Available readout ports:");
        for port in &ports {
            println!("  [{}] {}", port.index, port.name);
        }

        println!("Total readout ports: {}", ports.len());
    }

    #[test]
    fn hardware_list_gain_modes() {
        let conn = open_camera();

        let modes = PvcamFeatures::list_gain_modes(&conn).unwrap();
        println!("Available gain modes:");
        for mode in &modes {
            println!("  [{}] {}", mode.index, mode.name);
        }

        println!("Total gain modes: {}", modes.len());
    }

    #[test]
    fn hardware_exposure_mode() {
        let conn = open_camera();

        let mode = PvcamFeatures::get_exposure_mode(&conn).unwrap();
        println!("Current exposure mode: {:?}", mode);

        // Try setting exposure mode - may be access denied on some cameras
        match PvcamFeatures::set_exposure_mode(&conn, ExposureMode::Timed) {
            Ok(()) => {
                let current = PvcamFeatures::get_exposure_mode(&conn).unwrap();
                println!("Set exposure mode to Timed, read back: {:?}", current);
            }
            Err(e) => {
                println!("Could not set exposure mode (may be read-only): {}", e);
            }
        }
    }

    #[test]
    fn hardware_clear_mode() {
        let conn = open_camera();

        let mode = PvcamFeatures::get_clear_mode(&conn).unwrap();
        println!("Current clear mode: {:?}", mode);
    }

    #[test]
    fn hardware_readout_timing() {
        let conn = open_camera();

        let readout = PvcamFeatures::get_readout_time_us(&conn).unwrap();
        println!(
            "Readout time: {} µs ({:.2} ms)",
            readout,
            readout as f64 / 1000.0
        );

        // Some timing parameters may require acquisition setup first
        match PvcamFeatures::get_clearing_time_us(&conn) {
            Ok(clearing) => println!("Clearing time: {} µs", clearing),
            Err(e) => println!(
                "Clearing time not available (may need acquisition setup): {}",
                e
            ),
        }

        match PvcamFeatures::get_pre_trigger_delay_us(&conn) {
            Ok(delay) => println!("Pre-trigger delay: {} µs", delay),
            Err(e) => println!("Pre-trigger delay not available: {}", e),
        }

        match PvcamFeatures::get_post_trigger_delay_us(&conn) {
            Ok(delay) => println!("Post-trigger delay: {} µs", delay),
            Err(e) => println!("Post-trigger delay not available: {}", e),
        }

        // Readout should be reasonable (0-100 ms for full frame)
        // Note: May be 0 if not configured
        assert!(
            readout < 200_000,
            "Readout time {} unexpectedly high",
            readout
        );
    }

    #[test]
    fn hardware_shutter_control() {
        let conn = open_camera();

        let status = PvcamFeatures::get_shutter_status(&conn).unwrap();
        println!("Shutter status: {:?}", status);

        let mode = PvcamFeatures::get_shutter_mode(&conn).unwrap();
        println!("Shutter mode: {:?}", mode);

        let open_delay = PvcamFeatures::get_shutter_open_delay_us(&conn).unwrap();
        println!("Shutter open delay: {} µs", open_delay);

        let close_delay = PvcamFeatures::get_shutter_close_delay_us(&conn).unwrap();
        println!("Shutter close delay: {} µs", close_delay);
    }

    #[test]
    fn hardware_smart_streaming() {
        let conn = open_camera();

        let enabled = PvcamFeatures::is_smart_stream_enabled(&conn).unwrap();
        println!("Smart streaming enabled: {}", enabled);

        match PvcamFeatures::get_smart_stream_mode(&conn) {
            Ok(mode) => println!("Smart streaming mode: {:?}", mode),
            Err(e) => println!("Could not get smart streaming mode: {}", e),
        }
    }

    #[test]
    fn hardware_pp_features() {
        let conn = open_camera();

        let features = PvcamFeatures::list_pp_features(&conn).unwrap();
        println!("Post-processing features:");
        for feature in &features {
            println!(
                "  [{}] {} (ID: {})",
                feature.index, feature.name, feature.id
            );

            // List params for this feature
            if let Ok(params) = PvcamFeatures::list_pp_params(&conn, feature.index) {
                for param in params {
                    println!(
                        "    - [{}] {} = {} (ID: {})",
                        param.index, param.name, param.value, param.id
                    );
                }
            }
        }
        println!("Total PP features: {}", features.len());
    }

    #[test]
    fn hardware_binning() {
        let conn = open_camera();

        let (serial, parallel) = PvcamFeatures::get_binning(&conn).unwrap();
        println!("Current binning: {}x{}", serial, parallel);

        let serial_factors = PvcamFeatures::list_serial_binning(&conn).unwrap();
        println!("Available serial binning: {:?}", serial_factors);

        let parallel_factors = PvcamFeatures::list_parallel_binning(&conn).unwrap();
        println!("Available parallel binning: {:?}", parallel_factors);
    }

    #[test]
    fn hardware_metadata() {
        let conn = open_camera();

        let enabled = PvcamFeatures::is_metadata_enabled(&conn).unwrap();
        println!("Metadata enabled: {}", enabled);
    }
}
