//! RunEngineService implementation placeholder.
//!
//! The protocol (`proto/daq.proto`) already exposes the RunEngine surface so
//! clients such as the Slint GUI eagerly construct a `RunEngineServiceClient`.
//! Until the real bluesky-inspired RunEngine is delivered we still register a
//! gRPC service that responds explicitly with `UNIMPLEMENTED` errors. This
//! prevents hanging RPCs and gives callers a deterministic capability signal.

use crate::grpc::proto::{
    run_engine_service_server::RunEngineService, AbortPlanRequest, AbortPlanResponse, EngineStatus,
    GetEngineStatusRequest, HaltEngineRequest, HaltEngineResponse, ListPlanTypesRequest,
    ListPlanTypesResponse, PauseEngineRequest, PauseEngineResponse, PlanTypeInfo, PlanTypeSummary,
    QueuePlanRequest, QueuePlanResponse, ResumeEngineRequest, ResumeEngineResponse,
    StartEngineRequest, StartEngineResponse, StreamDocumentsRequest,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

/// Stub RunEngine service.
///
/// All RPCs currently return `UNIMPLEMENTED`. The stub is still valuable because
/// it advertises feature parity to clients without forcing them to derive
/// availability from connection errors.
#[derive(Debug, Default, Clone)]
pub struct RunEngineServiceImpl;

impl RunEngineServiceImpl {
    /// Construct a new stub RunEngine service.
    pub fn new() -> Self {
        Self
    }

    /// Convenience helper to construct a uniform UNIMPLEMENTED response.
    fn unimplemented<T>() -> Result<Response<T>, Status> {
        Err(Status::unimplemented(
            "RunEngine service is not yet implemented on this build",
        ))
    }
}

#[tonic::async_trait]
impl RunEngineService for RunEngineServiceImpl {
    async fn list_plan_types(
        &self,
        _request: Request<ListPlanTypesRequest>,
    ) -> Result<Response<ListPlanTypesResponse>, Status> {
        Self::unimplemented()
    }

    async fn get_plan_type_info(
        &self,
        _request: Request<crate::grpc::proto::GetPlanTypeInfoRequest>,
    ) -> Result<Response<PlanTypeInfo>, Status> {
        Self::unimplemented()
    }

    async fn queue_plan(
        &self,
        _request: Request<QueuePlanRequest>,
    ) -> Result<Response<QueuePlanResponse>, Status> {
        Self::unimplemented()
    }

    async fn start_engine(
        &self,
        _request: Request<StartEngineRequest>,
    ) -> Result<Response<StartEngineResponse>, Status> {
        Self::unimplemented()
    }

    async fn pause_engine(
        &self,
        _request: Request<PauseEngineRequest>,
    ) -> Result<Response<PauseEngineResponse>, Status> {
        Self::unimplemented()
    }

    async fn resume_engine(
        &self,
        _request: Request<ResumeEngineRequest>,
    ) -> Result<Response<ResumeEngineResponse>, Status> {
        Self::unimplemented()
    }

    async fn abort_plan(
        &self,
        _request: Request<AbortPlanRequest>,
    ) -> Result<Response<AbortPlanResponse>, Status> {
        Self::unimplemented()
    }

    async fn halt_engine(
        &self,
        _request: Request<HaltEngineRequest>,
    ) -> Result<Response<HaltEngineResponse>, Status> {
        Self::unimplemented()
    }

    async fn get_engine_status(
        &self,
        _request: Request<GetEngineStatusRequest>,
    ) -> Result<Response<EngineStatus>, Status> {
        Self::unimplemented()
    }

    type StreamDocumentsStream = ReceiverStream<Result<crate::grpc::proto::Document, Status>>;

    async fn stream_documents(
        &self,
        _request: Request<StreamDocumentsRequest>,
    ) -> Result<Response<Self::StreamDocumentsStream>, Status> {
        Self::unimplemented()
    }
}
