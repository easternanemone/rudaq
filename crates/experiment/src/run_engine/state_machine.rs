//! Engine state and frame observation types.
//!
//! Contains the `EngineState` enum, its `Display` impl, and the
//! `ExperimentFrameObserver` used for secondary frame capture during runs.

use common::capabilities::FrameObserver;
use common::data::FrameView;
use tokio::sync::mpsc;

/// Engine state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    /// No plan running, ready to accept new plans
    Idle,
    /// Executing a plan
    Running,
    /// Paused at a checkpoint, can resume or abort
    Paused,
    /// Aborting current plan (will return to Idle)
    Aborting,
}

impl std::fmt::Display for EngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineState::Idle => write!(f, "idle"),
            EngineState::Running => write!(f, "running"),
            EngineState::Paused => write!(f, "paused"),
            EngineState::Aborting => write!(f, "aborting"),
        }
    }
}

/// Frame capture data for experiment persistence
pub(crate) struct FrameCapture {
    pub device_id: String,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub frame_number: u64,
}

/// Observer that captures frames for experiment persistence
pub(crate) struct ExperimentFrameObserver {
    pub tx: mpsc::Sender<FrameCapture>,
    pub device_id: String,
}

impl FrameObserver for ExperimentFrameObserver {
    fn on_frame(&self, frame: &FrameView<'_>) {
        let capture = FrameCapture {
            device_id: self.device_id.clone(),
            data: frame.pixels().to_vec(),
            width: frame.width,
            height: frame.height,
            frame_number: frame.frame_number,
        };
        // Non-blocking send - drop frames if channel is full
        let _ = self.tx.try_send(capture);
    }

    fn name(&self) -> &'static str {
        "experiment_capture"
    }
}
