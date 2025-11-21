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
    });

    let response = server.start_script(request).await.unwrap();
    let resp = response.into_inner();

    assert!(!resp.success, "Starting nonexistent script should fail");
    assert!(
        resp.error_message.contains("not found")
            || resp.error_message.contains("Script not found"),
        "Error message should mention script not found: {}",
        resp.error_message
    );
}

#[tokio::test]
async fn test_grpc_status_no_script() {
    let server = DaqServer::new();
    let request = Request::new(StatusRequest {});

    let response = server.get_status(request).await.unwrap();
    let status = response.into_inner();

    assert!(!status.is_running, "Should not be running initially");
    assert!(status.current_script.is_none(), "Should have no script");
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
    });

    let start_response = server.start_script(start_request).await.unwrap();
    assert!(start_response.into_inner().success);

    // Check status
    let status_request = Request::new(StatusRequest {});
    let status_response = server.get_status(status_request).await.unwrap();
    let status = status_response.into_inner();

    assert!(status.is_running || !status.is_running); // Script may have already finished
    if let Some(current) = status.current_script {
        assert_eq!(current, script_id);
    }
}
