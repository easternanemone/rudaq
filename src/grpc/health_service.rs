use crate::grpc::proto::health::health_check_response::ServingStatus;
use crate::grpc::proto::health::health_server::Health;
use crate::grpc::proto::health::{HealthCheckRequest, HealthCheckResponse};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

/// gRPC Health Check Service implementation
#[derive(Debug, Clone)]
pub struct HealthServiceImpl {
    // Map service name -> status sender
    statuses: Arc<Mutex<HashMap<String, watch::Sender<ServingStatus>>>>,
}

impl Default for HealthServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthServiceImpl {
    pub fn new() -> Self {
        Self {
            statuses: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Update the serving status of a service
    pub fn set_serving_status(&self, service: &str, status: ServingStatus) {
        let mut statuses = self.statuses.lock().unwrap();
        if let Some(tx) = statuses.get(service) {
            let _ = tx.send(status);
        } else {
            let (tx, _) = watch::channel(status);
            statuses.insert(service.to_string(), tx);
        }
    }
}

#[tonic::async_trait]
impl Health for HealthServiceImpl {
    type WatchStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<HealthCheckResponse, Status>> + Send + Sync>,
    >;

    async fn check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let service = request.into_inner().service;
        let status = {
            let statuses = self.statuses.lock().unwrap();
            if service.is_empty() {
                // Overall server status
                ServingStatus::Serving
            } else if let Some(tx) = statuses.get(&service) {
                *tx.borrow()
            } else {
                // Service not found in our map
                // Standard behavior is to return NOT_FOUND status code
                return Err(Status::not_found(format!("Unknown service: {}", service)));
            }
        };

        Ok(Response::new(HealthCheckResponse {
            status: status.into(),
        }))
    }

    async fn watch(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let service = request.into_inner().service;

        let rx = {
            let mut statuses = self.statuses.lock().unwrap();
            if service.is_empty() {
                // Overall health - simplified to always SERVING for now
                let (_, rx) = watch::channel(ServingStatus::Serving);
                rx
            } else {
                // If known service, subscribe
                // If unknown, we can either return error or start tracking as UNKNOWN
                // Standard behavior for Watch is strictly streaming changes.
                // If service is unknown, it should return SERVICE_UNKNOWN
                if let Some(tx) = statuses.get(&service) {
                    tx.subscribe()
                } else {
                    // Initialize as SERVICE_UNKNOWN
                    let (tx, rx) = watch::channel(ServingStatus::ServiceUnknown);
                    statuses.insert(service, tx);
                    rx
                }
            }
        };

        let stream = tokio_stream::wrappers::WatchStream::new(rx).map(|status| {
            Ok(HealthCheckResponse {
                status: status.into(),
            })
        });

        Ok(Response::new(Box::pin(stream)))
    }
}
