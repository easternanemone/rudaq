//! gRPC client library for rust-daq daemon.
//!
//! This crate provides a high-level client for communicating with the rust-daq
//! daemon over gRPC. It is UI-agnostic and can be used by CLI tools, test
//! harnesses, and alternative frontends.

pub mod client;
pub mod connection;
pub mod error;
pub mod reconnect;

pub use client::{ChannelConfig, DaqClient};
pub use connection::{
    normalize_url, resolve_address, AddressError, AddressSource, DaemonAddress, DEFAULT_DAEMON_URL,
    DEFAULT_GRPC_PORT, STORAGE_KEY_DAEMON_ADDR,
};
pub use error::{ClientError, Result};
pub use reconnect::{ConnectionManager, ConnectionState, ReconnectConfig};
