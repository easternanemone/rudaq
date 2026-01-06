//! RunEngineService implementation (bd-w14j.2)
//!
//! Provides gRPC interface for the Bluesky-inspired RunEngine.
//! Enables declarative plan execution with pause/resume/abort capabilities.

use crate::grpc::proto::{
    AbortPlanRequest, AbortPlanResponse, EngineStatus, GetEngineStatusRequest, HaltEngineRequest,
    HaltEngineResponse, ListPlanTypesRequest, ListPlanTypesResponse, PauseEngineRequest,
    PauseEngineResponse, PlanTypeInfo, QueuePlanRequest, QueuePlanResponse, ResumeEngineRequest,
    ResumeEngineResponse, StartEngineRequest, StartEngineResponse, StreamDocumentsRequest,
    run_engine_service_server::RunEngineService,
};
use daq_experiment::Document; // Re-exported from daq_core
use daq_experiment::plans::{CountBuilder, GridScanBuilder, LineScanBuilder, PlanRegistry};
use daq_experiment::run_engine::RunEngine;
use daq_storage::DocumentWriter;
use futures::StreamExt; // For .filter_map() with async
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};
use tonic::{Request, Response, Status};

/// RunEngine gRPC service implementation.
///
/// Wraps the domain RunEngine and exposes its capabilities over gRPC.
///
/// Performance optimization (bd-p3k0): Converts domain documents to proto ONCE
/// and broadcasts to all clients, avoiding O(N×M) conversions for N clients and M events.
///
/// Observability (bd-f9hn): Tracks active streams and emits structured tracing events
/// for monitoring document throughput, lag events, and conversion performance.
pub struct RunEngineServiceImpl {
    engine: Arc<RunEngine>,
    /// Proto document broadcast (converted once, shared across all clients)
    proto_doc_sender: tokio::sync::broadcast::Sender<Arc<crate::grpc::proto::Document>>,
    /// Active stream count for observability (bd-f9hn)
    active_streams: Arc<AtomicU64>,
    /// Plan registry for dynamic plan creation
    plan_registry: Arc<PlanRegistry>,
    /// Persists documents to HDF5 (bd-jwsc)
    document_writer: Arc<DocumentWriter>,
}

