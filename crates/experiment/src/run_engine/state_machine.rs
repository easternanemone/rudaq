//! Engine state and frame observation types.
//!
//! Contains the `EngineState` enum, its `Display` impl, and the
//! `ExperimentFrameObserver` used for secondary frame capture during runs.

use std::collections::HashMap;

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
    /// Frame metadata for EventDoc propagation (bd-p6r4).
    /// Contains hardware timestamps, SMART stream index, driver-specific fields.
    pub metadata: HashMap<String, String>,
}

/// Observer that captures frames for experiment persistence
pub(crate) struct ExperimentFrameObserver {
    pub tx: mpsc::Sender<FrameCapture>,
    pub device_id: String,
}

impl FrameObserver for ExperimentFrameObserver {
    fn on_frame(&self, frame: &FrameView<'_>) {
        // bd-p6r4: Collect driver-specific extra metadata plus core FrameView fields
        // that aren't captured as typed fields on FrameCapture.
        let metadata = {
            // Pre-allocate: extra entries + up to 5 core fields (timestamp, exposure, binning x/y, temp)
            let mut md = HashMap::with_capacity(frame.extra.len() + 5);
            md.extend(frame.extra.iter().map(|(k, v)| (k.clone(), v.clone())));
            md.insert("timestamp_ns".into(), frame.timestamp_ns.to_string());
            if let Some(exp) = frame.exposure_ms {
                md.insert("exposure_ms".into(), exp.to_string());
            }
            if let Some((bx, by)) = frame.binning {
                md.insert("binning_x".into(), bx.to_string());
                md.insert("binning_y".into(), by.to_string());
            }
            if let Some(temp) = frame.temperature_c {
                md.insert("temperature_c".into(), temp.to_string());
            }
            md
        };

        let capture = FrameCapture {
            device_id: self.device_id.clone(),
            data: Bytes::copy_from_slice(frame.pixels()),
            width: frame.width,
            height: frame.height,
            frame_number: frame.frame_number,
            summing_count: frame.summing_count,
            metadata,
        };
        // Non-blocking send - drop frames if channel is full
        let _ = self.tx.try_send(capture);
    }

    fn name(&self) -> &'static str {
        "experiment_capture"
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_state_display() {
        assert_eq!(EngineState::Idle.to_string(), "idle");
        assert_eq!(EngineState::Running.to_string(), "running");
        assert_eq!(EngineState::Paused.to_string(), "paused");
        assert_eq!(EngineState::Aborting.to_string(), "aborting");
    }

    #[test]
    fn test_engine_state_equality() {
        assert_eq!(EngineState::Idle, EngineState::Idle);
        assert_eq!(EngineState::Running, EngineState::Running);
        assert_ne!(EngineState::Idle, EngineState::Running);
        assert_ne!(EngineState::Paused, EngineState::Aborting);
    }

    #[test]
    fn test_engine_state_copy() {
        let state = EngineState::Running;
        let copy = state;
        assert_eq!(state, copy);
    }

    #[test]
    fn test_engine_state_debug() {
        let dbg = format!("{:?}", EngineState::Paused);
        assert!(dbg.contains("Paused"));
    }

    #[test]
    fn test_all_engine_state_variants_covered() {
        let variants = [
            EngineState::Idle,
            EngineState::Running,
            EngineState::Paused,
            EngineState::Aborting,
        ];
        for v in &variants {
            // Ensures Display doesn't panic on any variant.
            let _ = v.to_string();
        }
    }
}
