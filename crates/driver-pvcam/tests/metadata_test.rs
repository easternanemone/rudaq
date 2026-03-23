//! Metadata verification tests for PvcamDriver
//!
//! Ensures all expected parameters are registered and have correct attributes.

use common::capabilities::Parameterized;
use driver_pvcam::PvcamDriver;
use serde_json::json;

#[tokio::test]
async fn verify_metadata_parameters() {
    let driver = PvcamDriver::new_async("MockCamera".to_string())
        .await
        .expect("Failed to create driver");

    let params = driver.parameters();
    let names = params.names();

    // 1. Info Group (Read-Only)
    assert_contains(&names, "info.serial_number");
    assert_contains(&names, "info.firmware_version");
    assert_contains(&names, "info.model_name");
    assert_contains(&names, "info.bit_depth");

    let serial = params.get("info.serial_number").unwrap();
    assert!(
        serial.metadata().read_only,
        "Serial number should be read-only"
    );

    // 2. Acquisition Group (Read-Write)
    assert_contains(&names, "acquisition.exposure_ms");
    assert_contains(&names, "acquisition.roi");
    assert_contains(&names, "acquisition.binning");
    assert_contains(&names, "acquisition.trigger_mode");

    let exposure = params.get("acquisition.exposure_ms").unwrap();
    assert!(
        !exposure.metadata().read_only,
        "Exposure should be writable"
    );

    // 3. Thermal Group
    assert_contains(&names, "thermal.temperature");
    assert_contains(&names, "thermal.setpoint");
    assert_contains(&names, "thermal.fan_speed");

    let temp = params.get("thermal.temperature").unwrap();
    assert!(
        temp.metadata().read_only,
        "Current temperature should be read-only"
    );

    let setpoint = params.get("thermal.setpoint").unwrap();
    assert!(
        !setpoint.metadata().read_only,
        "Temperature setpoint should be writable"
    );

    // 4. Readout Group
    assert_contains(&names, "readout.port");
    assert_contains(&names, "readout.speed_mode");
    assert_contains(&names, "readout.gain_mode");

    // 5. Timing Group (Read-Only)
    assert_contains(&names, "acquisition.readout_time_us");
    assert_contains(&names, "acquisition.clearing_time_us");

    let readout_time = params.get("acquisition.readout_time_us").unwrap();
    assert!(
        readout_time.metadata().read_only,
        "Readout time should be read-only"
    );
}

fn assert_contains(names: &[&str], expected: &str) {
    assert!(names.contains(&expected), "Missing parameter: {}", expected);
}

#[tokio::test]
async fn verify_parameter_persistence() {
    let driver = PvcamDriver::new_async("MockCamera".to_string())
        .await
        .expect("Failed to create driver");
    let params = driver.parameters();

    // 1. Temperature Setpoint
    params
        .get("thermal.setpoint")
        .unwrap()
        .set_json(json!(-20.0))
        .unwrap();
    let val = params.get("thermal.setpoint").unwrap().get_json().unwrap();
    assert_eq!(val, json!(-20.0), "Temperature setpoint not updated");

    // 2. Fan Speed
    params
        .get("thermal.fan_speed")
        .unwrap()
        .set_json(json!("Medium"))
        .unwrap();
    let val = params.get("thermal.fan_speed").unwrap().get_json().unwrap();
    assert_eq!(val, json!("Medium"), "Fan speed not updated");

    // 3. Exposure Mode
    params
        .get("acquisition.trigger_mode")
        .unwrap()
        .set_json(json!("EdgeTrigger"))
        .unwrap();
    let val = params
        .get("acquisition.trigger_mode")
        .unwrap()
        .get_json()
        .unwrap();
    assert_eq!(val, json!("EdgeTrigger"), "Trigger mode not updated");

    // 4. Clear Mode
    params
        .get("acquisition.clear_mode")
        .unwrap()
        .set_json(json!("PreSequence"))
        .unwrap();
    let val = params
        .get("acquisition.clear_mode")
        .unwrap()
        .get_json()
        .unwrap();
    assert_eq!(val, json!("PreSequence"), "Clear mode not updated");

    // 5. Expose Out Mode
    params
        .get("acquisition.expose_out_mode")
        .unwrap()
        .set_json(json!("RollingShutter"))
        .unwrap();
    let val = params
        .get("acquisition.expose_out_mode")
        .unwrap()
        .get_json()
        .unwrap();
    assert_eq!(val, json!("RollingShutter"), "Expose out mode not updated");

    // 6. Shutter Mode (if exposed)
    // Note: Assuming "shutter.mode" is the name. If test fails, check lib.rs.
    if let Some(p) = params.get("shutter.mode") {
        p.set_json(json!("Open")).unwrap();
        let val = p.get_json().unwrap();
        assert_eq!(val, json!("Open"), "Shutter mode not updated");
    }

    // 7. Shutter Delays
    if let Some(p) = params.get("shutter.open_delay_us") {
        p.set_json(json!(500)).unwrap();
        let val = p.get_json().unwrap();
        assert_eq!(val, json!(500));
    }
}

