//! Prelude module for convenient imports
//!
//! This module provides **organized re-exports** from the `rust-daq` ecosystem, created during
//! the bd-232k refactoring to eliminate import ambiguity and clarify module ownership.
//!
//! # Usage
//!
//! ```rust,ignore
//! use rust_daq::prelude::*;
//! ```
//!
//! # Organization
//!
//! Re-exports are grouped by functional area for clarity:
//!
//! - **Core domain types and errors** (`core`, `error`)
//! - **Reactive programming** (`parameter`, `observable`)
//! - **Hardware abstraction and drivers** (`hardware`)
//! - **Experiment orchestration** (`experiment`)
//! - **Scripting integration** (`scripting` - requires `scripting` feature)
//! - **Module system** (`modules`)
//!
//! # Design Rationale (bd-232k)
//!
//! Before bd-232k, `rust_daq` re-exported types at the crate root (e.g., `rust_daq::core`,
//! `rust_daq::error`), creating ambiguity about whether code lived in `rust-daq` or a
//! dependency crate. The prelude pattern makes it explicit:
//!
//! - `rust_daq::prelude::core` → clearly from `daq-core` crate
//! - `rust_daq::prelude::hardware` → clearly from `daq-hardware` crate
//! - `rust_daq::prelude::scripting` → clearly from `daq-scripting` crate (optional)
//!
//! Root re-exports are deprecated and will be removed in 0.6.0.

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
