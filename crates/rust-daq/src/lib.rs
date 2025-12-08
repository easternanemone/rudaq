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

pub use daq_core::core;
use std::borrow;
#[cfg(not(target_arch = "wasm32"))]
pub mod data; // Re-enabled for ring buffer implementation (Phase 4J: bd-q2we)
pub use daq_core::error;
#[cfg(not(target_arch = "wasm32"))]
pub mod error_recovery;
pub mod measurement;
pub mod metadata;
#[cfg(not(target_arch = "wasm32"))]
pub mod session;
pub mod validation;

// Phase 1: Architectural redesign - New core abstractions
pub use daq_core::observable;
pub use daq_core::parameter;

// V5 Headless-First Architecture (bd-oq51)
#[cfg(not(target_arch = "wasm32"))]
pub mod experiment; // Phase 4: RunEngine experiment orchestration (bd-73yh)
// pub mod app; // Removed (legacy)
#[cfg(feature = "gui_egui")]
pub mod gui;
#[cfg(not(target_arch = "wasm32"))]
pub mod hardware;
// pub mod instrument; // Removed (legacy)
#[cfg(not(target_arch = "wasm32"))]
#[cfg(all(not(target_arch = "wasm32"), feature = "gui_egui"))]
pub mod log_capture;
#[cfg(not(target_arch = "wasm32"))]
pub mod modules;
#[cfg(not(target_arch = "wasm32"))]
pub mod scripting;
#[cfg(not(target_arch = "wasm32"))]
pub mod grpc; // gRPC server & protobuf types
#[cfg(target_arch = "wasm32")]
pub mod grpc {
    // Re-export generated modules for WASM (if build script generated them)
    // Note: We need a way to include the generated files without the full 'tonic' server stack
    // Usually tonic::include_proto! works if tonic-build was used.
    // We might need to manually include them or use a subset of grpc module.
    // For now, let's try assuming include_proto works with limited tonic deps.
    pub mod daq {
        tonic::include_proto!("daq");
    }
    pub use daq::*; // Re-export generated types to crate::grpc scope
    // Explicitly re-export service clients if they are in submodules
    pub use daq::hardware_service_client::HardwareServiceClient;
}
#[cfg(not(target_arch = "wasm32"))]
pub mod health;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub async fn start() -> Result<(), wasm_bindgen::JsValue> {
    web_sys::console::log_1(&"🚀 WASM Started!".into());
    let canvas_id = "the_canvas_id";
    console_error_panic_hook::set_once();
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let document = web_sys::window().unwrap().document().unwrap();
    let canvas = document
        .get_element_by_id(&canvas_id)
        .ok_or("Canvas not found")?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| "Element is not a canvas")?;

    let web_options = eframe::WebOptions::default();

    eframe::WebRunner::new()
        .start(
            canvas,
            web_options,
            Box::new(|_cc| Ok(Box::new(gui::app::DaqGuiApp::new()) as Box<dyn eframe::App>)),
        )
        .await
        .map_err(|e| format!("{:?}", e).into())
}