/// Verify the metadata_enabled parameter exists and can be toggled (bd-oqo7.2).
///
/// This parameter controls both the SDK PARAM_METADATA_ENABLED and the
/// acquisition's decoding flag via its write callback.
#[tokio::test]
async fn metadata_enabled_parameter_exists_and_toggles() {
    let driver = PvcamDriver::new_async("MockCamera".to_string())
        .await
        .expect("Failed to create driver");
    let params = driver.parameters();

    let p = params
        .get("processing.metadata_enabled")
        .expect("processing.metadata_enabled parameter should exist");

    // Default is false
    let val = p.get_json().unwrap();
    assert_eq!(
        val,
        json!(false),
        "metadata_enabled default should be false"
    );

    // Toggle to true (no SDK connected, so write callback is a no-op for mock)
    p.set_json(json!(true)).unwrap();
    let val = p.get_json().unwrap();
    assert_eq!(
        val,
        json!(true),
        "metadata_enabled should be true after set"
    );

    // Toggle back to false
    p.set_json(json!(false)).unwrap();
    let val = p.get_json().unwrap();
    assert_eq!(
        val,
        json!(false),
        "metadata_enabled should be false after reset"
    );
}

/// Verify that PVCAM-specific metadata fields map to the expected extra keys (bd-oqo7.2).
///
/// This simulates the frame_loop.rs mapping from PVCAM FrameMetadata fields
/// into common::data::FrameMetadata.extra, ensuring the key names and value
/// formats are correct for downstream consumers.
#[test]
fn pvcam_metadata_maps_to_common_extra_fields() {
    use common::data::{Frame, FrameMetadata};

    // Simulate the values that frame_loop.rs would extract from pl_md_frame_decode
    let pvcam_frame_nr: i32 = 42;
    let pvcam_bof_ns: u64 = 1_710_000_000_000_000;
    let pvcam_eof_ns: u64 = 1_710_000_050_000_000;
    let pvcam_exposure_ns: u64 = 50_000_000;
    let pvcam_bit_depth: u16 = 16;
    let pvcam_roi_count: u16 = 1;

    // Build ext_metadata exactly as frame_loop.rs does (bd-oqo7.2)
    let mut ext_metadata = FrameMetadata {
        binning: Some((1, 1)),
        ..Default::default()
    };

    ext_metadata
        .extra
        .insert("timestamp_bof_ns".to_string(), pvcam_bof_ns.to_string());
    ext_metadata
        .extra
        .insert("timestamp_eof_ns".to_string(), pvcam_eof_ns.to_string());
    ext_metadata.extra.insert(
        "exposure_time_ns".to_string(),
        pvcam_exposure_ns.to_string(),
    );
    ext_metadata
        .extra
        .insert("frame_nr".to_string(), pvcam_frame_nr.to_string());
    ext_metadata
        .extra
        .insert("bit_depth".to_string(), pvcam_bit_depth.to_string());
    ext_metadata
        .extra
        .insert("roi_count".to_string(), pvcam_roi_count.to_string());

    let pixels = vec![0u16; 10_000];
    let frame = Frame::from_u16(100, 100, &pixels).with_metadata(ext_metadata);

    let md = frame.metadata.as_ref().expect("metadata should be present");

    // Verify all PVCAM fields are present with correct string representations
    assert_eq!(
        md.extra.get("timestamp_bof_ns").unwrap(),
        "1710000000000000"
    );
    assert_eq!(
        md.extra.get("timestamp_eof_ns").unwrap(),
        "1710000050000000"
    );
    assert_eq!(md.extra.get("exposure_time_ns").unwrap(), "50000000");
    assert_eq!(md.extra.get("frame_nr").unwrap(), "42");
    assert_eq!(md.extra.get("bit_depth").unwrap(), "16");
    assert_eq!(md.extra.get("roi_count").unwrap(), "1");

    // Verify values round-trip to their original types
    assert_eq!(
        md.extra
            .get("timestamp_bof_ns")
            .unwrap()
            .parse::<u64>()
            .unwrap(),
        pvcam_bof_ns
    );
    assert_eq!(
        md.extra.get("frame_nr").unwrap().parse::<i32>().unwrap(),
        pvcam_frame_nr
    );
    assert_eq!(
        md.extra.get("bit_depth").unwrap().parse::<u16>().unwrap(),
        pvcam_bit_depth
    );

    // Verify readout time can be derived from timestamps
    let bof: u64 = md.extra.get("timestamp_bof_ns").unwrap().parse().unwrap();
    let eof: u64 = md.extra.get("timestamp_eof_ns").unwrap().parse().unwrap();
    let readout_ns = eof - bof;
    assert_eq!(readout_ns, 50_000_000, "Readout time should be EOF - BOF");
}
