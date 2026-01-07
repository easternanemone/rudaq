// TODO: Fix doc comment links
#![allow(rustdoc::broken_intra_doc_links)]

pub mod grpc;
pub mod health;
#[cfg(feature = "modules")]
pub mod modules;
pub mod rerun_sink;

#[cfg(feature = "server")]
pub use grpc::server::DaqServer;

// Re-export Rerun types for server configuration
pub use rerun::{MemoryLimit, ServerOptions};
pub use rerun_sink::{APP_ID, DEFAULT_RERUN_PORT, RerunSink};
