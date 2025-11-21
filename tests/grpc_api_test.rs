//! Integration test to verify gRPC API definitions are accessible
//! Requires 'networking' feature

#![cfg(feature = "networking")]

use rust_daq::grpc::proto;
use rust_daq::grpc::{
    DataPoint, MeasurementRequest, ScriptStatus, StartRequest, StartResponse, StatusRequest,
    StopRequest, StopResponse, SystemStatus, UploadRequest, UploadResponse,
};

#[test]
fn test_grpc_types_accessible() {
    // This test verifies that all gRPC types are properly exported from the library

    // Upload types
    let upload_req = UploadRequest {
        script_content: "test".to_string(),
        name: "test_script".to_string(),
        metadata: Default::default(),
    };
    assert_eq!(upload_req.name, "test_script");

    let upload_resp = UploadResponse {
        script_id: "123".to_string(),
        success: true,
        error_message: String::new(),
    };
    assert!(upload_resp.success);

    // Start types
    let start_req = StartRequest {
        script_id: "123".to_string(),
    };
    assert_eq!(start_req.script_id, "123");

    let start_resp = StartResponse {
        success: true,
        error_message: String::new(),
    };
    assert!(start_resp.success);

    // Stop types
    let stop_req = StopRequest {};
    let stop_resp = StopResponse {
        success: true,
        error_message: String::new(),
    };
    assert!(stop_resp.success);

    // Status types
    let status_req = StatusRequest {};
    let system_status = SystemStatus {
        is_running: true,
        current_script: Some("test".to_string()),
        script_status: ScriptStatus::Running.into(),
        error_message: String::new(),
    };
    assert!(system_status.is_running);

    // Measurement types
    let measurement_req = MeasurementRequest {};
    let data_point = DataPoint {
        timestamp: 0,
        channel: "test".to_string(),
        value: 1.0,
        unit: "V".to_string(),
        metadata: Default::default(),
    };
    assert_eq!(data_point.channel, "test");
}
