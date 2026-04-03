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

// Suppressed: generic types in doc comments (e.g., `Parameter<T>`) parse as HTML tags
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub mod core;
pub mod validation;
// Data types (Frame, etc.)
pub mod data;
// Document model (Bluesky-style)
pub mod error;
pub mod error_recovery;
pub mod experiment;
pub mod health;
pub mod limits;
pub mod log_scrubbing;

// Trait and reactive parameter system — canonical source is common-traits crate.
// Re-exported here for backward compatibility so all consumers continue to use
// `common::capabilities`, `common::driver`, etc. without import changes.
pub use common_traits::capabilities;
pub use common_traits::driver;
pub use common_traits::modules;
pub use common_traits::observable;
pub use common_traits::parameter;
pub use common_traits::pipeline;

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
