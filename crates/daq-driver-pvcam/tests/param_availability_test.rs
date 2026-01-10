//! PVCAM Parameter Availability Test (bd-wwn0 epic)
//!
//! Tests the `PvcamFeatures::is_param_available` function against real hardware
//! to verify parameter availability checks work correctly.
//!
//! This test is informational - it discovers which parameters are available
//! rather than asserting specific expectations (optional parameters may vary
//! by camera model and firmware).
//!
//! Run with:
//! ```bash
//! ssh maitai@100.117.5.12 'source /etc/profile.d/pvcam.sh && \
//!   export LIBRARY_PATH=/opt/pvcam/library/x86_64:$LIBRARY_PATH && \
//!   export LD_LIBRARY_PATH=/opt/pvcam/library/x86_64:$LD_LIBRARY_PATH && \
//!   cd ~/rust-daq && cargo nextest run -p daq-driver-pvcam \
//!     --features pvcam_hardware --test param_availability_test -- --nocapture'
//! ```

#![cfg(feature = "pvcam_hardware")]
#![cfg(not(target_arch = "wasm32"))]

use once_cell::sync::Lazy;
use pvcam_sys::*;
use std::ffi::{c_void, CStr};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

// Initialize tracing subscriber once for test logging
static TRACING: Lazy<()> = Lazy::new(|| {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
});

/// Get PVCAM error message for the last error
fn get_error_message() -> String {
    let mut msg = [0i8; 256];
    unsafe {
        let code = pl_error_code();
        pl_error_message(code, msg.as_mut_ptr());
        CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned()
    }
}

/// Check if a parameter is available using ATTR_AVAIL
/// This mirrors PvcamFeatures::is_param_available
fn is_param_available(hcam: i16, param_id: u32) -> bool {
    let mut avail: rs_bool = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_AVAIL as i16,
            &mut avail as *mut _ as *mut c_void,
        ) != 0
        {
            avail != 0
        } else {
            false
        }
    }
}

/// Try to read a parameter as i32, return None if unavailable or read fails
fn try_read_i32(hcam: i16, param_id: u32) -> Option<i32> {
    let mut value: i32 = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_CURRENT as i16,
            &mut value as *mut _ as *mut c_void,
        ) != 0
        {
            Some(value)
        } else {
            None
        }
    }
}

/// Try to read a parameter as u16, return None if unavailable or read fails
fn try_read_u16(hcam: i16, param_id: u32) -> Option<u16> {
    let mut value: uns16 = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_CURRENT as i16,
            &mut value as *mut _ as *mut c_void,
        ) != 0
        {
            Some(value as u16)
        } else {
            None
        }
    }
}

/// Try to read a parameter as signed i16 (for temperature values), return None if fails
fn try_read_i16(hcam: i16, param_id: u32) -> Option<i16> {
    let mut value: i16 = 0;
    unsafe {
        if pl_get_param(
            hcam,
            param_id,
            ATTR_CURRENT as i16,
            &mut value as *mut _ as *mut c_void,
        ) != 0
        {
            Some(value)
        } else {
            None
        }
    }
}

/// Open the first available camera
fn open_first_camera() -> Option<i16> {
    unsafe {
        if pl_pvcam_init() == 0 {
            eprintln!("pl_pvcam_init failed: {}", get_error_message());
            return None;
        }

        let mut cam_count: i16 = 0;
        if pl_cam_get_total(&mut cam_count) == 0 || cam_count == 0 {
            eprintln!("No cameras found");
            pl_pvcam_uninit();
            return None;
        }

        let mut name = [0i8; PARAM_NAME_LEN as usize];
        if pl_cam_get_name(0, name.as_mut_ptr()) == 0 {
            eprintln!("pl_cam_get_name failed: {}", get_error_message());
            pl_pvcam_uninit();
            return None;
        }

        let mut hcam: i16 = 0;
        if pl_cam_open(name.as_mut_ptr(), &mut hcam, 0) == 0 {
            eprintln!("pl_cam_open failed: {}", get_error_message());
            pl_pvcam_uninit();
            return None;
        }

        Some(hcam)
    }
}

/// Close camera and uninit SDK
fn close_camera(hcam: i16) {
    unsafe {
        pl_cam_close(hcam);
        pl_pvcam_uninit();
    }
}

/// Parameter test result
#[derive(Debug)]
struct ParamTestResult {
    name: &'static str,
    #[allow(dead_code)]
    param_id: u32,
    available: bool,
    read_value: Option<String>,
    read_error: Option<String>,
}

