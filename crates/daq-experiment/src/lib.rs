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
