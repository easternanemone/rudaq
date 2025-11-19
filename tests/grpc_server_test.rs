use rust_daq::grpc::{DaqServer, ControlService, UploadRequest, StartRequest, StatusRequest};
use tonic::Request;
use std::collections::HashMap;

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

    assert!(!resp.success, "Invalid script should fail validation");
    assert!(resp.script_id.is_empty(), "Should not generate ID for invalid script");
    assert!(!resp.error_message.is_empty(), "Should have error message");
}

#[tokio::test]
async fn test_grpc_start_nonexistent_script() {
    let server = DaqServer::new();
    let request = Request::new(StartRequest {
        script_id: "does-not-exist".to_string(),
        parameters: HashMap::new(),
    });

    let result = server.start_script(request).await;
    assert!(result.is_err(), "Starting nonexistent script should fail");

    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound);
}

#[tokio::test]
async fn test_grpc_script_execution_lifecycle() {
    let server = DaqServer::new();

    // 1. Upload script
    let upload_req = Request::new(UploadRequest {
        script_content: r#"
            let a = 10;
            let b = 20;
            a + b
        "#.to_string(),
        name: "math_script".to_string(),
        metadata: HashMap::new(),
    });

    let upload_resp = server.upload_script(upload_req).await.unwrap().into_inner();
    assert!(upload_resp.success, "Script upload should succeed");
    let script_id = upload_resp.script_id;

    // 2. Start execution
    let start_req = Request::new(StartRequest {
        script_id: script_id.clone(),
        parameters: HashMap::new(),
    });

    let start_resp = server.start_script(start_req).await.unwrap().into_inner();
    assert!(start_resp.started, "Script should start successfully");
    let execution_id = start_resp.execution_id;

    // 3. Wait for completion (scripts execute in background)
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // 4. Check final status
    let status_req = Request::new(StatusRequest {
        execution_id: execution_id.clone(),
    });

    let status_resp = server.get_script_status(status_req).await.unwrap().into_inner();
    assert_eq!(status_resp.state, "COMPLETED", "Script should complete successfully");
    assert_eq!(status_resp.error_message, "", "Should have no errors");
    assert!(status_resp.start_time_ns > 0, "Should have start time");
    assert!(status_resp.end_time_ns > 0, "Should have end time");
}

#[tokio::test]
async fn test_grpc_script_with_error() {
    let server = DaqServer::new();

    // Upload script that will cause runtime error
    let upload_req = Request::new(UploadRequest {
        script_content: "let x = 1 / 0;".to_string(), // Division by zero
        name: "error_script".to_string(),
        metadata: HashMap::new(),
    });

    let upload_resp = server.upload_script(upload_req).await.unwrap().into_inner();
    let script_id = upload_resp.script_id;

    // Start execution
    let start_req = Request::new(StartRequest {
        script_id,
        parameters: HashMap::new(),
    });

    let start_resp = server.start_script(start_req).await.unwrap().into_inner();
    let execution_id = start_resp.execution_id;

    // Wait for error
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Check error status
    let status_req = Request::new(StatusRequest {
        execution_id,
    });

    let status_resp = server.get_script_status(status_req).await.unwrap().into_inner();
    assert_eq!(status_resp.state, "ERROR", "Script should error");
    assert!(!status_resp.error_message.is_empty(), "Should have error message");
}

#[tokio::test]
async fn test_grpc_concurrent_scripts() {
    let server = DaqServer::new();

    // Upload two scripts
    let script1 = server.upload_script(Request::new(UploadRequest {
        script_content: "let x = 1; x".to_string(),
        name: "script1".to_string(),
        metadata: HashMap::new(),
    })).await.unwrap().into_inner();

    let script2 = server.upload_script(Request::new(UploadRequest {
        script_content: "let y = 2; y".to_string(),
        name: "script2".to_string(),
        metadata: HashMap::new(),
    })).await.unwrap().into_inner();

    // Start both scripts
    let exec1 = server.start_script(Request::new(StartRequest {
        script_id: script1.script_id,
        parameters: HashMap::new(),
    })).await.unwrap().into_inner();

    let exec2 = server.start_script(Request::new(StartRequest {
        script_id: script2.script_id,
        parameters: HashMap::new(),
    })).await.unwrap().into_inner();

    // Wait for both to complete
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify both completed successfully
    let status1 = server.get_script_status(Request::new(StatusRequest {
        execution_id: exec1.execution_id,
    })).await.unwrap().into_inner();

    let status2 = server.get_script_status(Request::new(StatusRequest {
        execution_id: exec2.execution_id,
    })).await.unwrap().into_inner();

    assert_eq!(status1.state, "COMPLETED");
    assert_eq!(status2.state, "COMPLETED");
}
