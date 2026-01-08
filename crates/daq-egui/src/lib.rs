//! DAQ GUI Library
//!
//! Shared code between rust-daq-gui (standalone) and daq-rerun (embedded viewer).

pub mod client;
pub mod connection;
pub mod daemon_launcher;
pub mod reconnect;

#[cfg(feature = "standalone")]
pub mod app;
#[cfg(feature = "standalone")]
pub mod panels;
#[cfg(feature = "standalone")]
pub mod widgets;
