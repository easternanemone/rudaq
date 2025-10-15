//! Core library for the rust_daq application.
//!
//! This library contains the core traits, data structures, and instrument
//! implementations for the DAQ system. It is used by both the main GUI
//! application and the Python bindings.

pub mod app;
pub mod config;
pub mod core;
pub mod data;
pub mod error;
pub mod gui;
pub mod instrument;
pub mod log_capture;
pub mod metadata;
pub mod session;
