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
//! use rust_daq::experiment::{RunEngine, plans::GridScan};
//!
//! let engine = RunEngine::new(device_registry);
//!
//! // Queue a plan
//! let plan = GridScan::new()
//!     .axis("stage_x", 0.0, 10.0, 11)
//!     .axis("stage_y", 0.0, 5.0, 6)
//!     .detector("power_meter")
//!     .build();
//!
//! let run_uid = engine.queue(plan).await?;
//! engine.start().await?;
//!
//! // Can pause/resume at any checkpoint
//! engine.pause().await?;
//! engine.resume().await?;
//! ```

pub mod document;
pub mod plans;
pub mod run_engine;

pub use document::{DataKey, DescriptorDoc, Document, EventDoc, StartDoc, StopDoc};
pub use plans::{Plan, PlanCommand, PlanRegistry};
pub use run_engine::{EngineState, RunEngine};
