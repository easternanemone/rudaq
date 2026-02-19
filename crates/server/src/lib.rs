//! # daq-server
//!
//! gRPC server for rust-daq providing remote hardware control and data streaming.
//!
//! This crate exposes the rust-daq system over gRPC, enabling:
//!
//! - **Remote Hardware Control** - Device operations over network
//! - **Frame Streaming** - Adaptive quality video with bandwidth optimization
//! - **Script Execution** - Run Rhai experiments remotely
//! - **Plan Execution** - Bluesky-style experiment plans with pause/resume
//!
//! ## gRPC Services
//!
//! | Service | Purpose |
//! |---------|---------|
//! | `HardwareService` | Direct device control (move, read, trigger) |
//! | `ControlService` | Script upload, validation, and execution |
//! | `RunEngineService` | Plan execution with pause/resume/abort |
//! | `PresetService` | Save/load device configuration presets |
//!
//! ## Quick Example
//!
//! ```rust,ignore
//! use daq_server::DaqServer;
//! use daq_hardware::DeviceRegistry;
//!
//! let registry = Arc::new(DeviceRegistry::new());
//! let server = DaqServer::new(registry)?;
//! server.serve("0.0.0.0:50051").await?;
//! ```
//!
//! ## Feature Flags
//!
//! - `server` - Core gRPC server functionality
//! - `scripting` - Rhai script execution support
//! - `modules` - Device module lifecycle management
//! - `rerun_sink` - Rerun visualization integration
//! - `storage_hdf5` - HDF5 persistence
//!
//! [`DaqServer`]: grpc::server::DaqServer

// TODO: Fix doc comment links
#![allow(rustdoc::broken_intra_doc_links)]
// Protobuf/tonic generated code triggers these clippy lints; suppress at crate level (bd-m5fh.4.2).
#![allow(clippy::mixed_attributes_style)]
#![allow(clippy::redundant_field_names)]
#![allow(clippy::result_large_err)]
#![allow(clippy::single_match)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::vec_init_then_push)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::io_other_error)]

pub mod config;
pub mod grpc;
pub mod health;
#[cfg(feature = "rerun_sink")]
pub mod rerun_sink;
#[cfg(feature = "scripting")]
pub mod script_journal;

#[cfg(feature = "server")]
pub use grpc::server::DaqServer;

// Re-export Rerun types for server configuration
#[cfg(feature = "rerun_sink")]
pub use rerun::{MemoryLimit, ServerOptions};
#[cfg(feature = "rerun_sink")]
pub use rerun_sink::{APP_ID, DEFAULT_RERUN_PORT, RerunSink};
