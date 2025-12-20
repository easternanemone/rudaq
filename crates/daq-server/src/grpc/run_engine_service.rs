//! RunEngineService implementation (bd-w14j.2)
//!
//! Provides gRPC interface for the Bluesky-inspired RunEngine.
//! Enables declarative plan execution with pause/resume/abort capabilities.

use crate::grpc::proto::{
    run_engine_service_server::RunEngineService, AbortPlanRequest, AbortPlanResponse, EngineStatus,
    GetEngineStatusRequest, HaltEngineRequest, HaltEngineResponse, ListPlanTypesRequest,
    ListPlanTypesResponse, PauseEngineRequest, PauseEngineResponse, PlanTypeInfo, QueuePlanRequest,
    QueuePlanResponse, ResumeEngineRequest, ResumeEngineResponse, StartEngineRequest,
    StartEngineResponse, StreamDocumentsRequest,
};
use daq_experiment::run_engine::RunEngine;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tokio_stream::wrappers::BroadcastStream;
use tonic::{Request, Response, Status};

/// RunEngine gRPC service implementation.
///
/// Wraps the domain RunEngine and exposes its capabilities over gRPC.
#[derive(Debug, Clone)]
pub struct RunEngineServiceImpl {
    engine: Arc<RunEngine>,
}

