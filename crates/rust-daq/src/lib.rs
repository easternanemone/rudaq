//! # Rust DAQ Core Library
//!
//! This crate serves as the integration layer for the `rust_daq` application. It encapsulates all the
//! fundamental components required for data acquisition, instrument control, data processing,
//! and the graphical user interface. By organizing the project as a library, we can share
//! core logic between different frontends, such as the native GUI application (`main.rs`)
//! and potential future integrations like Python bindings.
//!
//! ## Recommended Usage
//!
//! **Use [`prelude`] for convenient imports:**
//!
//! ```rust,ignore
//! use rust_daq::prelude::*;
//! ```
//!
//! The prelude module provides organized re-exports from the entire `rust_daq` ecosystem,
//! avoiding import ambiguity and making it clear which crate each type comes from.
//!
//! ## Crate Structure
//!
//! The library is organized into several modules, each with a distinct responsibility:
//!
//! - **`config`**: Defines the structures for loading and validating application configuration
//!   from TOML files. See `config::Settings`.
//! - **`core`**: Re-exported from `daq-core`. Provides fundamental traits and types for the DAQ
//!   system, such as domain types and essential abstractions.
//! - **`data`**: Includes components for data handling, such as ring buffers and data processors
//!   (non-WASM only).
//! - **`error`**: Re-exported from `daq-core`. Defines the custom `DaqError` enum for centralized
//!   error handling across the application.
//! - **`experiment`**: Re-exported from `daq-experiment`. RunEngine and Plan definitions for
//!   experiment orchestration (non-WASM only).
//! - **`gui`**: egui-based GUI components (requires `gui_egui` feature).
//! - **`hardware`**: Re-exported from `daq-hardware`. Hardware Abstraction Layer with capability
//!   traits and device drivers (non-WASM only).
//! - **`log_capture`**: Provides a custom `log::Log` implementation to capture log messages for
//!   display within the GUI (requires `gui_egui` feature, non-WASM only).
//! - **`measurement`**: Measurement-related functionality (non-WASM only).
//! - **`metadata`**: Defines structures for capturing and storing experimental metadata.
//! - **`modules`**: Module management for experiment-specific workflows (non-WASM only).
//! - **`observable`**: Re-exported from `daq-core`. Observable pattern for reactive state.
//! - **`parameter`**: Re-exported from `daq-core`. Reactive Parameter<T> system with async hardware
//!   callbacks. All V5 drivers MUST implement Parameterized trait to expose parameters for gRPC
//!   control, presets, and experiment metadata. See docs/architecture/ADR_005_REACTIVE_PARAMETERS.md
//! - **`scripting`**: Re-exported from `daq-scripting`. Rhai scripting engine integration (non-WASM only).
//! - **`session`**: Implements session management for saving and loading the application state (non-WASM only).
//! - **`validation`**: A collection of utility functions for validating configuration parameters.

pub mod config;
pub mod prelude;

#[deprecated(
    since = "0.5.0",
    note = "Use `rust_daq::prelude::core` instead. Root re-exports will be removed in 0.6.0"
)]
pub use daq_core::core;

#[deprecated(
    since = "0.5.0",
    note = "Use `rust_daq::prelude::error` instead. Root re-exports will be removed in 0.6.0"
)]
pub use daq_core::error;
pub mod validation;

// Phase 1: Architectural redesign - New core abstractions
#[deprecated(
    since = "0.5.0",
    note = "Use `rust_daq::prelude::observable` instead. Root re-exports will be removed in 0.6.0"
)]
pub use daq_core::observable;

#[deprecated(
    since = "0.5.0",
    note = "Use `rust_daq::prelude::parameter` instead. Root re-exports will be removed in 0.6.0"
)]
pub use daq_core::parameter;

// V5 Headless-First Architecture (bd-oq51)
#[cfg(not(target_arch = "wasm32"))]
#[deprecated(
    since = "0.5.0",
    note = "Use `rust_daq::prelude::experiment` instead. Root re-exports will be removed in 0.6.0"
)]
pub use daq_experiment as experiment;

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

#[cfg(all(not(target_arch = "wasm32"), feature = "scripting"))]
#[deprecated(
    since = "0.5.0",
    note = "Use `rust_daq::prelude::scripting` instead. Root re-exports will be removed in 0.6.0"
)]
pub use daq_scripting as scripting;
#[cfg(target_arch = "wasm32")]
pub mod grpc {
    // Re-export generated types from daq-proto for WASM
    // This avoids needing to compile protos in the WASM build script
    pub use daq_proto::daq::*;

    // Explicitly re-export service clients if expected by consumer code
    pub use daq_proto::daq::hardware_service_client::HardwareServiceClient;
}
#[cfg(not(target_arch = "wasm32"))]
// pub mod health; // moved to daq-server
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
