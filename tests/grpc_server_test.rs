//! Integration tests for the gRPC server implementation
//! Requires 'networking' feature

#![cfg(feature = "networking")]

use rust_daq::grpc::{ControlService, DaqServer, StartRequest, StatusRequest, UploadRequest};
use std::collections::HashMap;
use tonic::Request;

#[tokio::test]
async fn test_grpc_upload_valid_script() {
    let server = DaqServer::new();
    let request = Request::new(UploadRequest {
        script_content: "let x = 42; x + 1".to_string(),
        name: "test_script".to_string(),
        metadata: HashMap::new(),
    });

    let response = server.upload_script(request).await.unwrap();
    let resp = response.into_inner();

    assert!(resp.success, "Script upload should succeed");
    assert!(!resp.script_id.is_empty(), "Should generate script ID");
    assert_eq!(resp.error_message, "", "Should have no error message");
}

#[tokio::test]
async fn test_grpc_upload_invalid_script() {
    let server = DaqServer::new();
    let request = Request::new(UploadRequest {
        script_content: "this is not valid rhai {{{".to_string(),
        name: "bad_script".to_string(),
        metadata: HashMap::new(),
    });

    let response = server.upload_script(request).await.unwrap();
    let resp = response.into_inner();

    assert!(!resp.success, "Invalid script should fail");
    assert!(!resp.error_message.is_empty(), "Should have error message");
}

#[tokio::test]
async fn test_grpc_start_script_not_found() {
    let server = DaqServer::new();
    let request = Request::new(StartRequest {
        script_id: "nonexistent_id".to_string(),
        parameters: HashMap::new(),
    });

    let response = server.start_script(request).await;

    // Server returns NotFound status for nonexistent scripts
    assert!(response.is_err(), "Starting nonexistent script should fail");
    let err = response.unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
    assert!(
        err.message().contains("not found") || err.message().contains("Not found"),
        "Error message should mention not found: {}",
        err.message()
    );
}

#[tokio::test]
async fn test_grpc_status_no_execution() {
    let server = DaqServer::new();
    let request = Request::new(StatusRequest {
        execution_id: "nonexistent_execution".to_string(),
    });

    let response = server.get_script_status(request).await;

    // Server returns NotFound status for nonexistent executions
    assert!(
        response.is_err(),
        "Status for nonexistent execution should fail"
    );
    let err = response.unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_full_workflow() {
    let server = DaqServer::new();

    // Upload script
    let upload_request = Request::new(UploadRequest {
        script_content: "let result = 42; result".to_string(),
        name: "workflow_test".to_string(),
        metadata: HashMap::new(),
    });

    let upload_response = server.upload_script(upload_request).await.unwrap();
    let upload_resp = upload_response.into_inner();
    assert!(upload_resp.success);
    let script_id = upload_resp.script_id;

    // Start script
    let start_request = Request::new(StartRequest {
        script_id: script_id.clone(),
        parameters: HashMap::new(),
    });

    let start_response = server.start_script(start_request).await.unwrap();
    let start_resp = start_response.into_inner();
    assert!(start_resp.started, "Script should start successfully");
    let execution_id = start_resp.execution_id;

    // Check status
    let status_request = Request::new(StatusRequest {
        execution_id: execution_id.clone(),
    });
    let status_response = server.get_script_status(status_request).await.unwrap();
    let status = status_response.into_inner();

    // Script may be running or completed
    assert!(
        status.state == "RUNNING" || status.state == "COMPLETED" || status.state == "PENDING",
        "Script state should be valid: {}",
        status.state
    );
    assert_eq!(
        status.execution_id, execution_id,
        "Execution ID should match"
    );
    assert_eq!(status.script_id, script_id, "Script ID should match");
}