impl RunEngineServiceImpl {
    /// Construct a new RunEngine service.
    pub fn new(engine: Arc<RunEngine>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl RunEngineService for RunEngineServiceImpl {
    async fn list_plan_types(
        &self,
        _request: Request<ListPlanTypesRequest>,
    ) -> Result<Response<ListPlanTypesResponse>, Status> {
        // Return hardcoded list of available plan types
        let plan_types = vec![
            PlanTypeInfo {
                plan_type: "count".to_string(),
                description: "Repeated measurements at current position".to_string(),
                parameters: vec!["num_points".to_string(), "delay".to_string()],
                required_devices: vec![],
            },
            PlanTypeInfo {
                plan_type: "line_scan".to_string(),
                description: "1D linear scan along a motor axis".to_string(),
                parameters: vec![
                    "motor".to_string(),
                    "start".to_string(),
                    "end".to_string(),
                    "num_points".to_string(),
                    "detector".to_string(),
                ],
                required_devices: vec!["motor".to_string(), "detector".to_string()],
            },
            PlanTypeInfo {
                plan_type: "grid_scan".to_string(),
                description: "2D grid scan over two motor axes".to_string(),
                parameters: vec![
                    "motor_x".to_string(),
                    "start_x".to_string(),
                    "end_x".to_string(),
                    "num_x".to_string(),
                    "motor_y".to_string(),
                    "start_y".to_string(),
                    "end_y".to_string(),
                    "num_y".to_string(),
                    "detector".to_string(),
                ],
                required_devices: vec![
                    "motor_x".to_string(),
                    "motor_y".to_string(),
                    "detector".to_string(),
                ],
            },
        ];

        Ok(Response::new(ListPlanTypesResponse { plan_types }))
    }

    async fn get_plan_type_info(
        &self,
        _request: Request<crate::grpc::proto::GetPlanTypeInfoRequest>,
    ) -> Result<Response<PlanTypeInfo>, Status> {
        Err(Status::unimplemented("get_plan_type_info not yet implemented"))
    }

    async fn queue_plan(
        &self,
        _request: Request<QueuePlanRequest>,
    ) -> Result<Response<QueuePlanResponse>, Status> {
        // For now, return unimplemented - requires plan factory
        Err(Status::unimplemented(
            "queue_plan not yet implemented - requires plan factory from script parameters",
        ))
    }

    async fn start_engine(
        &self,
        _request: Request<StartEngineRequest>,
    ) -> Result<Response<StartEngineResponse>, Status> {
        // Start the engine (spawns background task)
        self.engine
            .start()
            .await
            .map_err(|e| Status::internal(format!("Failed to start engine: {}", e)))?;

        Ok(Response::new(StartEngineResponse { success: true }))
    }

    async fn pause_engine(
        &self,
        _request: Request<PauseEngineRequest>,
    ) -> Result<Response<PauseEngineResponse>, Status> {
        self.engine
            .pause()
            .await
            .map_err(|e| Status::internal(format!("Failed to pause engine: {}", e)))?;

        Ok(Response::new(PauseEngineResponse { success: true }))
    }

    async fn resume_engine(
        &self,
        _request: Request<ResumeEngineRequest>,
    ) -> Result<Response<ResumeEngineResponse>, Status> {
        self.engine
            .resume()
            .await
            .map_err(|e| Status::internal(format!("Failed to resume engine: {}", e)))?;

        Ok(Response::new(ResumeEngineResponse { success: true }))
    }

    async fn abort_plan(
        &self,
        _request: Request<AbortPlanRequest>,
    ) -> Result<Response<AbortPlanResponse>, Status> {
        self.engine
            .abort()
            .await
            .map_err(|e| Status::internal(format!("Failed to abort plan: {}", e)))?;

        Ok(Response::new(AbortPlanResponse { success: true }))
    }

    async fn halt_engine(
        &self,
        _request: Request<HaltEngineRequest>,
    ) -> Result<Response<HaltEngineResponse>, Status> {
        self.engine
            .halt()
            .await
            .map_err(|e| Status::internal(format!("Failed to halt engine: {}", e)))?;

        Ok(Response::new(HaltEngineResponse { success: true }))
    }

    async fn get_engine_status(
        &self,
        _request: Request<GetEngineStatusRequest>,
    ) -> Result<Response<EngineStatus>, Status> {
        use crate::grpc::proto::EngineState as ProtoEngineState;
        use daq_experiment::run_engine::EngineState as DomainEngineState;

        let domain_state = self.engine.state().await;
        let queue_len = self.engine.queue_len().await as u32;

        let proto_state = match domain_state {
            DomainEngineState::Idle => ProtoEngineState::EngineIdle,
            DomainEngineState::Running => ProtoEngineState::EngineRunning,
            DomainEngineState::Paused => ProtoEngineState::EnginePaused,
            DomainEngineState::Aborting => ProtoEngineState::EngineAborting,
            DomainEngineState::Halted => ProtoEngineState::EngineHalted,
        };

        Ok(Response::new(EngineStatus {
            state: proto_state as i32,
            current_run_uid: None,
            current_plan_type: None,
            current_event_number: None,
            total_events_expected: None,
            queued_plans: queue_len,
        }))
    }

    type StreamDocumentsStream = BroadcastStream<Result<crate::grpc::proto::Document, Status>>;

    async fn stream_documents(
        &self,
        _request: Request<StreamDocumentsRequest>,
    ) -> Result<Response<Self::StreamDocumentsStream>, Status> {
        // Subscribe to document stream from RunEngine
        let rx = self.engine.subscribe();

        // Convert broadcast::Receiver to BroadcastStream and map documents
        let stream = BroadcastStream::new(rx).map(|result| match result {
            Ok(domain_doc) => {
                // Convert domain document to proto (will implement conversion below)
                domain_to_proto_document(domain_doc)
                    .map_err(|e| Status::internal(format!("Document conversion failed: {}", e)))
            }
            Err(e) => Err(Status::internal(format!("Document stream error: {}", e))),
        });

        Ok(Response::new(stream))
    }
}

/// Convert domain Document to proto Document
fn domain_to_proto_document(
    doc: daq_experiment::document::Document,
) -> Result<crate::grpc::proto::Document, String> {
    use crate::grpc::proto::{
        Document as ProtoDocument, DocumentType as ProtoDocType, EventDocument, StartDocument,
        StopDocument,
    };
    use daq_experiment::document::Document as DomainDoc;

    let (doc_type, payload) = match doc {
        DomainDoc::Start(start) => {
            let proto_start = StartDocument {
                uid: start.uid.clone(),
                time: start.time as f64,
                plan_type: start.plan_type,
                plan_name: start.plan_name,
                scan_id: start.scan_id as i64,
                metadata: start.metadata,
            };
            (
                ProtoDocType::DocStart as i32,
                Some(crate::grpc::proto::document::Payload::Start(proto_start)),
            )
        }
        DomainDoc::Stop(stop) => {
            let proto_stop = StopDocument {
                uid: stop.uid.clone(),
                run_uid: stop.run_uid,
                time: stop.time as f64,
                exit_status: stop.exit_status,
                reason: stop.reason.unwrap_or_default(),
                num_events: stop.num_events.map(|n| n as u32),
            };
            (
                ProtoDocType::DocStop as i32,
                Some(crate::grpc::proto::document::Payload::Stop(proto_stop)),
            )
        }
        DomainDoc::Event(event) => {
            let proto_event = EventDocument {
                uid: event.uid.clone(),
                descriptor_uid: event.descriptor_uid,
                time: event.time as f64,
                seq_num: event.seq_num as u32,
                data: event.data,
                timestamps: event.timestamps,
            };
            (
                ProtoDocType::DocEvent as i32,
                Some(crate::grpc::proto::document::Payload::Event(proto_event)),
            )
        }
        DomainDoc::Descriptor(_) => {
            // Descriptor not yet implemented in proto
            return Err("Descriptor documents not yet implemented".to_string());
        }
    };

    Ok(ProtoDocument {
        doc_type,
        uid: doc.uid().to_string(),
        timestamp_ns: 0, // Will need to extract from domain doc
        payload,
    })
}
