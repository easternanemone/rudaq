/// Integration test to verify gRPC API definitions are accessible
use rust_daq::network::proto;
use rust_daq::network::{
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
        started: true,
        execution_id: "exec-456".to_string(),
    };
    assert!(start_resp.started);

    // Stop types
    let stop_req = StopRequest {
        execution_id: "exec-456".to_string(),
    };
    assert_eq!(stop_req.execution_id, "exec-456");

    let stop_resp = StopResponse { stopped: true };
    assert!(stop_resp.stopped);

    // Status types
    let status_req = StatusRequest {
        execution_id: "exec-456".to_string(),
    };
    assert_eq!(status_req.execution_id, "exec-456");

    let script_status = ScriptStatus {
        execution_id: "exec-456".to_string(),
        state: "RUNNING".to_string(),
        error_message: String::new(),
        start_time_ns: 1000,
        end_time_ns: 0,
    };
    assert_eq!(script_status.state, "RUNNING");

    // System status
    let system_status = SystemStatus {
        current_state: "RUNNING".to_string(),
        current_memory_usage_mb: 128.5,
        live_values: Default::default(),
        timestamp_ns: 2000,
    };
    assert_eq!(system_status.current_state, "RUNNING");

    // Measurement types
    let meas_req = MeasurementRequest {
        instrument: "camera".to_string(),
    };
    assert_eq!(meas_req.instrument, "camera");

    // DataPoint with scalar value
    let data_point = DataPoint {
        instrument: "detector".to_string(),
        value: Some(proto::data_point::Value::Scalar(42.5)),
        timestamp_ns: 3000,
    };
    assert_eq!(data_point.instrument, "detector");
}

#[test]
fn test_service_trait_exists() {
    // This test verifies the ControlService trait can be referenced
    // We can't implement it here without async runtime, but we can verify it exists

    // This will fail to compile if ControlService trait is not accessible
    fn _assert_trait_exists<T: proto::control_service_server::ControlService>() {}
}
