//! Plan trait and message types for experiment sequencing.
//!
//! Plans define experiment workflows as async streams of messages. Messages
//! describe actions for the RunEngine to perform (read data, set parameters,
//! checkpoint state, etc.).

use anyhow::Result;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Type alias for plan message streams.
///
/// Plans yield messages asynchronously via this stream type. The RunEngine
/// consumes the stream and translates messages into module/instrument commands.
pub type PlanStream<'a> = BoxStream<'a, Result<Message>>;

/// Messages emitted by Plans to control experiment execution.
///
/// This enum defines the protocol between Plans and the RunEngine. Each variant
/// represents an action the engine should perform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Message {
    /// Begin an experiment run with metadata.
    ///
    /// Metadata might include experiment name, operator, timestamp, parameters, etc.
    /// The RunEngine uses this to initialize logging and state tracking.
    BeginRun {
        /// Run metadata (experiment name, parameters, etc.)
        metadata: HashMap<String, String>,
    },

    /// End the current experiment run.
    ///
    /// Finalizes data collection, saves checkpoints, and releases resources.
    EndRun,

    /// Set a module or instrument parameter.
    ///
    /// Examples:
    /// - Set laser power: `target="laser", param="power", value="50.0"`
    /// - Set stage position: `target="stage_x", param="position", value="10.5"`
    Set {
        /// Target module or instrument ID
        target: String,
        /// Parameter name
        param: String,
        /// Parameter value (serialized as string)
        value: String,
    },

    /// Trigger data acquisition from a module.
    ///
    /// Tells the module to start its acquisition cycle. Does not wait for data.
    Trigger {
        /// Module ID to trigger
        module_id: String,
    },

    /// Read data from a module or instrument.
    ///
    /// Waits for the module to produce data and returns it to the plan.
    /// Used for adaptive experiments that need feedback.
    Read {
        /// Module ID to read from
        module_id: String,
    },

    /// Wait for a specified duration.
    ///
    /// Useful for equilibration time, cooling periods, etc.
    Sleep {
        /// Duration to sleep in seconds
        duration_secs: f64,
    },

    /// Create a checkpoint for pause/resume.
    ///
    /// Saves the current experiment state so execution can be resumed later.
    /// Plans should emit checkpoints at safe resumption points.
    Checkpoint {
        /// Optional checkpoint name/label
        label: Option<String>,
    },

    /// Pause experiment execution.
    ///
    /// The RunEngine will stop processing messages after the current one completes.
    /// Execution can be resumed via `Resume` message or RunEngine API.
    Pause,

    /// Resume experiment execution from a paused state.
    ///
    /// Only valid when RunEngine is in Paused state.
    Resume,

    /// Emit a log message during execution.
    ///
    /// Used for progress reporting and debugging.
    Log {
        /// Log level (info, warn, error)
        level: LogLevel,
        /// Log message
        message: String,
    },
}

/// Log levels for plan messages.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

/// Trait for experiment plans.
///
/// Plans define experiment workflows as async streams of [`Message`] items.
/// The RunEngine consumes these messages and executes the corresponding actions.
///
/// # Lifecycle
///
/// 1. Plan created with parameters
/// 2. `execute()` called to start message stream
/// 3. RunEngine consumes stream and performs actions
/// 4. Plan completes when stream ends
///
/// # Checkpointing
///
/// Plans should implement `Serialize` + `Deserialize` to enable checkpointing.
/// When a `Checkpoint` message is emitted, the RunEngine serializes the plan state
/// so execution can be resumed after pause or error.
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::experiment::{Plan, PlanStream, Message};
/// use futures::stream::{self, StreamExt};
/// use anyhow::Result;
///
/// struct SimplePlan {
///     count: usize,
/// }
///
/// impl Plan for SimplePlan {
///     fn execute(&mut self) -> PlanStream {
///         let count = self.count;
///         Box::pin(stream::iter((0..count).map(|i| {
///             Ok(Message::Log {
///                 level: LogLevel::Info,
///                 message: format!("Step {}/{}", i + 1, count),
///             })
///         })))
///     }
/// }
/// ```
pub trait Plan: Send {
    /// Execute the plan, yielding a stream of messages.
    ///
    /// The RunEngine will consume this stream and perform the corresponding actions.
    /// Plans can be stateful - they can maintain internal state across `execute()` calls
    /// for resumption after pause/checkpoint.
    fn execute(&mut self) -> PlanStream<'_>;

    /// Optional: Validate plan before execution.
    ///
    /// Return `Err` if the plan configuration is invalid or required resources
    /// are unavailable. Called by RunEngine before starting execution.
    fn validate(&self) -> Result<()> {
        Ok(())
    }

    /// Optional: Get plan metadata for logging.
    ///
    /// Returns a human-readable name and description for this plan.
    fn metadata(&self) -> (String, String) {
        (
            "Unnamed Plan".to_string(),
            "No description".to_string(),
        )
    }
}
