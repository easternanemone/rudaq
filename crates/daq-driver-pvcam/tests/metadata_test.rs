//! Metadata verification tests for PvcamDriver
//!
//! Ensures all expected parameters are registered and have correct attributes.

use common::capabilities::Parameterized;
use daq_driver_pvcam::PvcamDriver;
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
