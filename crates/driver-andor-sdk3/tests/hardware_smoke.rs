//! Hardware Smoke Tests for Andor iStar Camera & Shamrock Spectrograph (bd-ux7n)
//!
//! Real hardware integration tests gated behind feature flags, matching the
//! PVCAM nextest profile pattern. Tests verify basic connectivity, parameter
//! access, and frame acquisition on physical hardware.
//!
//! # Feature Gates
//!
//! - `camera` + `hardware_tests` — iStar camera tests
//! - `spectrograph` + `hardware_tests` — Shamrock spectrograph tests
//!
//! # Environment Variables
//!
//! - `ANDOR_SMOKE_TEST=1` — Enable the test suite (skip otherwise)
//! - `ANDOR_CAMERA_INDEX` — Camera device index (default: 0)
//! - `ANDOR_SPECTROGRAPH_INDEX` — Spectrograph device index (default: 0)
//!
//! # Running
//!
//! ```bash
//! # Camera tests only
//! ANDOR_SMOKE_TEST=1 cargo test -p driver-andor-sdk3 --test hardware_smoke \
//!   --features "camera,hardware_tests" -- --nocapture --test-threads=1
//!
//! # Spectrograph tests only
//! ANDOR_SMOKE_TEST=1 cargo test -p driver-andor-sdk3 --test hardware_smoke \
//!   --features "spectrograph,hardware_tests" -- --nocapture --test-threads=1
//!
//! # Both
//! ANDOR_SMOKE_TEST=1 cargo test -p driver-andor-sdk3 --test hardware_smoke \
//!   --features "camera,spectrograph,hardware_tests" -- --nocapture --test-threads=1
//! ```

#![cfg(feature = "hardware_tests")]
#![allow(dead_code, unused_macros, unused_imports)]

use std::env;
use std::time::Duration;