/// Test a parameter's availability and readability
fn test_parameter(hcam: i16, name: &'static str, param_id: u32) -> ParamTestResult {
    let available = is_param_available(hcam, param_id);

    let (read_value, read_error) = if available {
        // Try to read the value
        match try_read_i32(hcam, param_id) {
            Some(v) => (Some(format!("{}", v)), None),
            None => {
                // Try u16 as fallback
                match try_read_u16(hcam, param_id) {
                    Some(v) => (Some(format!("{}", v)), None),
                    None => (None, Some(get_error_message())),
                }
            }
        }
    } else {
        (None, None)
    };

    ParamTestResult {
        name,
        param_id,
        available,
        read_value,
        read_error,
    }
}

#[test]
fn test_parameter_availability_reporting() {
    // Initialize tracing for test output
    Lazy::force(&TRACING);

    println!("\n=== PVCAM Parameter Availability Test ===\n");

    let hcam = match open_first_camera() {
        Some(h) => h,
        None => {
            eprintln!("Could not open camera, skipping test");
            return;
        }
    };
    println!("[OK] Camera opened, handle = {}\n", hcam);

    // Define parameters to test, grouped by category
    let core_params: Vec<(&str, u32)> = vec![
        // These should ALWAYS be available on any camera
        ("PARAM_SER_SIZE", PARAM_SER_SIZE),
        ("PARAM_PAR_SIZE", PARAM_PAR_SIZE),
        ("PARAM_BIT_DEPTH", PARAM_BIT_DEPTH),
        ("PARAM_READOUT_PORT", PARAM_READOUT_PORT),
        ("PARAM_SPDTAB_INDEX", PARAM_SPDTAB_INDEX),
        ("PARAM_GAIN_INDEX", PARAM_GAIN_INDEX),
    ];

    let thermal_params: Vec<(&str, u32)> = vec![
        // Thermal params - should be available on Prime BSI
        ("PARAM_TEMP", PARAM_TEMP),
        ("PARAM_TEMP_SETPOINT", PARAM_TEMP_SETPOINT),
        ("PARAM_COOLING_MODE", PARAM_COOLING_MODE),
        ("PARAM_FAN_SPEED_SETPOINT", PARAM_FAN_SPEED_SETPOINT),
    ];

    let advanced_params: Vec<(&str, u32)> = vec![
        // Advanced params - may or may not be available
        ("PARAM_CENTROIDS_ENABLED", PARAM_CENTROIDS_ENABLED),
        ("PARAM_CENTROIDS_RADIUS", PARAM_CENTROIDS_RADIUS),
        ("PARAM_CENTROIDS_COUNT", PARAM_CENTROIDS_COUNT),
        ("PARAM_CENTROIDS_MODE", PARAM_CENTROIDS_MODE),
        (
            "PARAM_SMART_STREAM_MODE_ENABLED",
            PARAM_SMART_STREAM_MODE_ENABLED,
        ),
        ("PARAM_SMART_STREAM_MODE", PARAM_SMART_STREAM_MODE),
        ("PARAM_METADATA_ENABLED", PARAM_METADATA_ENABLED),
    ];

    let exposure_params: Vec<(&str, u32)> = vec![
        ("PARAM_EXPOSURE_MODE", PARAM_EXPOSURE_MODE),
        ("PARAM_EXPOSE_OUT_MODE", PARAM_EXPOSE_OUT_MODE),
        ("PARAM_EXP_TIME", PARAM_EXP_TIME),
        ("PARAM_EXP_RES", PARAM_EXP_RES),
        ("PARAM_EXP_RES_INDEX", PARAM_EXP_RES_INDEX),
    ];

    let timing_params: Vec<(&str, u32)> = vec![
        ("PARAM_READOUT_TIME", PARAM_READOUT_TIME),
        ("PARAM_CLEARING_TIME", PARAM_CLEARING_TIME),
        ("PARAM_PRE_TRIGGER_DELAY", PARAM_PRE_TRIGGER_DELAY),
        ("PARAM_POST_TRIGGER_DELAY", PARAM_POST_TRIGGER_DELAY),
    ];

    // Track results
    let mut available_count = 0;
    let mut unavailable_count = 0;
    let mut read_errors = 0;

    // Test core parameters (should all be available)
    println!("--- Core Parameters (should always be available) ---");
    for (name, param_id) in &core_params {
        let result = test_parameter(hcam, name, *param_id);

        if result.available {
            available_count += 1;
            if let Some(value) = &result.read_value {
                println!("[AVAILABLE] {} = {}", result.name, value);
                info!(param = result.name, value = %value, "Core param available");
            } else if let Some(err) = &result.read_error {
                read_errors += 1;
                println!("[AVAILABLE] {} (read error: {})", result.name, err);
                warn!(param = result.name, error = %err, "Core param read failed");
            }
        } else {
            unavailable_count += 1;
            println!("[NOT AVAIL] {} - UNEXPECTED for core param!", result.name);
            warn!(
                param = result.name,
                "Core parameter unexpectedly unavailable"
            );
        }
    }

    // Test thermal parameters
    // Note: Temperature values in PVCAM are stored as signed int16 in centidegrees Celsius
    println!("\n--- Thermal Parameters (expected on Prime BSI) ---");
    for (name, param_id) in &thermal_params {
        let avail = is_param_available(hcam, *param_id);

        if avail {
            available_count += 1;
            // For temperature params, read as signed i16 for correct negative values
            if name.contains("TEMP") {
                if let Some(temp_raw) = try_read_i16(hcam, *param_id) {
                    let temp_c = temp_raw as f64 / 100.0;
                    println!("[AVAILABLE] {} = {:.2}°C (raw: {})", name, temp_c, temp_raw);
                    debug!(param = name, value = temp_raw, "Thermal param available");
                } else {
                    read_errors += 1;
                    println!("[AVAILABLE] {} (read error: {})", name, get_error_message());
                }
            } else {
                // Non-temperature params (COOLING_MODE, FAN_SPEED) read as i32
                if let Some(value) = try_read_i32(hcam, *param_id) {
                    println!("[AVAILABLE] {} = {}", name, value);
                    debug!(param = name, value = value, "Thermal param available");
                } else {
                    read_errors += 1;
                    println!("[AVAILABLE] {} (read error: {})", name, get_error_message());
                }
            }
        } else {
            unavailable_count += 1;
            println!("[NOT AVAIL] {}", name);
            debug!(param = name, "Thermal param not available");
        }
    }

    // Test exposure parameters
    println!("\n--- Exposure Parameters ---");
    for (name, param_id) in &exposure_params {
        let result = test_parameter(hcam, name, *param_id);

        if result.available {
            available_count += 1;
            if let Some(value) = &result.read_value {
                println!("[AVAILABLE] {} = {}", result.name, value);
            } else if let Some(err) = &result.read_error {
                read_errors += 1;
                println!("[AVAILABLE] {} (read error: {})", result.name, err);
            }
        } else {
            unavailable_count += 1;
            println!("[NOT AVAIL] {}", result.name);
        }
    }

    // Test timing parameters
    println!("\n--- Timing Parameters ---");
    for (name, param_id) in &timing_params {
        let result = test_parameter(hcam, name, *param_id);

        if result.available {
            available_count += 1;
            if let Some(value) = &result.read_value {
                println!("[AVAILABLE] {} = {}", result.name, value);
            } else if let Some(err) = &result.read_error {
                read_errors += 1;
                println!("[AVAILABLE] {} (read error: {})", result.name, err);
            }
        } else {
            unavailable_count += 1;
            println!("[NOT AVAIL] {}", result.name);
        }
    }

    // Test advanced/optional parameters
    println!("\n--- Advanced Parameters (may or may not be available) ---");
    for (name, param_id) in &advanced_params {
        let result = test_parameter(hcam, name, *param_id);

        if result.available {
            available_count += 1;
            if let Some(value) = &result.read_value {
                println!("[AVAILABLE] {} = {}", result.name, value);
            } else if let Some(err) = &result.read_error {
                read_errors += 1;
                println!("[AVAILABLE] {} (read error: {})", result.name, err);
            }
        } else {
            unavailable_count += 1;
            println!("[NOT AVAIL] {} (optional - this is OK)", result.name);
        }
    }

    // Summary
    println!("\n=== Summary ===");
    println!("Available parameters: {}", available_count);
    println!("Unavailable parameters: {}", unavailable_count);
    println!("Read errors: {}", read_errors);

    // Verify core parameters are available
    println!("\n--- Verification ---");
    let ser_size_avail = is_param_available(hcam, PARAM_SER_SIZE);
    let par_size_avail = is_param_available(hcam, PARAM_PAR_SIZE);

    if ser_size_avail && par_size_avail {
        let ser = try_read_u16(hcam, PARAM_SER_SIZE).unwrap_or(0);
        let par = try_read_u16(hcam, PARAM_PAR_SIZE).unwrap_or(0);
        println!(
            "[PASS] Core sensor size params available: {}x{} pixels",
            ser, par
        );
    } else {
        println!(
            "[FAIL] Core sensor size params NOT available (ser={}, par={})",
            ser_size_avail, par_size_avail
        );
    }

    // Test that reading unavailable params returns appropriate behavior
    println!("\n--- Unavailable Parameter Behavior ---");

    // Find an unavailable param from our test set
    let mut found_unavailable = false;
    for (name, param_id) in &advanced_params {
        if !is_param_available(hcam, *param_id) {
            found_unavailable = true;
            // Verify that reading it fails gracefully
            let read_result = try_read_i32(hcam, *param_id);
            if read_result.is_none() {
                println!(
                    "[PASS] Reading unavailable param {} correctly returned None",
                    name
                );
            } else {
                println!(
                    "[WARN] Reading unavailable param {} returned value: {:?}",
                    name, read_result
                );
            }
            break;
        }
    }

    if !found_unavailable {
        println!("[INFO] All advanced params were available - could not test unavailable behavior");
    }

    close_camera(hcam);
    println!("\n=== Test Complete ===\n");
}

