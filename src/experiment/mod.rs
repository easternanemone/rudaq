//! Experiment sequencing and automation.
//!
//! This module provides a declarative system for defining and executing complex
//! multi-step experiments, inspired by BlueSky's RunEngine and Plan architecture.
//!
//! # Architecture
//!
//! The experiment system consists of three main components:
//!
//! - **Plan**: Async trait defining experiment sequences as streams of messages
//! - **RunEngine**: Executor that processes plan messages and controls modules/instruments
//! - **Checkpoint**: State management for pause/resume and error recovery
//!
//! # Comparison to BlueSky
//!
//! | BlueSky (Python)        | rust-daq (Rust)          |
//! |------------------------|--------------------------|
//! | Generator functions     | Async stream trait       |
//! | Yield messages         | Stream items             |
//! | RunEngine              | RunEngine                |
//! | Checkpoints            | Serde-based snapshots    |
//! | Open/Close run         | Begin/End messages       |
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_daq::experiment::{Plan, RunEngine, Message};
//! use futures::stream;
//!
//! // Define a simple time series plan
//! struct TimeSeriesPlan {
//!     duration: Duration,
//!     interval: Duration,
//! }
//!
//! impl Plan for TimeSeriesPlan {
//!     fn execute(&mut self) -> PlanStream {
//!         // Implementation yields Begin, Read, Checkpoint, End messages
//!         todo!()
//!     }
//! }
//!
//! // Execute the plan
//! let mut engine = RunEngine::new(actor_tx);
//! let plan = TimeSeriesPlan { duration: 60s, interval: 1s };
//! engine.run(plan).await?;
//! ```
//!
//! # Integration with Modules
//!
//! Plans interact with the module system via DaqCommand messages. The RunEngine
//! translates plan messages into module commands:
//!
//! ```text
//! Plan → Message Stream → RunEngine → DaqCommand → Module/Instrument
//! ```

pub mod plan;
pub mod primitives;
pub mod run_engine;
pub mod state;

pub use plan::{LogLevel, Message, Plan, PlanStream};
pub use primitives::{GridScanPlan, ScanPlan, TimeSeriesPlan};
pub use run_engine::{RunEngine, RunEngineStatus};
pub use state::{Checkpoint, ExperimentState};
