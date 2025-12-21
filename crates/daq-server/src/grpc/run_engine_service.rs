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
use daq_experiment::Document; // Re-exported from daq_core
use futures::StreamExt; // For .filter_map() with async
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tonic::{Request, Response, Status};

/// RunEngine gRPC service implementation.
///
/// Wraps the domain RunEngine and exposes its capabilities over gRPC.
#[derive(Clone)]
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
        use crate::grpc::proto::PlanTypeSummary;

        // Return hardcoded list of available plan types
        let plan_types = vec![
            PlanTypeSummary {
                type_id: "count".to_string(),
                display_name: "Count".to_string(),
                description: "Repeated measurements at current position".to_string(),
                categories: vec!["0d".to_string()],
            },
            PlanTypeSummary {
                type_id: "line_scan".to_string(),
                display_name: "Line Scan".to_string(),
                description: "1D linear scan along a motor axis".to_string(),
                categories: vec!["scanning".to_string(), "1d".to_string()],
            },
            PlanTypeSummary {
                type_id: "grid_scan".to_string(),
                display_name: "Grid Scan".to_string(),
                description: "2D grid scan over two motor axes".to_string(),
                categories: vec!["scanning".to_string(), "2d".to_string()],
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
        request: Request<QueuePlanRequest>,
    ) -> Result<Response<QueuePlanResponse>, Status> {
        let req = request.get_ref();

        // Create plan from request parameters
        let plan = create_plan_from_request(req)
            .map_err(|e| Status::invalid_argument(format!("Failed to create plan: {}", e)))?;

        // Queue the plan
        let run_uid = if req.metadata.is_empty() {
            self.engine.queue(plan).await
        } else {
            self.engine.queue_with_metadata(plan, req.metadata.clone()).await
        };

        let queue_len = self.engine.queue_len().await;

        Ok(Response::new(QueuePlanResponse {
            success: true,
            run_uid,
            error_message: String::new(),
            queue_position: queue_len as u32,
        }))
    }

    async fn start_engine(
        &self,
        _request: Request<StartEngineRequest>,
    ) -> Result<Response<StartEngineResponse>, Status> {
        // Start the engine (spawns background task)
        self.engine.start().await
            .map_err(|e| Status::internal(format!("Failed to start engine: {}", e)))?;

        Ok(Response::new(StartEngineResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    async fn pause_engine(
        &self,
        _request: Request<PauseEngineRequest>,
    ) -> Result<Response<PauseEngineResponse>, Status> {
        match self.engine.pause().await {
            Ok(_) => Ok(Response::new(PauseEngineResponse {
                success: true,
                paused_at: "checkpoint".to_string(),
            })),
            Err(e) => Err(Status::internal(format!("Failed to pause engine: {}", e))),
        }
    }

    async fn resume_engine(
        &self,
        _request: Request<ResumeEngineRequest>,
    ) -> Result<Response<ResumeEngineResponse>, Status> {
        self.engine.resume().await
            .map_err(|e| Status::internal(format!("Failed to resume engine: {}", e)))?;

        Ok(Response::new(ResumeEngineResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    async fn abort_plan(
        &self,
        request: Request<AbortPlanRequest>,
    ) -> Result<Response<AbortPlanResponse>, Status> {
        let _req = request.into_inner();

        // TODO: Support aborting specific run_uid (currently aborts current run only)
        // For now, ignore _req.run_uid and abort the currently executing plan
        self.engine.abort("user requested abort via gRPC").await
            .map_err(|e| Status::internal(format!("Failed to abort plan: {}", e)))?;

        Ok(Response::new(AbortPlanResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    async fn halt_engine(
        &self,
        _request: Request<HaltEngineRequest>,
    ) -> Result<Response<HaltEngineResponse>, Status> {
        self.engine.halt().await
            .map_err(|e| Status::internal(format!("Failed to halt engine: {}", e)))?;

        Ok(Response::new(HaltEngineResponse {
            halted: true,
            message: "Engine halted successfully".to_string(),
        }))
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
        };

        Ok(Response::new(EngineStatus {
            state: proto_state as i32,
            current_run_uid: None,
            current_plan_type: None,
            current_event_number: None,
            total_events_expected: None,
            queued_plans: queue_len,
            run_start_ns: 0, // TODO: Track run start time
            elapsed_ns: 0,   // TODO: Track elapsed time
        }))
    }

    type StreamDocumentsStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<crate::grpc::proto::Document, Status>> + Send>>;

    async fn stream_documents(
        &self,
        request: Request<StreamDocumentsRequest>,
    ) -> Result<Response<Self::StreamDocumentsStream>, Status> {
        // Subscribe to document stream from RunEngine
        let rx = self.engine.subscribe();

        // Extract filters from request
        let req = request.into_inner();
        let run_uid_filter = req.run_uid.filter(|s| !s.is_empty());
        let doc_types_filter = if req.doc_types.is_empty() {
            None
        } else {
            Some(req.doc_types)
        };

        // Convert broadcast::Receiver to BroadcastStream and map documents
        // Use filter_map to skip documents that can't be converted (e.g., Manifest)
        // and apply request filters
        let stream = BroadcastStream::new(rx).filter_map(move |result| {
            let run_uid_filter = run_uid_filter.clone();
            let doc_types_filter = doc_types_filter.clone();

            async move {
                match result {
                    Ok(domain_doc) => {
                        // Convert domain document to proto
                        match domain_to_proto_document(domain_doc) {
                            Ok(Some(proto_doc)) => {
                                // Apply run_uid filter
                                if let Some(ref filter_uid) = run_uid_filter {
                                    // Extract run_uid from document based on type
                                    let doc_run_uid = match &proto_doc.payload {
                                        Some(crate::grpc::proto::document::Payload::Start(s)) => Some(&s.run_uid),
                                        Some(crate::grpc::proto::document::Payload::Stop(s)) => Some(&s.run_uid),
                                        Some(crate::grpc::proto::document::Payload::Descriptor(d)) => Some(&d.run_uid),
                                        Some(crate::grpc::proto::document::Payload::Event(_)) => None, // Event has descriptor_uid, not run_uid
                                        None => None,
                                    };

                                    if let Some(uid) = doc_run_uid {
                                        if uid != filter_uid {
                                            return None; // Skip - different run
                                        }
                                    }
                                }

                                // Apply doc_types filter
                                if let Some(ref filter_types) = doc_types_filter {
                                    if !filter_types.contains(&proto_doc.doc_type) {
                                        return None; // Skip - type not in filter
                                    }
                                }

                                Some(Ok(proto_doc))
                            }
                            Ok(None) => None, // Skip this document (e.g., Manifest)
                            Err(e) => Some(Err(Status::internal(format!("Document conversion failed: {}", e)))),
                        }
                    }
                    Err(e) => {
                        // Check if this is a Lagged error by examining the error string
                        let err_str = format!("{:?}", e);
                        if err_str.contains("Lagged") {
                            // Receiver fell behind - log and continue without terminating stream
                            tracing::warn!(
                                "Document stream lagged: client too slow, skipping messages"
                            );
                            None // Skip, don't terminate
                        } else {
                            // Other errors (e.g., Closed) - terminate stream
                            Some(Err(Status::unavailable(format!("Document stream error: {}", e))))
                        }
                    }
                }
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }
}

/// Create a Plan from QueuePlanRequest parameters
fn create_plan_from_request(req: &QueuePlanRequest) -> Result<Box<dyn daq_experiment::plans::Plan>, String> {
    use daq_experiment::plans::{Count, GridScan, LineScan};

    match req.plan_type.as_str() {
        "count" => {
            // Parse count parameters
            let num_points = req.parameters.get("num_points")
                .ok_or("Missing parameter: num_points")?
                .parse::<usize>()
                .map_err(|e| format!("Invalid num_points: {}", e))?;

            // Validate parameters
            if num_points == 0 {
                return Err("num_points must be > 0".to_string());
            }

            let mut plan = Count::new(num_points);

            // Optional detector
            if let Some(detector) = req.device_mapping.get("detector") {
                if detector.is_empty() {
                    return Err("detector device name cannot be empty".to_string());
                }
                plan = plan.with_detector(detector);
            }

            // Optional delay
            if let Some(delay_str) = req.parameters.get("delay") {
                let delay = delay_str.parse::<f64>()
                    .map_err(|e| format!("Invalid delay: {}", e))?;
                if delay < 0.0 {
                    return Err("delay must be >= 0".to_string());
                }
                plan = plan.with_delay(delay);
            }

            Ok(Box::new(plan))
        }
        "line_scan" => {
            // Parse line scan parameters
            let start = req.parameters.get("start")
                .ok_or("Missing parameter: start")?
                .parse::<f64>()
                .map_err(|e| format!("Invalid start: {}", e))?;

            let end = req.parameters.get("end")
                .ok_or("Missing parameter: end")?
                .parse::<f64>()
                .map_err(|e| format!("Invalid end: {}", e))?;

            let num_points = req.parameters.get("num_points")
                .ok_or("Missing parameter: num_points")?
                .parse::<usize>()
                .map_err(|e| format!("Invalid num_points: {}", e))?;

            let motor = req.device_mapping.get("motor")
                .ok_or("Missing device mapping: motor")?;

            // Validate parameters
            if num_points == 0 {
                return Err("num_points must be > 0".to_string());
            }
            if start == end {
                return Err("start and end must be different for line scan".to_string());
            }
            if motor.is_empty() {
                return Err("motor device name cannot be empty".to_string());
            }

            let mut plan = LineScan::new(motor, start, end, num_points);

            // Optional detector
            if let Some(detector) = req.device_mapping.get("detector") {
                if detector.is_empty() {
                    return Err("detector device name cannot be empty".to_string());
                }
                plan = plan.with_detector(detector);
            }

            // Optional settle time
            if let Some(settle_str) = req.parameters.get("settle_time") {
                let settle = settle_str.parse::<f64>()
                    .map_err(|e| format!("Invalid settle_time: {}", e))?;
                if settle < 0.0 {
                    return Err("settle_time must be >= 0".to_string());
                }
                plan = plan.with_settle_time(settle);
            }

            Ok(Box::new(plan))
        }
        "grid_scan" => {
            // Parse grid scan parameters
            let x_start = req.parameters.get("x_start")
                .ok_or("Missing parameter: x_start")?
                .parse::<f64>()
                .map_err(|e| format!("Invalid x_start: {}", e))?;

            let x_end = req.parameters.get("x_end")
                .ok_or("Missing parameter: x_end")?
                .parse::<f64>()
                .map_err(|e| format!("Invalid x_end: {}", e))?;

            let x_points = req.parameters.get("x_points")
                .ok_or("Missing parameter: x_points")?
                .parse::<usize>()
                .map_err(|e| format!("Invalid x_points: {}", e))?;

            let y_start = req.parameters.get("y_start")
                .ok_or("Missing parameter: y_start")?
                .parse::<f64>()
                .map_err(|e| format!("Invalid y_start: {}", e))?;

            let y_end = req.parameters.get("y_end")
                .ok_or("Missing parameter: y_end")?
                .parse::<f64>()
                .map_err(|e| format!("Invalid y_end: {}", e))?;

            let y_points = req.parameters.get("y_points")
                .ok_or("Missing parameter: y_points")?
                .parse::<usize>()
                .map_err(|e| format!("Invalid y_points: {}", e))?;

            let x_motor = req.device_mapping.get("x_motor")
                .ok_or("Missing device mapping: x_motor")?;

            let y_motor = req.device_mapping.get("y_motor")
                .ok_or("Missing device mapping: y_motor")?;

            // Validate parameters
            if x_points == 0 {
                return Err("x_points must be > 0".to_string());
            }
            if y_points == 0 {
                return Err("y_points must be > 0".to_string());
            }
            if x_start == x_end {
                return Err("x_start and x_end must be different for grid scan".to_string());
            }
            if y_start == y_end {
                return Err("y_start and y_end must be different for grid scan".to_string());
            }
            if x_motor.is_empty() {
                return Err("x_motor device name cannot be empty".to_string());
            }
            if y_motor.is_empty() {
                return Err("y_motor device name cannot be empty".to_string());
            }
            if x_motor == y_motor {
                return Err("x_motor and y_motor must be different".to_string());
            }

            // Note: GridScan takes (outer/slow, inner/fast) axes
            // Convention: y is outer (slow), x is inner (fast)
            let mut plan = GridScan::new(
                y_motor,
                y_start,
                y_end,
                y_points,
                x_motor,
                x_start,
                x_end,
                x_points,
            );

            // Optional detector
            if let Some(detector) = req.device_mapping.get("detector") {
                if detector.is_empty() {
                    return Err("detector device name cannot be empty".to_string());
                }
                plan = plan.with_detector(detector);
            }

            // Optional snake scanning
            if let Some(snake_str) = req.parameters.get("snake") {
                let snake = snake_str.parse::<bool>()
                    .map_err(|e| format!("Invalid snake: {}", e))?;
                plan = plan.with_snake(snake);
            }

            Ok(Box::new(plan))
        }
        _ => Err(format!("Unknown plan type: {}", req.plan_type)),
    }
}

/// Convert domain Document to proto Document
/// Returns Ok(None) for documents that have no proto equivalent (e.g., Manifest)
fn domain_to_proto_document(
    doc: Document,
) -> Result<Option<crate::grpc::proto::Document>, String> {
    use crate::grpc::proto::{
        DataKey as ProtoDataKey, DescriptorDocument, Document as ProtoDocument,
        DocumentType as ProtoDocType, EventDocument, StartDocument, StopDocument,
    };
    use daq_experiment::Document as DomainDoc;

    let (doc_type, uid, timestamp_ns, payload) = match doc {
        DomainDoc::Start(start) => {
            let proto_start = StartDocument {
                run_uid: start.uid.clone(),
                plan_type: start.plan_type.clone(),
                plan_name: start.plan_name.clone(),
                plan_args: start.plan_args.clone(),
                metadata: start.metadata.clone(),
                hints: start.hints.clone(),
                time_ns: start.time_ns,
            };
            (
                ProtoDocType::DocStart as i32,
                start.uid,
                start.time_ns,
                Some(crate::grpc::proto::document::Payload::Start(proto_start)),
            )
        }
        DomainDoc::Stop(stop) => {
            let proto_stop = StopDocument {
                run_uid: stop.run_uid.clone(),
                exit_status: stop.exit_status.clone(),
                reason: stop.reason.clone(),
                time_ns: stop.time_ns,
                num_events: stop.num_events,
            };
            (
                ProtoDocType::DocStop as i32,
                stop.uid,
                stop.time_ns,
                Some(crate::grpc::proto::document::Payload::Stop(proto_stop)),
            )
        }
        DomainDoc::Event(event) => {
            let proto_event = EventDocument {
                descriptor_uid: event.descriptor_uid.clone(),
                seq_num: event.seq_num,
                time_ns: event.time_ns,
                data: event.data.clone(),
                timestamps: event.timestamps.clone(),
                bulk_data: std::collections::HashMap::new(), // TODO: Bulk data support
            };
            (
                ProtoDocType::DocEvent as i32,
                event.uid,
                event.time_ns,
                Some(crate::grpc::proto::document::Payload::Event(proto_event)),
            )
        }
        DomainDoc::Descriptor(desc) => {
            // Convert DataKey HashMap
            let proto_data_keys = desc
                .data_keys
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        ProtoDataKey {
                            dtype: v.dtype,
                            shape: v.shape,
                            source: v.source,
                            units: v.units,
                            precision: v.precision,
                        },
                    )
                })
                .collect();

            let proto_desc = DescriptorDocument {
                run_uid: desc.run_uid.clone(),
                descriptor_uid: desc.uid.clone(),
                name: desc.name.clone(),
                data_keys: proto_data_keys,
                configuration: desc.configuration.clone(),
            };
            (
                ProtoDocType::DocDescriptor as i32,
                desc.uid,
                desc.time_ns,
                Some(crate::grpc::proto::document::Payload::Descriptor(proto_desc)),
            )
        }
        DomainDoc::Manifest(_manifest) => {
            // Manifest has no proto equivalent - skip gracefully
            tracing::debug!("Skipping Manifest document (no proto mapping)");
            return Ok(None);
        }
    };

    Ok(Some(ProtoDocument {
        doc_type,
        uid,
        timestamp_ns,
        payload,
    }))
}
