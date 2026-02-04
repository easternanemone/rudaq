// Allow Rust 1.93+ clippy lints that require significant refactoring
// These can be addressed incrementally in future PRs
#![allow(
    dead_code,                         // FrameCapture fields reserved for future use
    clippy::unnecessary_literal_bound, // lifetime-related str returns
    clippy::float_cmp,                 // f32/f64 comparisons (scan bounds)
    clippy::implicit_clone,            // .to_string() on String
    clippy::manual_let_else,           // if-let to let-else
    clippy::unchecked_time_subtraction, // Duration subtraction
    clippy::assigning_clones,          // clone vs clone_from
    clippy::unused_async,              // async fns without await
    clippy::bool_to_int_with_if        // if x { 1 } else { 0 }
)]

//! Experiment orchestration module (bd-73yh)
//!
//! This module provides the RunEngine for orchestrating long-running experiments
//! with pause/resume capabilities, structured data management, and declarative plans.
//!
//! # Architecture (Bluesky-inspired)
//!
//! - **Plans**: Declarative experiment definitions that yield commands
//! - **RunEngine**: State machine that executes plans and manages lifecycle
//! - **Documents**: Structured data streams (Start, Descriptor, Event, Stop)
//!
//! # Example
//!
//! ```rust,ignore
//! use daq_experiment::{RunEngine, plans::GridScan};
//!
//! let engine = RunEngine::new(device_registry);
//!
//! // Queue a plan
//! let plan = GridScan::new("stage_x", 0.0, 10.0, 11)
//!     .with_detector("power_meter")
//!     .build();
//!
//! let run_uid = engine.queue(plan).await?;
//! engine.start().await?;
//!
//! // Can pause/resume at any checkpoint
//! engine.pause().await?;
//! engine.resume().await?;
//! ```

pub mod plans;
pub mod plans_daq;
pub mod plans_imperative;
pub mod run_engine;

// Re-export document types from common
pub use common::experiment::document::{
    DataKey, DescriptorDoc, Document, EventDoc, ExperimentManifest, StartDoc, StopDoc,
};
pub use plans::{Plan, PlanCommand, PlanRegistry};
pub use plans_daq::{
    TimeSeries, TimeSeriesBuilder, TriggeredAcquisition, TriggeredAcquisitionBuilder, VoltageScan,
    VoltageScanBuilder,
};
pub use plans_imperative::ImperativePlan;
pub use run_engine::{EngineState, RunEngine, RunResult};
