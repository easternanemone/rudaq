use tonic::{transport::Server, Request, Response, Status};
use crate::grpc::proto::{
    control_service_server::{ControlService, ControlServiceServer},
    UploadRequest, UploadResponse, StartRequest, StartResponse,
    StopRequest, StopResponse, StatusRequest, ScriptStatus, SystemStatus,
};
use crate::scripting::ScriptHost;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use uuid::Uuid;

/// State of a script execution
#[derive(Clone)]
struct ExecutionState {
    script_id: String,
    state: String,
    start_time: u64,
    end_time: Option<u64>,
    error: Option<String>,
}

/// DAQ gRPC server implementation
pub struct DaqServer {
    script_host: Arc<RwLock<ScriptHost>>,
    scripts: Arc<RwLock<HashMap<String, String>>>,
    executions: Arc<RwLock<HashMap<String, ExecutionState>>>,
}

impl DaqServer {
    /// Create a new DAQ server instance
    pub fn new() -> Self {
        Self {
            script_host: Arc::new(RwLock::new(
                ScriptHost::with_hardware(tokio::runtime::Handle::current())
            )),
            scripts: Arc::new(RwLock::new(HashMap::new())),
            executions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for DaqServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl ControlService for DaqServer {
    /// Upload and validate a script
    async fn upload_script(
        &self,
        request: Request<UploadRequest>,
    ) -> Result<Response<UploadResponse>, Status> {
        let req = request.into_inner();
        let script_id = Uuid::new_v4().to_string();

        // Validate script syntax
        let host = self.script_host.read().await;
        if let Err(e) = host.validate_script(&req.script_content) {
            return Ok(Response::new(UploadResponse {
                script_id: String::new(),
                success: false,
                error_message: format!("Validation failed: {}", e),
            }));
        }

        // Store validated script
        self.scripts.write().await.insert(script_id.clone(), req.script_content);

        Ok(Response::new(UploadResponse {
            script_id,
            success: true,
            error_message: String::new(),
        }))
    }

    /// Start execution of an uploaded script
    async fn start_script(
        &self,
        request: Request<StartRequest>,
    ) -> Result<Response<StartResponse>, Status> {
        let req = request.into_inner();
        let scripts = self.scripts.read().await;

        let script = scripts.get(&req.script_id)
            .ok_or_else(|| Status::not_found("Script not found"))?;

        let execution_id = Uuid::new_v4().to_string();

        // Record execution start
        self.executions.write().await.insert(execution_id.clone(), ExecutionState {
            script_id: req.script_id,
            state: "RUNNING".to_string(),
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
            end_time: None,
            error: None,
        });

        // Execute script in background (non-blocking)
        let script_clone = script.clone();
        let host_clone = self.script_host.clone();
        let executions_clone = self.executions.clone();
        let exec_id_clone = execution_id.clone();

        tokio::spawn(async move {
            let host = host_clone.read().await;
            let result = host.run_script(&script_clone);

            // Update execution state with result
            let mut executions = executions_clone.write().await;
            if let Some(exec) = executions.get_mut(&exec_id_clone) {
                exec.state = if result.is_ok() { "COMPLETED" } else { "ERROR" }.to_string();
                exec.end_time = Some(std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64);
                if let Err(e) = result {
                    exec.error = Some(e.to_string());
                }
            }
        });

        Ok(Response::new(StartResponse {
            started: true,
            execution_id,
        }))
    }

    /// Stop a running script execution
    async fn stop_script(
        &self,
        _request: Request<StopRequest>,
    ) -> Result<Response<StopResponse>, Status> {
        // TODO: Implement script cancellation with tokio::task::JoinHandle
        // For now, scripts run to completion
        Ok(Response::new(StopResponse {
            stopped: false,
        }))
    }

    /// Get current status of a script execution
    async fn get_script_status(
        &self,
        request: Request<StatusRequest>,
    ) -> Result<Response<ScriptStatus>, Status> {
        let req = request.into_inner();
        let executions = self.executions.read().await;

        let exec = executions.get(&req.execution_id)
            .ok_or_else(|| Status::not_found("Execution not found"))?;

        Ok(Response::new(ScriptStatus {
            execution_id: req.execution_id,
            state: exec.state.clone(),
            error_message: exec.error.clone().unwrap_or_default(),
            start_time_ns: exec.start_time,
            end_time_ns: exec.end_time.unwrap_or(0),
        }))
    }

    type StreamStatusStream = tokio_stream::wrappers::ReceiverStream<Result<SystemStatus, Status>>;

    /// Stream system status updates at 10Hz
    async fn stream_status(
        &self,
        _request: Request<StatusRequest>,
    ) -> Result<Response<Self::StreamStatusStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // Spawn background task to send status updates
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
            loop {
                interval.tick().await;

                // TODO: Get real system metrics
                let status = SystemStatus {
                    current_state: "RUNNING".to_string(),
                    current_memory_usage_mb: 42.0,
                    live_values: HashMap::new(),
                    timestamp_ns: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos() as u64,
                };

                if tx.send(Ok(status)).await.is_err() {
                    break; // Client disconnected
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    type StreamMeasurementsStream = tokio_stream::wrappers::ReceiverStream<Result<crate::grpc::proto::DataPoint, Status>>;

    /// Stream measurement data from specified channels
    async fn stream_measurements(
        &self,
        _request: Request<crate::grpc::proto::MeasurementRequest>,
    ) -> Result<Response<Self::StreamMeasurementsStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        // TODO: Connect to actual hardware measurement sources
        // For now, just close the channel (no data)
        drop(tx);

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

/// Start the DAQ gRPC server
pub async fn start_server(addr: std::net::SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    let server = DaqServer::new();

    println!("🌐 DAQ gRPC server listening on {}", addr);

    Server::builder()
        .add_service(ControlServiceServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_upload_valid_script() {
        let server = DaqServer::new();
        let request = Request::new(UploadRequest {
            script_content: "let x = 42;".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });

        let response = server.upload_script(request).await.unwrap();
        let resp = response.into_inner();

        assert!(resp.success);
        assert!(!resp.script_id.is_empty());
        assert_eq!(resp.error_message, "");
    }

    #[tokio::test]
    async fn test_upload_invalid_script() {
        let server = DaqServer::new();
        let request = Request::new(UploadRequest {
            script_content: "this is not valid rhai syntax {{{".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });

        let response = server.upload_script(request).await.unwrap();
        let resp = response.into_inner();

        assert!(!resp.success);
        assert!(resp.script_id.is_empty());
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_start_nonexistent_script() {
        let server = DaqServer::new();
        let request = Request::new(StartRequest {
            script_id: "nonexistent-id".to_string(),
            parameters: HashMap::new(),
        });

        let result = server.start_script(request).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_script_execution_lifecycle() {
        let server = DaqServer::new();

        // Upload script
        let upload_req = Request::new(UploadRequest {
            script_content: "let x = 1 + 1;".to_string(),
            name: "test".to_string(),
            metadata: HashMap::new(),
        });
        let upload_resp = server.upload_script(upload_req).await.unwrap().into_inner();
        assert!(upload_resp.success);

        // Start execution
        let start_req = Request::new(StartRequest {
            script_id: upload_resp.script_id,
            parameters: HashMap::new(),
        });
        let start_resp = server.start_script(start_req).await.unwrap().into_inner();
        assert!(start_resp.started);

        // Wait for completion
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Check status
        let status_req = Request::new(StatusRequest {
            execution_id: start_resp.execution_id,
        });
        let status_resp = server.get_script_status(status_req).await.unwrap().into_inner();
        assert_eq!(status_resp.state, "COMPLETED");
        assert_eq!(status_resp.error_message, "");
    }
}
