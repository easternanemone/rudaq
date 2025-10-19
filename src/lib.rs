//! # Rust DAQ Core Library
//!
//! This crate serves as the core library for the `rust_daq` application. It encapsulates all the
//! fundamental components required for data acquisition, instrument control, data processing,
//! and the graphical user interface. By organizing the project as a library, we can share
//! core logic between different frontends, such as the native GUI application (`main.rs`)
//! and potential future integrations like Python bindings.
//!
//! ## Crate Structure
//!
//! The library is organized into several modules, each with a distinct responsibility:
//!
//! - **`app`**: Contains the main `DaqApp` struct, which acts as the central hub of the
//!   application, managing state, instruments, and data flow.
//! - **`config`**: Defines the structures for loading and validating application configuration
//!   from TOML files. See `config::Settings`.
//! - **`core`**: Provides the fundamental traits and enums for the DAQ system, such as `Instrument`,
//!   `DataPoint`, and `InstrumentCommand`. This module defines the essential abstractions.
//! - **`data`**: Includes components for data handling, such as storage writers (e.g., CSV, HDF5)
//!   and data processors.
//! - **`error`**: Defines the custom `DaqError` enum for centralized error handling across the
//!   application.
//! - **`gui`**: Implements the native graphical user interface using `eframe` and `egui`. It contains
//!   all the UI components, panels, and docking logic.
//! - **`instrument`**: Contains the concrete implementations of the `Instrument` trait for various
//!   hardware devices (e.g., mock instruments, VISA-based devices, cameras).
//! - **`log_capture`**: Provides a custom `log::Log` implementation to capture log messages for
//!   display within the GUI.
//! - **`metadata`**: Defines structures for capturing and storing experimental metadata.
//! - **`session`**: Implements session management for saving and loading the application state.
//! - **`validation`**: A collection of utility functions for validating configuration parameters.

pub mod app;
pub mod app_actor;
pub mod config;
pub mod core;
pub mod data;
pub mod error;
pub mod gui;
pub mod instrument;
pub mod log_capture;
pub mod measurement;
pub mod messages;
pub mod metadata;
pub mod session;
pub mod validation;

// New v2 architecture modules (Phase 1)
pub mod adapters;
pub mod instruments_v2;
