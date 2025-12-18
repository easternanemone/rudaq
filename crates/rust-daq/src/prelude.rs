//! Prelude module for convenient imports
//!
//! This module provides organized re-exports from the `rust-daq` ecosystem.
//! Import this to get access to common types and traits without dealing with
//! import ambiguity.
//!
//! # Usage
//!
//! ```rust,ignore
//! use rust_daq::prelude::*;
//! ```
//!
//! # Organization
//!
//! Re-exports are grouped by functional area:
//! - Core domain types and errors
//! - Reactive programming (Parameter, Observable)
//! - Hardware abstraction and drivers
//! - Data storage and processing
//! - Experiment orchestration
//! - Scripting integration

// =============================================================================
// Core Domain Types & Errors
// =============================================================================

/// Core domain types and utilities
pub use daq_core::core;

/// Error handling and DaqError type
pub use daq_core::error;

// =============================================================================
// Reactive Programming
// =============================================================================

/// Observable pattern for reactive state management
pub use daq_core::observable;

/// Reactive Parameter<T> system with async hardware callbacks
pub use daq_core::parameter;

// =============================================================================
// Hardware Abstraction Layer
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
/// Hardware drivers, capability traits, and device registry
///
/// Re-exported from `daq-hardware`. Includes:
/// - Capability traits: `Movable`, `Readable`, `FrameProducer`, etc.
/// - Hardware drivers: ELL14, ESP300, PVCAM, MaiTai, Newport 1830-C
/// - Hardware registry and resource pooling
pub use crate::hardware;

// =============================================================================
// Experiment Orchestration
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
/// Experiment orchestration (RunEngine and Plans)
///
/// Re-exported from `daq-experiment`.
pub use daq_experiment as experiment;

// =============================================================================
// Scripting Integration
// =============================================================================

#[cfg(all(not(target_arch = "wasm32"), feature = "scripting"))]
/// Rhai scripting engine integration
///
/// Re-exported from `daq-scripting`.
pub use daq_scripting as scripting;

// =============================================================================
// Module System
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
/// Module management for experiment-specific workflows
pub use crate::modules;
