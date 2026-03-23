//! Engine state and frame observation types.
//!
//! Contains the `EngineState` enum, its `Display` impl, and the
//! `ExperimentFrameObserver` used for secondary frame capture during runs.

use bytes::Bytes;
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

/// Frame capture data for experiment persistence (bd-nctn: uses Bytes instead of Vec<u8>).
pub(crate) struct FrameCapture {
    pub device_id: String,
    /// Pixel data as `Bytes` — avoids an intermediate `Vec<u8>` allocation
    /// when the data is later inserted into `collected_frames` (which stores `Bytes`).
    pub data: Bytes,
    pub width: u32,
    pub height: u32,
    pub frame_number: u64,
    /// Number of raw frames summed into this output frame (host-side summing, bd-oqo7.7).
    /// `None` or `Some(1)` means no summing. `Some(N)` means N frames were accumulated.
    pub summing_count: Option<u32>,
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
            data: Bytes::copy_from_slice(frame.pixels()),
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