impl RunEngineServiceImpl {
    /// Construct a new RunEngine service.
    ///
    /// Spawns a background task that converts domain documents to proto and broadcasts
    /// them to all gRPC clients. This ensures O(M) conversions instead of O(N×M).
    pub fn new(engine: Arc<RunEngine>) -> Self {
        // Create proto document broadcast channel
        let (proto_doc_sender, _) = tokio::sync::broadcast::channel(1024);

        // Create observability metrics
        let active_streams = Arc::new(AtomicU64::new(0));

        // Initialize plan registry
        let mut registry = PlanRegistry::new();
        registry.register("count", CountBuilder);
        registry.register("line_scan", LineScanBuilder);
        registry.register("grid_scan", GridScanBuilder);
        let plan_registry = Arc::new(registry);

        // Initialize document writer (data stored in ./data directory)
        let data_dir = std::path::Path::new("data").to_path_buf();
        std::fs::create_dir_all(&data_dir).ok(); // Ensure directory exists
        let document_writer = Arc::new(DocumentWriter::new(data_dir));

        // Spawn persistence task (bd-jwsc)
        let engine_clone_writer = engine.clone();
        let writer_clone = document_writer.clone();
        tokio::spawn(async move {
            let mut domain_rx = engine_clone_writer.subscribe();
            loop {
                match domain_rx.recv().await {
                    Ok(doc) => {
                        // Forward to writer (handles HDF5 interaction on blocking thread)
                        if let Err(e) = writer_clone.write(doc).await {
                            tracing::error!(error = %e, "Failed to persist document");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "Persistence task lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        // Spawn converter task that subscribes to domain stream and broadcasts proto
        let engine_clone = engine.clone();
        let proto_sender_clone = proto_doc_sender.clone();
        tokio::spawn(async move {
            let mut domain_rx = engine_clone.subscribe();
            let mut total_converted = 0u64;

            loop {
                match domain_rx.recv().await {
                    Ok(domain_doc) => {
                        // Convert domain → proto (ONCE for all clients)
                        let start = std::time::Instant::now();
                        match domain_to_proto_document(domain_doc) {
                            Ok(Some(proto_doc)) => {
                                let conversion_micros = start.elapsed().as_micros();
                                total_converted += 1;

                                // Broadcast Arc to avoid cloning the proto doc for each client
                                let subscriber_count = proto_sender_clone.receiver_count();
                                let send_result = proto_sender_clone.send(Arc::new(proto_doc));

                                // Observability: Log throughput periodically
                                if total_converted.is_multiple_of(1000) {
                                    tracing::info!(
                                        total_converted,
                                        subscriber_count,
                                        conversion_micros,
                                        "Document converter throughput"
                                    );
                                }

                                // Warn if no subscribers (documents being dropped)
                                if send_result.is_err() || subscriber_count == 0 {
                                    tracing::debug!(
                                        total_converted,
                                        "Document broadcast has no subscribers"
                                    );
                                }
                            }
                            Ok(None) => {
                                // Skip documents that don't convert (e.g., Manifest)
                                tracing::trace!(
                                    "Skipped non-convertible document (e.g., Manifest)"
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    total_converted,
                                    "Failed to convert domain document to proto"
                                );
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            skipped,
                            total_converted,
                            "Converter task lagged, skipped domain documents"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!(
                            total_converted,
                            "RunEngine document stream closed, stopping converter task"
                        );
                        break;
                    }
                }
            }
        });

        Self {
            engine,
            proto_doc_sender,
            active_streams,
            plan_registry,
            document_writer,
        }
    }
}

impl Clone for RunEngineServiceImpl {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            proto_doc_sender: self.proto_doc_sender.clone(),
            active_streams: self.active_streams.clone(),
            plan_registry: self.plan_registry.clone(),
            document_writer: self.document_writer.clone(),
        }
    }
}

#[tonic::async_trait]
impl RunEngineService for RunEngineServiceImpl {
    async fn list_plan_types(
        &self,
        _request: Request<ListPlanTypesRequest>,
    ) -> Result<Response<ListPlanTypesResponse>, Status> {
        use crate::grpc::proto::PlanTypeSummary;

        // Get available plan types from registry
        let plan_types = self
            .plan_registry
            .list_types()
            .into_iter()
            .map(|(type_id, description, categories)| {
                let display_name = match type_id.as_str() {
                    "count" => "Count",
                    "line_scan" => "Line Scan",
                    "grid_scan" => "Grid Scan",
                    s => s, // Fallback to ID
                }
                .to_string();

                PlanTypeSummary {
                    type_id,
                    display_name,
                    description,
                    categories,
                }
            })
            .collect();

        Ok(Response::new(ListPlanTypesResponse { plan_types }))
    }

    async fn get_plan_type_info(
        &self,
        _request: Request<crate::grpc::proto::GetPlanTypeInfoRequest>,
    ) -> Result<Response<PlanTypeInfo>, Status> {
        Err(Status::unimplemented(
            "get_plan_type_info not yet implemented",
        ))
    }

    async fn queue_plan(
        &self,
        request: Request<QueuePlanRequest>,
    ) -> Result<Response<QueuePlanResponse>, Status> {
        let req = request.get_ref();

        // Create plan from request parameters using the registry
        let plan = self
            .plan_registry
            .create_plan(&req.plan_type, &req.parameters, &req.device_mapping)
            .map_err(|e| Status::invalid_argument(format!("Failed to create plan: {}", e)))?;

        // Queue the plan
        let run_uid = if req.metadata.is_empty() {
            self.engine.queue(plan).await
        } else {
            self.engine
                .queue_with_metadata(plan, req.metadata.clone())
                .await
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
        self.engine
            .start()
            .await
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
        self.engine
            .resume()
            .await
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
        let req = request.into_inner();

        // Support aborting specific run_uid or current if empty (bd-vi16.3)
        let run_uid = if req.run_uid.is_empty() {
            None
        } else {
            Some(req.run_uid.as_str())
        };

        self.engine
            .abort_run(run_uid, "user requested abort via gRPC")
            .await
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
        self.engine
            .halt()
            .await
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

        // Get run timing information
        let run_start_ns = self.engine.current_run_start_ns().await.unwrap_or(0);
        let elapsed_ns = if run_start_ns > 0 {
            let now_ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            now_ns.saturating_sub(run_start_ns)
        } else {
            0
        };

        Ok(Response::new(EngineStatus {
            state: proto_state as i32,
            current_run_uid: None,
            current_plan_type: None,
            current_event_number: None,
            total_events_expected: None,
            queued_plans: queue_len,
            run_start_ns,
            elapsed_ns,
        }))
    }

    type StreamDocumentsStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<crate::grpc::proto::Document, Status>> + Send>,
    >;

    async fn stream_documents(
        &self,
        request: Request<StreamDocumentsRequest>,
    ) -> Result<Response<Self::StreamDocumentsStream>, Status> {
        // Subscribe to proto document broadcast (already converted by background task)
        // Performance: O(M) conversions instead of O(N×M) for N clients, M events
        let proto_rx = self.proto_doc_sender.subscribe();

        // Extract filters from request
        let req = request.into_inner();
        let run_uid_filter = req.run_uid.filter(|s| !s.is_empty()).map(Arc::new);

        // Wrap doc_types filter in Arc to avoid cloning Vec on every document
        let doc_types_filter: Option<Arc<Vec<i32>>> = if req.doc_types.is_empty() {
            None
        } else {
            Some(Arc::new(req.doc_types))
        };

        // Maintain descriptor_uid → run_uid mapping for Event filtering
        // Events only have descriptor_uid, need to look up run_uid
        let descriptor_to_run_map = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::<
            String,
            String,
        >::new()));

        // Observability: Track active streams (bd-f9hn)
        let active_streams = self.active_streams.clone();
        let stream_count = active_streams.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(
            active_streams = stream_count,
            run_uid_filter = ?run_uid_filter,
            doc_types_filter = ?doc_types_filter,
            "New document stream client connected"
        );

        // Metrics: Track filter matches and rejections per client
        let docs_received = Arc::new(AtomicU64::new(0));
        let docs_filtered = Arc::new(AtomicU64::new(0));
        let docs_sent = Arc::new(AtomicU64::new(0));
        let lag_events = Arc::new(AtomicU64::new(0));

        // Clone metrics for use outside closure
        let docs_received_outer = docs_received.clone();
        let docs_sent_outer = docs_sent.clone();
        let docs_filtered_outer = docs_filtered.clone();
        let lag_events_outer = lag_events.clone();

        // Apply client-specific filters to the shared proto stream
        let stream = BroadcastStream::new(proto_rx).filter_map(move |result| {
            let run_uid_filter = run_uid_filter.clone();
            let doc_types_filter = doc_types_filter.clone(); // Arc clone, cheap
            let descriptor_map = descriptor_to_run_map.clone();
            let docs_received = docs_received.clone();
            let docs_filtered = docs_filtered.clone();
            let docs_sent = docs_sent.clone();
            let lag_events = lag_events.clone();

            async move {
                match result {
                    Ok(proto_doc_arc) => {
                        // Observability: Track received documents
                        let received = docs_received.fetch_add(1, Ordering::Relaxed) + 1;

                        // Document already converted to proto by background task
                        // Apply run_uid filter
                        if let Some(ref filter_uid) = run_uid_filter {
                            // Extract run_uid from document based on type
                            let doc_run_uid = match &proto_doc_arc.payload {
                                Some(crate::grpc::proto::document::Payload::Start(s)) => {
                                    Some(s.run_uid.clone())
                                }
                                Some(crate::grpc::proto::document::Payload::Stop(s)) => {
                                    Some(s.run_uid.clone())
                                }
                                Some(crate::grpc::proto::document::Payload::Descriptor(d)) => {
                                    // Record descriptor_uid → run_uid mapping
                                    let mut map = descriptor_map.lock().await;
                                    map.insert(d.descriptor_uid.clone(), d.run_uid.clone());
                                    Some(d.run_uid.clone())
                                }
                                Some(crate::grpc::proto::document::Payload::Event(e)) => {
                                    // Look up run_uid via descriptor_uid
                                    let map = descriptor_map.lock().await;
                                    map.get(&e.descriptor_uid).cloned()
                                }
                                None => None,
                            };

                            if let Some(uid) = doc_run_uid {
                                if uid.as_str() != filter_uid.as_str() {
                                    docs_filtered.fetch_add(1, Ordering::Relaxed);
                                    return None; // Skip - different run
                                }
                            } else if matches!(&proto_doc_arc.payload, Some(crate::grpc::proto::document::Payload::Event(_))) {
                                // Event with unknown descriptor - drop when filter active
                                docs_filtered.fetch_add(1, Ordering::Relaxed);
                                tracing::debug!(
                                    "Dropping Event with unknown descriptor_uid (run_uid filter active)"
                                );
                                return None;
                            }
                        }

                        // Apply doc_types filter
                        if let Some(ref filter_types) = doc_types_filter
                            && !filter_types.contains(&proto_doc_arc.doc_type) {
                                docs_filtered.fetch_add(1, Ordering::Relaxed);
                                return None; // Skip - type not in filter
                            }

                        // Observability: Track sent documents and log periodically
                        let sent = docs_sent.fetch_add(1, Ordering::Relaxed) + 1;
                        if sent.is_multiple_of(100) {
                            let filtered = docs_filtered.load(Ordering::Relaxed);
                            let filter_rate = if received > 0 {
                                (sent as f64 / received as f64) * 100.0
                            } else {
                                0.0
                            };
                            tracing::info!(
                                docs_received = received,
                                docs_sent = sent,
                                docs_filtered = filtered,
                                filter_match_rate_percent = format!("{:.1}", filter_rate),
                                "Client stream metrics"
                            );
                        }

                        // Clone the proto doc for this client (Arc::deref + clone)
                        // This is cheaper than re-converting from domain
                        Some(Ok((*proto_doc_arc).clone()))
                    }
                    Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                        // Receiver fell behind - log and continue without terminating stream
                        let lag_count = lag_events.fetch_add(1, Ordering::Relaxed) + 1;
                        let received = docs_received.load(Ordering::Relaxed);
                        let sent = docs_sent.load(Ordering::Relaxed);
                        tracing::warn!(
                            skipped,
                            lag_count,
                            docs_received = received,
                            docs_sent = sent,
                            "Document stream lagged: client too slow, skipped messages"
                        );
                        None // Skip, don't terminate
                    }
                    // Note: BroadcastStreamRecvError does not have a Closed variant
                    // The stream ends with None when the sender is dropped
                    // This exhaustive match ensures we handle all actual error cases
                }
            }
        });

        // Wrap stream to decrement active_streams on drop
        let active_streams_cleanup = self.active_streams.clone();
        let wrapped_stream = StreamWithCleanup {
            inner: Box::pin(stream),
            active_streams: active_streams_cleanup,
            docs_received: docs_received_outer,
            docs_sent: docs_sent_outer,
            docs_filtered: docs_filtered_outer,
            lag_events: lag_events_outer,
        };

        Ok(Response::new(Box::pin(wrapped_stream)))
    }
}

/// Wrapper to decrement active_streams counter when stream is dropped
struct StreamWithCleanup<S> {
    inner: std::pin::Pin<Box<S>>,
    active_streams: Arc<AtomicU64>,
    docs_received: Arc<AtomicU64>,
    docs_sent: Arc<AtomicU64>,
    docs_filtered: Arc<AtomicU64>,
    lag_events: Arc<AtomicU64>,
}

impl<S> Drop for StreamWithCleanup<S> {
    fn drop(&mut self) {
        let remaining = self.active_streams.fetch_sub(1, Ordering::Relaxed) - 1;
        let received = self.docs_received.load(Ordering::Relaxed);
        let sent = self.docs_sent.load(Ordering::Relaxed);
        let filtered = self.docs_filtered.load(Ordering::Relaxed);
        let lags = self.lag_events.load(Ordering::Relaxed);

        tracing::info!(
            active_streams = remaining,
            docs_received = received,
            docs_sent = sent,
            docs_filtered = filtered,
            lag_events = lags,
            "Document stream client disconnected"
        );
    }
}

impl<S> tokio_stream::Stream for StreamWithCleanup<S>
where
    S: tokio_stream::Stream,
{
    type Item = S::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// Convert domain Document to proto Document
/// Returns Ok(None) for documents that have no proto equivalent (e.g., Manifest)
fn domain_to_proto_document(doc: Document) -> Result<Option<crate::grpc::proto::Document>, String> {
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
                // Middle-data support (bd-9unn)
                metadata: event.metadata.clone(),
                arrays: event.arrays.clone(),
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
                Some(crate::grpc::proto::document::Payload::Descriptor(
                    proto_desc,
                )),
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
