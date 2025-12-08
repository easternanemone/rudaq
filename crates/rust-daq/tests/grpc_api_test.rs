#![cfg(not(target_arch = "wasm32"))]
//! Integration test to verify gRPC API definitions are accessible
//! Requires 'networking' feature

#![cfg(feature = "networking")]

use rust_daq::grpc::{
    DataPoint, MeasurementRequest, ScriptStatus, StartRequest, StartResponse, StatusRequest,
    StopRequest, StopResponse, SystemStatus, UploadRequest, UploadResponse,
};
use std::collections::HashMap;

#[test]
fn test_grpc_types_accessible() {
    // This test verifies that all gRPC types are properly exported from the library

    // Upload types
    let upload_req = UploadRequest {
        script_content: "test".to_string(),
        name: "test_script".to_string(),
        metadata: HashMap::new(),
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
        parameters: HashMap::new(),
    };
    assert_eq!(start_req.script_id, "123");

    let start_resp = StartResponse {
        started: true,
        execution_id: "exec-123".to_string(),
    };
    assert!(start_resp.started);

    // Stop types
    let stop_req = StopRequest {
        execution_id: "exec-123".to_string(),
        force: false,
    };
    assert_eq!(stop_req.execution_id, "exec-123");

    let stop_resp = StopResponse {
        stopped: true,
        message: String::new(),
    };
    assert!(stop_resp.stopped);

    // Status types
    let status_req = StatusRequest {
        execution_id: "exec-123".to_string(),
    };
    assert_eq!(status_req.execution_id, "exec-123");

    let system_status = SystemStatus {
        current_state: "running".to_string(),
        current_memory_usage_mb: 100.0,
        live_values: HashMap::new(),
        timestamp_ns: 0,
    };
    assert_eq!(system_status.current_state, "running");

    // ScriptStatus is now a message, not an enum
    let script_status = ScriptStatus {
        execution_id: "exec-123".to_string(),
        state: "RUNNING".to_string(),
        error_message: String::new(),
        start_time_ns: 0,
        end_time_ns: 0,
        script_id: "script-123".to_string(),
        progress_percent: 50,
        current_line: String::new(),
    };
    assert_eq!(script_status.state, "RUNNING");

    // Measurement types
    let measurement_req = MeasurementRequest {
        channels: vec!["ch1".to_string()],
        max_rate_hz: 100,
    };
    assert_eq!(measurement_req.channels[0], "ch1");

    let data_point = DataPoint {
        channel: "test".to_string(),
        value: 1.0,
        timestamp_ns: 0,
    };
    assert_eq!(data_point.channel, "test");
}