/// Test that verifies PARAM_TEMP and PARAM_TEMP_SETPOINT behavior specifically
/// These are key parameters for the PvcamFeatures thermal control methods
#[test]
fn test_thermal_param_availability() {
    Lazy::force(&TRACING);

    println!("\n=== Thermal Parameter Availability Test ===\n");

    let hcam = match open_first_camera() {
        Some(h) => h,
        None => {
            eprintln!("Could not open camera, skipping test");
            return;
        }
    };

    // Test PARAM_TEMP - Note: temperature is stored as signed int16 in centidegrees
    let temp_avail = is_param_available(hcam, PARAM_TEMP);
    println!("PARAM_TEMP available: {}", temp_avail);

    if temp_avail {
        // Use int16 reading for temperature (signed centidegrees)
        if let Some(temp_raw) = try_read_i16(hcam, PARAM_TEMP) {
            let temp_c = temp_raw as f64 / 100.0;
            println!("  Current temperature: {:.2}°C (raw: {})", temp_c, temp_raw);
        } else {
            println!("  [WARN] Could not read temperature value");
        }
    }

    // Test PARAM_TEMP_SETPOINT - also signed int16 in centidegrees
    let setpoint_avail = is_param_available(hcam, PARAM_TEMP_SETPOINT);
    println!("PARAM_TEMP_SETPOINT available: {}", setpoint_avail);

    if setpoint_avail {
        if let Some(setpoint_raw) = try_read_i16(hcam, PARAM_TEMP_SETPOINT) {
            let setpoint_c = setpoint_raw as f64 / 100.0;
            println!(
                "  Current setpoint: {:.2}°C (raw: {})",
                setpoint_c, setpoint_raw
            );
        } else {
            println!("  [WARN] Could not read setpoint value");
        }
    }

    // Prime BSI should have both thermal params
    if temp_avail && setpoint_avail {
        println!("\n[PASS] Both thermal parameters available (expected for Prime BSI)");
    } else {
        println!("\n[INFO] One or both thermal parameters unavailable");
        println!("       This may be expected depending on camera model");
    }

    close_camera(hcam);
}