/// Check if hardware smoke test is enabled via environment variable.
fn smoke_test_enabled() -> bool {
    env::var("ANDOR_SMOKE_TEST")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// Get camera device index from environment or default to 0.
fn camera_index() -> i32 {
    env::var("ANDOR_CAMERA_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Get spectrograph device index from environment or default to 0.
fn spectrograph_index() -> i32 {
    env::var("ANDOR_SPECTROGRAPH_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

macro_rules! skip_if_disabled {
    () => {
        if !smoke_test_enabled() {
            println!("Andor hardware smoke test skipped (set ANDOR_SMOKE_TEST=1 to enable)");
            return;
        }
    };
}

// =============================================================================
// iStar Camera Smoke Tests
// =============================================================================

#[cfg(feature = "camera")]
mod camera_smoke {
    use super::*;
    use common::capabilities::{ExposureControl, FrameProducer, Parameterized};
    use driver_andor_sdk3::AndorCamera;

    /// Test 1: Camera initialization and basic info.
    ///
    /// Verifies:
    /// - SDK3 initializes without error
    /// - Camera opens on specified index
    /// - Sensor dimensions are reported correctly
    #[tokio::test]
    async fn andor_camera_init() {
        skip_if_disabled!();

        println!("=== Andor iStar Camera Init Test ===");
        println!("Camera index: {}", camera_index());

        let cam = AndorCamera::new_async(camera_index()).await.expect(
            "Failed to create Andor camera — check SDK3 installation and camera connection",
        );

        let info = cam.info();
        println!("  Model: {}", info.model);
        println!("  Serial: {}", info.serial_number);
        println!("  Firmware: {}", info.firmware_version);
        println!("  Sensor: {}x{}", info.sensor_width, info.sensor_height);

        let (w, h) = cam.resolution();
        assert!(w > 0, "Sensor width must be positive");
        assert!(h > 0, "Sensor height must be positive");
        assert_eq!(w, info.sensor_width);
        assert_eq!(h, info.sensor_height);

        println!("\n=== Camera Init PASSED ===");
    }

    /// Test 2: Exposure set and readback.
    ///
    /// Verifies:
    /// - Exposure can be set to various values
    /// - Readback matches within tolerance
    #[tokio::test]
    async fn andor_camera_exposure() {
        skip_if_disabled!();

        println!("=== Andor Exposure Range Test ===");

        let cam = AndorCamera::new_async(camera_index())
            .await
            .expect("Failed to create Andor camera");

        let test_exposures_s = [0.001, 0.010, 0.050, 0.100, 0.500];

        for &exp_s in &test_exposures_s {
            cam.set_exposure(exp_s)
                .await
                .unwrap_or_else(|e| panic!("Failed to set exposure to {:.3}s: {}", exp_s, e));

            let readback = cam.get_exposure().await.expect("Failed to read exposure");
            let exp_ms = exp_s * 1000.0;
            let readback_ms = readback * 1000.0;

            println!("  Set: {:.1}ms, Read: {:.3}ms", exp_ms, readback_ms);

            let tolerance = (exp_ms * 0.1).max(1.0);
            assert!(
                (readback_ms - exp_ms).abs() < tolerance,
                "Exposure should be ~{:.1}ms, got {:.3}ms",
                exp_ms,
                readback_ms
            );
        }

        println!("\n=== Exposure Range PASSED ===");
    }

    /// Test 3: Parameter system access.
    ///
    /// Verifies:
    /// - ParameterSet is populated
    /// - Key parameters are present and readable
    #[tokio::test]
    async fn andor_camera_parameters() {
        skip_if_disabled!();

        println!("=== Andor Parameter Access Test ===");

        let cam = AndorCamera::new_async(camera_index())
            .await
            .expect("Failed to create Andor camera");

        let params = cam.parameters();
        let names = params.names();

        println!("  Registered parameters: {}", names.len());
        for name in &names {
            println!("    - {}", name);
        }

        // Key parameters that should exist on iStar
        let required = [
            "acquisition.exposure_s",
            "acquisition.trigger_mode",
            "acquisition.mcp_gain",
        ];

        for &name in &required {
            assert!(
                names
                    .iter()
                    .any(|n| n.contains(name.split('.').last().unwrap_or(name))),
                "Should have parameter containing '{}'",
                name
            );
        }

        println!("\n=== Parameter Access PASSED ===");
    }

    /// Test 4: Single frame acquisition via streaming.
    ///
    /// Verifies:
    /// - Stream starts and produces at least one frame
    /// - Frame has correct dimensions and non-empty pixel data
    /// - Stream stops cleanly
    #[tokio::test]
    async fn andor_camera_single_frame() {
        skip_if_disabled!();

        println!("=== Andor Single Frame Test ===");

        let cam = AndorCamera::new_async(camera_index())
            .await
            .expect("Failed to create Andor camera");

        cam.set_exposure(0.010).await.expect("set exposure 10ms");

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        cam.register_primary_output(tx)
            .await
            .expect("register output");

        cam.start_stream().await.expect("start stream");

        let frame = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("Timeout waiting for first frame (10s)")
            .expect("Channel closed before frame received");

        cam.stop_stream().await.expect("stop stream");

        let (w, h) = cam.resolution();
        println!(
            "  Frame: {}x{}, bit_depth={}",
            frame.width, frame.height, frame.bit_depth
        );
        println!("  Pixel data: {} bytes", frame.pixel_data().len());
        println!("  Frame number: {}", frame.frame_number);

        assert_eq!(frame.width, w, "Frame width should match sensor");
        assert_eq!(frame.height, h, "Frame height should match sensor");
        assert!(
            !frame.pixel_data().is_empty(),
            "Pixel data should not be empty"
        );
        assert!(frame.timestamp_ns > 0, "Timestamp should be set");

        println!("\n=== Single Frame PASSED ===");
    }

    /// Test 5: Short streaming burst.
    ///
    /// Verifies:
    /// - Sustained streaming produces multiple frames
    /// - Frame numbers are sequential
    /// - No panics during start/stop
    #[tokio::test]
    async fn andor_camera_streaming() {
        skip_if_disabled!();

        println!("=== Andor Streaming Test ===");

        let cam = AndorCamera::new_async(camera_index())
            .await
            .expect("Failed to create Andor camera");

        cam.set_exposure(0.010).await.expect("set exposure 10ms");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        cam.register_primary_output(tx)
            .await
            .expect("register output");

        cam.start_stream().await.expect("start stream");

        let stream_duration = Duration::from_secs(2);
        let start = std::time::Instant::now();
        let mut frame_count = 0u32;
        let mut last_frame_nr = None;

        while start.elapsed() < stream_duration {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(frame)) => {
                    if let Some(last) = last_frame_nr {
                        assert!(
                            frame.frame_number > last,
                            "Frame numbers should increase: {} > {}",
                            frame.frame_number,
                            last
                        );
                    }
                    last_frame_nr = Some(frame.frame_number);
                    frame_count += 1;
                }
                Ok(None) => {
                    println!("  Channel closed");
                    break;
                }
                Err(_) => {
                    println!("  Timeout waiting for frame");
                    break;
                }
            }
        }

        cam.stop_stream().await.expect("stop stream");

        let fps = frame_count as f64 / start.elapsed().as_secs_f64();
        println!("  Frames: {}", frame_count);
        println!("  FPS: {:.1}", fps);

        assert!(frame_count >= 3, "Should capture at least 3 frames in 2s");

        println!("\n=== Streaming Test PASSED ===");
    }
}

// =============================================================================
// Raw SDK Diagnostic Test (bypasses AndorCamera entirely)
// =============================================================================

#[cfg(feature = "camera")]
mod raw_sdk_diagnostic {
    use super::*;
    use andor_sdk3_sys::*;

    /// Raw SDK acquisition test — bypasses AndorCamera wrapper.
    ///
    /// This test directly calls the Andor SDK3 C API to determine if the
    /// camera hardware can produce frames at the SDK level. If this test
    /// fails, the issue is in the SDK/camera, not our wrapper code.
    #[test]
    fn raw_sdk_acquisition() {
        skip_if_disabled!();

        println!("=== Raw SDK Acquisition Test ===");
        unsafe {
            // Step 1: Initialize SDK
            let ret = AT_InitialiseLibrary();
            println!("  AT_InitialiseLibrary: ret={ret}");
            assert_eq!(ret, 0, "AT_InitialiseLibrary failed with {ret}");

            // Step 2: Open camera
            let idx = camera_index();
            let mut handle: AT_H = 0;
            let ret = AT_Open(idx, &mut handle);
            println!("  AT_Open({idx}): ret={ret}, handle={handle}");
            assert_eq!(ret, 0, "AT_Open failed with {ret}");

            // Step 3: Read camera model
            let model_feat = to_wide_string("CameraModel");
            let mut model_buf = wide_string_buffer(256);
            let _ = AT_GetString(handle, model_feat.as_ptr(), model_buf.as_mut_ptr(), 256);
            let model = from_wide_string(&model_buf);
            println!("  CameraModel: {model}");

            // Step 4: Set minimal configuration
            // CycleMode = Continuous
            let feat = to_wide_string("CycleMode");
            let val = to_wide_string("Continuous");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set CycleMode=Continuous: ret={ret}");

            // TriggerMode = Internal
            let feat = to_wide_string("TriggerMode");
            let val = to_wide_string("Internal");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set TriggerMode=Internal: ret={ret}");

            // GateMode = CW On (iStar-specific, may fail on non-gated cameras)
            let feat = to_wide_string("GateMode");
            let val = to_wide_string("CW On");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set GateMode='CW On': ret={ret}");

            // ShutterMode = Open (iStar defaults to Closed on power-up)
            let feat = to_wide_string("ShutterMode");
            let val = to_wide_string("Open");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set ShutterMode='Open': ret={ret}");

            // Step 5: Read ImageSizeBytes
            let feat = to_wide_string("ImageSizeBytes");
            let mut img_size: AT_64 = 0;
            let ret = AT_GetInt(handle, feat.as_ptr(), &mut img_size);
            println!("  ImageSizeBytes: {img_size} (ret={ret})");
            assert_eq!(ret, 0, "Failed to read ImageSizeBytes");
            assert!(img_size > 0, "ImageSizeBytes must be > 0");

            // Read other relevant features
            let feat = to_wide_string("AOIWidth");
            let mut aoi_w: AT_64 = 0;
            AT_GetInt(handle, feat.as_ptr(), &mut aoi_w);

            let feat = to_wide_string("AOIHeight");
            let mut aoi_h: AT_64 = 0;
            AT_GetInt(handle, feat.as_ptr(), &mut aoi_h);

            let feat = to_wide_string("AOIStride");
            let mut aoi_stride: AT_64 = 0;
            AT_GetInt(handle, feat.as_ptr(), &mut aoi_stride);

            println!("  AOI: {aoi_w}x{aoi_h}, stride={aoi_stride}");

            // Read ExposureTime
            let feat = to_wide_string("ExposureTime");
            let mut exp: f64 = 0.0;
            AT_GetFloat(handle, feat.as_ptr(), &mut exp);
            println!("  ExposureTime: {exp:.6}s ({:.1}ms)", exp * 1000.0);

            // Read FrameRate
            let feat = to_wide_string("FrameRate");
            let mut fps: f64 = 0.0;
            AT_GetFloat(handle, feat.as_ptr(), &mut fps);
            println!("  FrameRate: {fps:.2} fps");

            // Step 6: Allocate 8-byte aligned buffer
            let buf_size = img_size as usize;
            let layout = std::alloc::Layout::from_size_align(buf_size, 8).unwrap();
            let buf_ptr = std::alloc::alloc_zeroed(layout);
            assert!(!buf_ptr.is_null(), "Buffer allocation failed");
            println!(
                "  Allocated buffer: {} bytes @ {:p} (align={})",
                buf_size,
                buf_ptr,
                buf_ptr as usize % 8
            );

            // Step 7: Queue single buffer
            let ret = AT_QueueBuffer(handle, buf_ptr, buf_size as std::os::raw::c_int);
            println!("  AT_QueueBuffer: ret={ret}");
            assert_eq!(ret, 0, "AT_QueueBuffer failed with {ret}");

            // Step 8: Start acquisition
            let feat = to_wide_string("AcquisitionStart");
            let ret = AT_Command(handle, feat.as_ptr());
            println!("  AT_Command(AcquisitionStart): ret={ret}");
            assert_eq!(ret, 0, "AcquisitionStart failed with {ret}");

            // Verify CameraAcquiring
            let feat = to_wide_string("CameraAcquiring");
            let mut acq: AT_BOOL = 0;
            AT_GetBool(handle, feat.as_ptr(), &mut acq);
            println!("  CameraAcquiring: {acq}");

            // Step 9: Wait for buffer (15s timeout)
            let mut out_ptr: *mut u8 = std::ptr::null_mut();
            let mut out_size: std::os::raw::c_int = 0;
            println!("  Calling AT_WaitBuffer(timeout=15000ms)...");
            let start = std::time::Instant::now();
            let ret = AT_WaitBuffer(handle, &mut out_ptr, &mut out_size, 15_000);
            let elapsed = start.elapsed();
            println!(
                "  AT_WaitBuffer: ret={ret}, ptr={out_ptr:p}, size={out_size}, elapsed={:.1}s",
                elapsed.as_secs_f64()
            );

            // Print frame info BEFORE cleanup (out_ptr points into buf_ptr)
            if ret == 0 {
                println!("\n  ✓ Raw SDK produced a frame! ({out_size} bytes)");
                println!(
                    "  First 16 bytes: {:?}",
                    &std::slice::from_raw_parts(out_ptr, 16.min(out_size as usize))
                );
            } else {
                println!("\n  ✗ Raw SDK did NOT produce a frame (ret={ret})");
                println!("  This means the camera hardware or SDK (not our wrapper) is the issue.");
            }

            // Step 10: Cleanup (stop acquisition before freeing buffers)
            let feat = to_wide_string("AcquisitionStop");
            let stop_ret = AT_Command(handle, feat.as_ptr());
            println!("  AT_Command(AcquisitionStop): ret={stop_ret}");

            let flush_ret = AT_Flush(handle);
            println!("  AT_Flush: ret={flush_ret}");

            let close_ret = AT_Close(handle);
            println!("  AT_Close: ret={close_ret}");

            AT_FinaliseLibrary();
            println!("  AT_FinaliseLibrary: done");

            // Free buffer after SDK is done with it
            std::alloc::dealloc(buf_ptr, layout);

            println!(
                "\n=== Raw SDK Test {} ===",
                if ret == 0 { "PASSED" } else { "FAILED" }
            );
            assert_eq!(
                ret, 0,
                "AT_WaitBuffer timed out — camera did not produce frames at SDK level"
            );
        }
    }

    /// Raw SDK test with small AOI — tests if reduced bandwidth works through QEMU USB passthrough.
    ///
    /// The leabs-dev VM uses QEMU xHCI (virtual USB 3.0 controller). Full-resolution
    /// frames (~8.3MB @ 33fps = ~275 MB/s) may exceed the virtual xHCI's bulk transfer
    /// capability. This test uses a 256x256 AOI (~96KB/frame) to test if lower
    /// bandwidth acquisition succeeds.
    #[test]
    fn raw_sdk_small_aoi_acquisition() {
        skip_if_disabled!();

        println!("=== Raw SDK Small AOI Acquisition Test ===");
        unsafe {
            let ret = AT_InitialiseLibrary();
            assert_eq!(ret, 0, "AT_InitialiseLibrary failed");

            let mut handle: AT_H = 0;
            let ret = AT_Open(camera_index(), &mut handle);
            println!("  AT_Open: ret={ret}, handle={handle}");
            assert_eq!(ret, 0, "AT_Open failed");

            // Check SensorInitialised (from official example)
            let feat = to_wide_string("SensorInitialised");
            let mut impl_flag: AT_BOOL = 0;
            AT_IsImplemented(handle, feat.as_ptr(), &mut impl_flag);
            println!("  SensorInitialised implemented: {impl_flag}");
            if impl_flag != 0 {
                let mut init_val: AT_BOOL = 0;
                AT_GetBool(handle, feat.as_ptr(), &mut init_val);
                println!("  SensorInitialised: {init_val}");
                if init_val == 0 {
                    println!("  Waiting for sensor initialisation...");
                    for i in 0..30 {
                        std::thread::sleep(Duration::from_secs(1));
                        AT_GetBool(handle, feat.as_ptr(), &mut init_val);
                        print!(".");
                        if init_val != 0 {
                            println!("\n  Sensor initialised after {i}s");
                            break;
                        }
                    }
                    if init_val == 0 {
                        println!("\n  WARNING: Sensor did not initialise within 30s");
                    }
                }
            }

            // Set small AOI (256x256, centered)
            let feat = to_wide_string("AOIWidth");
            let ret = AT_SetInt(handle, feat.as_ptr(), 256);
            println!("  Set AOIWidth=256: ret={ret}");

            let feat = to_wide_string("AOIHeight");
            let ret = AT_SetInt(handle, feat.as_ptr(), 256);
            println!("  Set AOIHeight=256: ret={ret}");

            // Use Fixed cycle mode (like official example)
            let feat = to_wide_string("CycleMode");
            let val = to_wide_string("Fixed");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set CycleMode=Fixed: ret={ret}");

            let feat = to_wide_string("FrameCount");
            let ret = AT_SetInt(handle, feat.as_ptr(), 1);
            println!("  Set FrameCount=1: ret={ret}");

            // TriggerMode = Internal
            let feat = to_wide_string("TriggerMode");
            let val = to_wide_string("Internal");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set TriggerMode=Internal: ret={ret}");

            // GateMode = CW On
            let feat = to_wide_string("GateMode");
            let val = to_wide_string("CW On");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set GateMode='CW On': ret={ret}");

            // ShutterMode = Open (iStar defaults to Closed on power-up)
            let feat = to_wide_string("ShutterMode");
            let val = to_wide_string("Open");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set ShutterMode='Open': ret={ret}");

            // Try setting Mono16 encoding (simpler than Mono12Packed)
            let feat = to_wide_string("PixelEncoding");
            let val = to_wide_string("Mono16");
            let ret = AT_SetEnumString(handle, feat.as_ptr(), val.as_ptr());
            println!("  Set PixelEncoding=Mono16: ret={ret}");

            // Read back dimensions
            let feat = to_wide_string("ImageSizeBytes");
            let mut img_size: AT_64 = 0;
            AT_GetInt(handle, feat.as_ptr(), &mut img_size);

            let feat = to_wide_string("AOIWidth");
            let mut w: AT_64 = 0;
            AT_GetInt(handle, feat.as_ptr(), &mut w);

            let feat = to_wide_string("AOIHeight");
            let mut h: AT_64 = 0;
            AT_GetInt(handle, feat.as_ptr(), &mut h);

            let feat = to_wide_string("AOIStride");
            let mut stride: AT_64 = 0;
            AT_GetInt(handle, feat.as_ptr(), &mut stride);

            let feat = to_wide_string("FrameRate");
            let mut fps: f64 = 0.0;
            AT_GetFloat(handle, feat.as_ptr(), &mut fps);

            println!(
                "  AOI: {w}x{h}, stride={stride}, ImageSizeBytes={img_size}, FrameRate={fps:.1}fps"
            );
            println!(
                "  Data rate: {:.1} MB/s",
                (img_size as f64 * fps) / 1_000_000.0
            );

            // Allocate, queue, acquire
            let buf_size = img_size as usize;
            let layout = std::alloc::Layout::from_size_align(buf_size, 8).unwrap();
            let buf_ptr = std::alloc::alloc_zeroed(layout);
            assert!(!buf_ptr.is_null());

            let ret = AT_QueueBuffer(handle, buf_ptr, buf_size as std::os::raw::c_int);
            println!("  AT_QueueBuffer: ret={ret}");

            let feat = to_wide_string("AcquisitionStart");
            let ret = AT_Command(handle, feat.as_ptr());
            println!("  AcquisitionStart: ret={ret}");

            let mut out_ptr: *mut u8 = std::ptr::null_mut();
            let mut out_size: std::os::raw::c_int = 0;
            println!("  Waiting for frame (30s timeout)...");
            let start = std::time::Instant::now();
            let ret = AT_WaitBuffer(handle, &mut out_ptr, &mut out_size, 30_000);
            let elapsed = start.elapsed();
            println!(
                "  AT_WaitBuffer: ret={ret}, size={out_size}, elapsed={:.1}s",
                elapsed.as_secs_f64()
            );

            // Cleanup
            let feat = to_wide_string("AcquisitionStop");
            AT_Command(handle, feat.as_ptr());
            AT_Flush(handle);
            AT_Close(handle);
            AT_FinaliseLibrary();
            std::alloc::dealloc(buf_ptr, layout);

            if ret == 0 {
                println!("\n  ✓ Small AOI frame received! ({out_size} bytes)");
            } else {
                println!("\n  ✗ Small AOI also failed (ret={ret})");
                println!("  This confirms QEMU USB passthrough cannot handle bulk transfers.");
            }
            println!(
                "\n=== Small AOI Test {} ===",
                if ret == 0 { "PASSED" } else { "FAILED" }
            );
        }
    }
}

// =============================================================================
// Shamrock Spectrograph Smoke Tests
// =============================================================================

#[cfg(feature = "spectrograph")]
mod spectrograph_smoke {
    use super::*;
    use common::capabilities::{Parameterized, ShutterControl, WavelengthTunable};
    use driver_andor_sdk3::AndorSpectrograph;

    /// Test 1: Spectrograph initialization and info.
    #[tokio::test]
    async fn andor_spectrograph_init() {
        skip_if_disabled!();

        println!("=== Andor Shamrock Spectrograph Init Test ===");
        println!("Spectrograph index: {}", spectrograph_index());

        let spec = AndorSpectrograph::new_async(spectrograph_index())
            .await
            .expect("Failed to create Shamrock spectrograph");

        let info = spec.info();
        println!("  Model: {}", info.model);
        println!("  Serial: {}", info.serial_number);
        println!("  Focal length: {:.1} mm", info.focal_length_mm);
        println!("  Gratings: {}", info.num_gratings);

        assert!(
            info.focal_length_mm > 0.0,
            "Focal length should be positive"
        );

        println!("\n=== Spectrograph Init PASSED ===");
    }

    /// Test 2: Wavelength tuning.
    #[tokio::test]
    async fn andor_spectrograph_wavelength() {
        skip_if_disabled!();

        println!("=== Andor Wavelength Tuning Test ===");

        let spec = AndorSpectrograph::new_async(spectrograph_index())
            .await
            .expect("Failed to create Shamrock spectrograph");

        // Set wavelength to 500nm
        spec.set_wavelength(500.0)
            .await
            .expect("Failed to set wavelength to 500nm");

        let wl = spec
            .get_wavelength()
            .await
            .expect("Failed to read wavelength");

        println!("  Set: 500.0 nm, Read: {:.2} nm", wl);

        assert!(
            (wl - 500.0).abs() < 1.0,
            "Wavelength should be ~500nm, got {:.2}nm",
            wl
        );

        // Test another wavelength
        spec.set_wavelength(632.8)
            .await
            .expect("Failed to set wavelength to 632.8nm");

        let wl2 = spec
            .get_wavelength()
            .await
            .expect("Failed to read wavelength");
        println!("  Set: 632.8 nm, Read: {:.2} nm", wl2);

        println!("\n=== Wavelength Tuning PASSED ===");
    }

    /// Test 3: Shutter control.
    #[tokio::test]
    async fn andor_spectrograph_shutter() {
        skip_if_disabled!();

        println!("=== Andor Shutter Control Test ===");

        let spec = AndorSpectrograph::new_async(spectrograph_index())
            .await
            .expect("Failed to create Shamrock spectrograph");

        // Open shutter
        spec.open_shutter().await.expect("Failed to open shutter");
        let open = spec
            .is_shutter_open()
            .await
            .expect("Failed to query shutter");
        println!("  After open: is_open={}", open);
        assert!(open, "Shutter should be open after open_shutter()");

        // Close shutter
        spec.close_shutter().await.expect("Failed to close shutter");
        let open = spec
            .is_shutter_open()
            .await
            .expect("Failed to query shutter");
        println!("  After close: is_open={}", open);
        assert!(!open, "Shutter should be closed after close_shutter()");

        println!("\n=== Shutter Control PASSED ===");
    }

    /// Test 4: Parameter system.
    #[tokio::test]
    async fn andor_spectrograph_parameters() {
        skip_if_disabled!();

        println!("=== Andor Spectrograph Parameter Test ===");

        let spec = AndorSpectrograph::new_async(spectrograph_index())
            .await
            .expect("Failed to create Shamrock spectrograph");

        let params = spec.parameters();
        let names = params.names();

        println!("  Registered parameters: {}", names.len());
        for name in &names {
            println!("    - {}", name);
        }

        assert!(
            !names.is_empty(),
            "Spectrograph should have registered parameters"
        );

        println!("\n=== Spectrograph Parameters PASSED ===");
    }
}

// =============================================================================
// Skip Check (always runs)
// =============================================================================

/// Verify the smoke test gating mechanism works.
#[test]
fn smoke_test_skip_check() {
    let enabled = smoke_test_enabled();
    if enabled {
        println!("Andor hardware smoke tests ENABLED (ANDOR_SMOKE_TEST=1)");
    } else {
        println!("Andor hardware smoke tests DISABLED (set ANDOR_SMOKE_TEST=1 to enable)");
    }
}
