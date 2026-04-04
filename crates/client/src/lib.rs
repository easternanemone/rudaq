//! gRPC client library for rust-daq daemon.
//!
//! This crate provides a high-level client for communicating with the rust-daq
//! daemon over gRPC. It is UI-agnostic and can be used by CLI tools, test
//! harnesses, and alternative frontends.
//!
//! On native platforms, uses HTTP/2 transport with auto-reconnect.
//! On WASM, uses gRPC-web transport via the browser's fetch API.

pub mod client;
pub mod connection;
pub mod error;
#[cfg(not(target_arch = "wasm32"))]
pub mod reconnect;

#[cfg(not(target_arch = "wasm32"))]
pub use client::ChannelConfig;
pub use client::DaqClient;
pub use client::Transport;
pub use connection::{
    AddressError, AddressSource, DEFAULT_DAEMON_URL, DEFAULT_GRPC_PORT, DaemonAddress,
    STORAGE_KEY_DAEMON_ADDR, normalize_url, resolve_address,
};
pub use error::{ClientError, Result};
#[cfg(not(target_arch = "wasm32"))]
pub use reconnect::{ConnectionManager, ConnectionState, ReconnectConfig};
