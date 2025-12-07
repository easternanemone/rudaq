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
//! - **egui Desktop GUI**: The primary GUI is implemented as a separate binary (`rust_daq_gui_egui`)
//!   that uses `eframe` + `egui` and connects to the daemon over gRPC. It lives in `src/gui_main.rs`.
//! - **`instrument`**: Contains the concrete implementations of the `Instrument` trait for various
//!   hardware devices (e.g., mock instruments, VISA-based devices, cameras).
//! - **`log_capture`**: Provides a custom `log::Log` implementation to capture log messages for
//!   display within the GUI.
//! - **`metadata`**: Defines structures for capturing and storing experimental metadata.
//! - **`modules`**: Provides the `Module` trait for implementing experiment-specific workflows
//!   that orchestrate instruments to accomplish scientific tasks.
//! - **`session`**: Implements session management for saving and loading the application state.
//! - **`validation`**: A collection of utility functions for validating configuration parameters.
//! - **`parameter`**: Reactive Parameter<T> system with async hardware callbacks.
//!   All V5 drivers MUST implement Parameterized trait to expose parameters for
//!   gRPC control, presets, and experiment metadata. See docs/architecture/ADR_005_REACTIVE_PARAMETERS.md

pub mod config;
pub mod core;
pub mod data; // Re-enabled for ring buffer implementation (Phase 4J: bd-q2we)
pub mod error;
pub mod error_recovery;
pub mod log_capture;
pub mod measurement;
pub mod metadata;
pub mod session;
pub mod validation;

// Phase 1: Architectural redesign - New core abstractions
pub mod observable;
pub mod parameter; // Base reactive primitive (used by Parameter and Modules)

// V5 Headless-First Architecture (bd-oq51)
pub mod experiment; // Phase 4: RunEngine experiment orchestration (bd-73yh)
#[cfg(feature = "networking")]
pub mod grpc; // Re-enabled (bd-gmwv): gRPC services compile with networking feature
pub mod hardware; // Phase 1: Capability traits + Mock implementations
pub mod health;
pub mod scripting;
