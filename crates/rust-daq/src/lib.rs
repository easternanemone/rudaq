//! # Rust DAQ Integration Layer
//!
//! This crate serves as the **integration layer** for the `rust_daq` application, providing
//! organized re-exports and feature-gating for optional components. After the bd-232k refactoring,
//! this crate no longer owns implementation code—it orchestrates dependencies and provides
//! convenient import patterns.
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
//! grouped by functional area (core types, hardware, storage, scripting, etc.).
//!
//! **Or import directly from focused crates:**
//!
//! ```rust,ignore
//! use daq_core::error::DaqError;
//! use daq_storage::ring_buffer::RingBuffer;
//! use daq_hardware::capabilities::Movable;
//! ```
//!
//! ## Architecture (Post bd-232k Refactoring)
//!
//! **Key Changes:**
//! - `rust-daq` is now an **integration layer** (not a monolithic crate)
//! - Dead modules removed: `data/`, `metadata.rs`, `session.rs`, `measurement/` (-3,023 lines)
//! - Optional dependencies: `daq-server` and `daq-scripting` enabled via feature flags
//! - Root re-exports deprecated in favor of `prelude` module (will be removed in 0.6.0)
//!
//! **Module Organization:**
//!
//! - **[`prelude`]**: Organized re-exports grouped by functional area (core, hardware, storage, etc.)
//! - **[`config`]**: Application configuration structures (TOML-based, Figment integration)
//! - **[`validation`]**: Configuration parameter validation utilities
//! - **[`hardware`]**: Re-exported from `daq-hardware` (HAL, capability traits, drivers)
//! - **[`modules`]**: Module management for experiment-specific workflows (non-WASM only)
//! - **[`gui`]**: egui-based GUI components (requires `gui_egui` feature)
//! - **[`log_capture`]**: Log capture for GUI display (requires `gui_egui` feature, non-WASM only)
//!
//! **Deprecated Re-exports (will be removed in 0.6.0):**
//! - `rust_daq::core` → Use `rust_daq::prelude::core` or `daq_core::core`
//! - `rust_daq::error` → Use `rust_daq::prelude::error` or `daq_core::error`
//! - `rust_daq::observable` → Use `rust_daq::prelude::observable` or `daq_core::observable`
//! - `rust_daq::parameter` → Use `rust_daq::prelude::parameter` or `daq_core::parameter`
//! - `rust_daq::experiment` → Use `rust_daq::prelude::experiment` or `daq_experiment`
//! - `rust_daq::scripting` → Use `rust_daq::prelude::scripting` or `daq_scripting` (requires `scripting` feature)
//!
//! ## Feature Flags
//!
//! **Optional Components:**
//! - `scripting` - Enables `daq-scripting` dependency (Rhai engine integration)
//! - `server` - Enables `daq-server` dependency (gRPC server implementation)
//! - `modules` - Module system (depends on `scripting`)
//!
//! **High-Level Profiles:**
//! - `backend` - Server + modules + all hardware + CSV storage
//! - `frontend` - GUI (egui) + networking
//! - `cli` - All hardware + CSV storage + scripting
//! - `full` - Most features (excludes HDF5 which requires native libraries)
//!
//! See [`CLAUDE.md`](https://github.com/yourusername/rust-daq/blob/main/CLAUDE.md) for complete
//! documentation on the bd-232k refactoring and migration guide.

// TODO: Fix doc comment links and generic types
#![allow(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::invalid_html_tags)]
// TODO: Address clippy lints in dedicated refactoring pass
#![allow(clippy::redundant_field_names)]
#![allow(clippy::expect_used)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::unwrap_used)]

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
#[cfg(all(not(target_arch = "wasm32"), any(feature = "scripting", feature = "native_plugins")))]
pub mod plugins;

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
