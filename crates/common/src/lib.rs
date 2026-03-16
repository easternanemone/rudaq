//! # common
//!
//! Core abstraction layer for the rust-daq data acquisition system.
//!
//! This crate provides foundational types and traits used throughout the ecosystem:
//!
//! - **Capability Traits** - [`Movable`], [`Readable`], [`FrameProducer`], [`Triggerable`]
//! - **Reactive Parameters** - [`Observable`] and [`Parameter<T>`] with validation
//! - **Error Model** - [`DaqError`] with categorized errors and recovery strategies
//! - **Driver Plugin System** - [`DriverFactory`] for dynamic hardware registration
//! - **Frame Data** - [`Frame`], [`PixelBuffer`], zero-copy data handling
//!
//! ## Quick Example
//!
//! ```rust,ignore
//! use common::observable::Observable;
//! use common::capabilities::Movable;
//!
//! // Reactive parameter with validation
//! let wavelength = Observable::new(800.0)
//!     .with_name("wavelength")
//!     .with_units("nm")
//!     .with_range(700.0..=1000.0);
//!
//! // Subscribe to changes
//! let mut rx = wavelength.subscribe();
//! wavelength.set(850.0)?;
//! ```
//!
//! ## Feature Flags
//!
//! - `serial` - Enable serial port support for hardware drivers
//! - `storage_arrow` - Enable Arrow IPC format support
//!
//! [`Movable`]: capabilities::Movable
//! [`Readable`]: capabilities::Readable
//! [`FrameProducer`]: capabilities::FrameProducer
//! [`Triggerable`]: capabilities::Triggerable
//! [`Observable`]: observable::Observable
//! [`Parameter<T>`]: parameter::Parameter
//! [`DaqError`]: error::DaqError
//! [`DriverFactory`]: driver::DriverFactory
//! [`Frame`]: data::Frame
//! [`PixelBuffer`]: data::PixelBuffer

// TODO: Fix doc comment generic types (e.g., `Parameter<T>`) to use backticks
// and broken intra-doc links (e.g., `#[async_trait]`)
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub mod core;
pub mod validation;
// Data types (Frame, etc.)
pub mod data;
// Document model (Bluesky-style)
pub mod capabilities;
pub mod echelle;
pub mod echelle_calibration_pipeline;
pub mod echelle_optimal_extraction;
pub mod echelle_rectification;
pub mod echelle_scattered_light;
pub mod echelle_simulation;
pub mod echelle_trace_fitting;
pub mod echelle_wavelength_fitting;
pub mod error;
pub mod error_recovery;
pub mod experiment;
pub mod health;
pub mod limits;
pub mod log_scrubbing;
pub mod modules;
pub mod observable;
pub mod parameter;
pub mod pipeline;

// Driver factory and capability types for plugin architecture
pub mod driver;

// Well-known panel_kind string constants for explicit UI routing
pub mod panel_kind;

// Runtime feature flags loaded from config/feature_flags.toml
pub mod feature_flags;

// Serial port abstractions for driver crates (requires "serial" feature)
#[cfg(feature = "serial")]
pub mod serial;

// FITS file I/O for calibration frame import (requires "fits" feature + cfitsio C library)
#[cfg(feature = "fits")]
pub mod fits_io;

// Game loop state broadcasting (Phase 4)
pub mod state_cache;

// Timestamp utilities
pub mod time;

// Arrow extension metadata helpers for Python interop
pub mod arrow_metadata;

// Signal-processing utilities (radiance calibration, interpolation, etc.)
pub mod processing;
