//! Protocol buffer definitions and conversions for rust-daq.
//!
//! This crate contains:
//! - Generated protobuf types from `proto/daq.proto` - Core DAQ service API
//! - Health check service from `proto/health.proto` - gRPC health checking
//! - NI DAQ extensions from `proto/ni_daq.proto` - NI-specific hardware access
//! - Conversion traits between proto types and domain types in `daq-core`
//!
//! # Architecture
//!
//! The proto types are kept separate from domain types to:
//! - Avoid transport-layer coupling in domain code
//! - Enable modules to work without networking features
//! - Provide clear boundaries for type conversions
//!
//! # Modules
//!
//! - [`daq`] - Core DAQ service (devices, configuration, streaming)
//! - [`health`] - gRPC health checking protocol
//! - [`ni_daq`] - NI DAQ-specific extensions for Comedi hardware
//! - [`convert`] - Type conversions between proto and domain types

#![allow(missing_docs)] // Generated code doesn't have docs

pub mod compression;
pub mod convert;
pub mod downsample;

/// Generated DAQ protocol buffer types.
pub mod daq {
    tonic::include_proto!("daq");
}

/// Generated health check protocol buffer types.
pub mod health {
    tonic::include_proto!("grpc.health.v1");
}

/// Generated NI DAQ protocol buffer types.
pub mod ni_daq {
    tonic::include_proto!("daq.ni_daq");
}

// Re-export commonly used types at crate root
pub use daq::*;
