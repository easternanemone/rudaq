//! # daq-core
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
//! use daq_core::observable::Observable;
//! use daq_core::capabilities::Movable;
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
// Data types (Frame, etc.)
pub mod data;
// Document model (Bluesky-style)
pub mod capabilities;
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

// Serial port abstractions for driver crates (requires "serial" feature)
#[cfg(feature = "serial")]
pub mod serial;
