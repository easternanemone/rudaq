use crate::grpc::proto::run_engine_service_server::RunEngineServiceServer;
use crate::grpc::proto::{DaemonInfoRequest, DaemonInfoResponse, SystemStatus};
#[cfg(feature = "scripting")]
use crate::grpc::proto::{
    ListExecutionsRequest, ListExecutionsResponse, ListScriptsRequest, ListScriptsResponse,
    ScriptInfo, ScriptStatus, StartRequest, StartResponse, StatusRequest, StopRequest,
    StopResponse,
    control_service_server::{ControlService, ControlServiceServer},
};
use crate::grpc::run_engine_service::RunEngineServiceImpl;
#[cfg(feature = "serial")]
use crate::grpc::{PluginServiceImpl, PluginServiceServer};
use common::core::Measurement;
#[cfg(feature = "scripting")]
use common::limits;
#[cfg(feature = "scripting")]
use scripting::ScriptEngine; // Trait import
// use common::error::DaqError; // Unused
#[cfg(feature = "scripting")]
use experiment::RunEngine;
use protocol::daq::{UploadRequest, UploadResponse};
#[cfg(feature = "scripting")]
use scripting::RhaiEngine;
#[cfg(feature = "scripting")]
use scripting::ScriptPlanRunner;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(feature = "storage_hdf5")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::System;
#[cfg(feature = "storage_hdf5")]
use tokio::sync::mpsc;
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tonic::service::interceptor::interceptor;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use uuid::Uuid;

#[cfg(test)]
use crate::config::GrpcSettings;
use crate::config::ServerConfig;

mod auth;
mod daq_server;
mod measurement_pipeline;
mod startup;
mod web_ui;

#[cfg(test)]
mod tests;

pub use daq_server::DaqServer;
pub use startup::{start_server, start_server_with_hardware};

use auth::{build_cors_layer, build_tls_config, validate_auth};
#[cfg(test)]
use measurement_pipeline::ImageSequenceFault;
use measurement_pipeline::{
    annotate_measurement_data_integrity, build_image_measurement, encode_measurement_frame,
    load_echelle_profile_for_device, log_data_integrity_fault, metadata_string,
    spectrum_payload_from_parts,
};
use web_ui::WebUiLayer;
// C1 decomposition map (2026-04):
//   - `daq_server`           - `DaqServer` struct + impls + `ControlService` (4a)
//   - `startup`              - `build_grpc_server!` macro + `start_server*`    (3)
//   - `auth`                 - JWT/TLS/CORS helpers                            (2)
//   - `measurement_pipeline` - spectrum/image payload + integrity checks       (2)
//   - `web_ui`               - static-file Tower layer for WASM UI             (2)
//   - `tests`                - parent-level integration tests                  (4b)
