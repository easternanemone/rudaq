//! Core library for the Rust DAQ application.
//!
//! This crate contains the main application logic, including the core traits,
//! GUI implementation, instrument handling, and data processing pipelines.

pub mod app;
pub mod config;
pub mod core;
pub mod data;
pub mod error;
pub mod gui;
pub mod instrument;
pub mod log_capture;