/// Test that verifies centroids and smart streaming feature detection
#[test]
fn test_advanced_feature_availability() {
    Lazy::force(&TRACING);

    println!("\n=== Advanced Feature Availability Test ===\n");

    let hcam = match open_first_camera() {
        Some(h) => h,
        None => {
            eprintln!("Could not open camera, skipping test");
            return;
        }
    };

    // Centroids feature
    let centroids_enabled_avail = is_param_available(hcam, PARAM_CENTROIDS_ENABLED);
    println!("Centroids Feature:");
    println!(
        "  PARAM_CENTROIDS_ENABLED available: {}",
        centroids_enabled_avail
    );

    if centroids_enabled_avail {
        let mode_avail = is_param_available(hcam, PARAM_CENTROIDS_MODE);
        let radius_avail = is_param_available(hcam, PARAM_CENTROIDS_RADIUS);
        let count_avail = is_param_available(hcam, PARAM_CENTROIDS_COUNT);
        println!("  PARAM_CENTROIDS_MODE available: {}", mode_avail);
        println!("  PARAM_CENTROIDS_RADIUS available: {}", radius_avail);
        println!("  PARAM_CENTROIDS_COUNT available: {}", count_avail);
    }

    // Smart streaming feature
    let smart_stream_enabled_avail = is_param_available(hcam, PARAM_SMART_STREAM_MODE_ENABLED);
    println!("\nSmart Streaming Feature:");
    println!(
        "  PARAM_SMART_STREAM_MODE_ENABLED available: {}",
        smart_stream_enabled_avail
    );

    if smart_stream_enabled_avail {
        let mode_avail = is_param_available(hcam, PARAM_SMART_STREAM_MODE);
        println!("  PARAM_SMART_STREAM_MODE available: {}", mode_avail);
    }

    // Metadata feature
    let metadata_avail = is_param_available(hcam, PARAM_METADATA_ENABLED);
    println!("\nMetadata Feature:");
    println!("  PARAM_METADATA_ENABLED available: {}", metadata_avail);

    if metadata_avail {
        if let Some(enabled) = try_read_i32(hcam, PARAM_METADATA_ENABLED) {
            println!("  Metadata currently enabled: {}", enabled != 0);
        }
    }

    close_camera(hcam);
    println!("\n[INFO] Advanced feature availability logged for camera profile");
}
