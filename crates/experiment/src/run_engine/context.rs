//! Run context for the currently executing plan.
//!
//! `RunContext` holds all per-run mutable state: collected data, frame
//! channels, position tracking, and timing. It is created when a plan
//! starts and dropped when it completes or aborts.

use std::collections::HashMap;

use bytes::Bytes;
use tokio::sync::mpsc;

use common::capabilities::ObserverHandle;

use super::state_machine::FrameCapture;

/// Run context for the currently executing plan.
///
/// Created by `execute_plan` and stored in `RunEngine::run_context`.
/// Cleared when the plan completes, aborts, or fails.
pub(crate) struct RunContext {
    /// Unique identifier for this run.
    pub(crate) run_uid: String,
    /// UID of the primary descriptor document.
    pub(crate) descriptor_uid: String,
    /// Monotonically increasing event sequence number.
    pub(crate) seq_num: u32,

    // --- Collected data (domain) ---
    /// Scalar readings collected between EmitEvent commands.
    pub(crate) collected_data: HashMap<String, f64>,
    /// Frame data collected between EmitEvent commands.
    pub(crate) collected_frames: HashMap<String, Bytes>,
    /// Last-known position for each mover device.
    pub(crate) current_positions: HashMap<String, f64>,

    // --- Frame I/O ---
    /// Active frame observer handles (unregistered on run completion).
    pub(crate) frame_observers: HashMap<String, ObserverHandle>,
    /// Frame capture channels from registered observers.
    pub(crate) frame_channels: HashMap<String, mpsc::Receiver<FrameCapture>>,

    // --- Timing ---
    /// Unix timestamp in nanoseconds when the run started.
    pub(crate) run_start_ns: u64,
}
