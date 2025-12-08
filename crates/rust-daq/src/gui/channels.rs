//! Channel hub for UI <-> Backend communication.
//!
//! Architecture (from Codex validation):
//! - `watch` channels for latest-only state (device state, metrics)
//! - Bounded `mpsc` for event streams (plots, frames)
//! - Non-blocking `try_send` with drop-on-full for high-rate data

use tokio::sync::{mpsc, watch};
use tracing::warn;

use super::types::{
    BackendCommand, BackendEvent, BackendMetrics, DeviceStateSnapshot, PlotPoint,
};

/// Channel capacities (from Codex recommendations).
pub const STATE_CHANNEL_CAPACITY: usize = 256;
pub const PLOT_CHANNEL_CAPACITY: usize = 1024;
pub const FRAME_CHANNEL_CAPACITY: usize = 4;
pub const EVENT_CHANNEL_CAPACITY: usize = 64;
pub const COMMAND_CHANNEL_CAPACITY: usize = 32;

/// Channels held by the UI thread for receiving data from backend.
pub struct UiChannels {
    /// Latest device state snapshot (watch = always latest, no backlog)
    pub state_rx: watch::Receiver<DeviceStateSnapshot>,
    /// Plot data points (bounded mpsc)
    pub plot_rx: mpsc::Receiver<PlotPoint>,
    /// One-shot events from backend (device list, errors, etc.)
    pub event_rx: mpsc::Receiver<BackendEvent>,
    /// Backend performance metrics
    pub metrics_rx: watch::Receiver<BackendMetrics>,
    /// Send commands to backend
    pub cmd_tx: mpsc::Sender<BackendCommand>,
}

/// Handle held by the backend thread for sending data to UI.
pub struct BackendHandle {
    /// Send device state updates
    pub state_tx: watch::Sender<DeviceStateSnapshot>,
    /// Send plot data points
    pub plot_tx: mpsc::Sender<PlotPoint>,
    /// Send one-shot events
    pub event_tx: mpsc::Sender<BackendEvent>,
    /// Send metrics updates
    pub metrics_tx: watch::Sender<BackendMetrics>,
    /// Receive commands from UI
    pub cmd_rx: mpsc::Receiver<BackendCommand>,
}

/// Create a new pair of UI and Backend channel handles.
pub fn create_channels() -> (UiChannels, BackendHandle) {
    // watch channels for latest-only state
    let (state_tx, state_rx) = watch::channel(DeviceStateSnapshot::default());
    let (metrics_tx, metrics_rx) = watch::channel(BackendMetrics::default());

    // bounded mpsc for event streams
    let (plot_tx, plot_rx) = mpsc::channel(PLOT_CHANNEL_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);

    // command channel from UI to backend
    let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);

    let ui_channels = UiChannels {
        state_rx,
        plot_rx,
        event_rx,
        metrics_rx,
        cmd_tx,
    };

    let backend_handle = BackendHandle {
        state_tx,
        plot_tx,
        event_tx,
        metrics_tx,
        cmd_rx,
    };

    (ui_channels, backend_handle)
}

impl UiChannels {
    /// Get latest device state snapshot (non-blocking).
    pub fn get_state(&mut self) -> DeviceStateSnapshot {
        self.state_rx.borrow_and_update().clone()
    }

    /// Get latest metrics (non-blocking).
    pub fn get_metrics(&mut self) -> BackendMetrics {
        self.metrics_rx.borrow_and_update().clone()
    }

    /// Try to receive a plot point (non-blocking).
    pub fn try_recv_plot(&mut self) -> Option<PlotPoint> {
        self.plot_rx.try_recv().ok()
    }

    /// Drain all available plot points (non-blocking).
    pub fn drain_plots(&mut self) -> Vec<PlotPoint> {
        let mut points = Vec::new();
        while let Ok(point) = self.plot_rx.try_recv() {
            points.push(point);
        }
        points
    }

    /// Try to receive an event (non-blocking).
    pub fn try_recv_event(&mut self) -> Option<BackendEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Drain all available events (non-blocking).
    pub fn drain_events(&mut self) -> Vec<BackendEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Send a command to the backend (non-blocking, returns false if full).
    pub fn send_command(&self, cmd: BackendCommand) -> bool {
        self.cmd_tx.try_send(cmd).is_ok()
    }
}

impl BackendHandle {
    /// Update the device state snapshot.
    pub fn update_state(&self, state: DeviceStateSnapshot) {
        // watch::send never blocks, just replaces the value
        let _ = self.state_tx.send(state);
    }

    /// Update backend metrics.
    pub fn update_metrics(&self, metrics: BackendMetrics) {
        let _ = self.metrics_tx.send(metrics);
    }

    /// Try to send a plot point (non-blocking, drops if full).
    pub fn try_send_plot(&self, point: PlotPoint) -> bool {
        match self.plot_tx.try_send(point) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(p)) => {
                warn!(device_id = %p.device_id, "Plot channel full, dropping point");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("Plot channel closed");
                false
            }
        }
    }

    /// Send an event to the UI.
    pub fn send_event(&self, event: BackendEvent) -> bool {
        match self.event_tx.try_send(event) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(e)) => {
                warn!(event = ?e, "Event channel full, dropping event");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("Event channel closed");
                false
            }
        }
    }

    /// Check if there's a command waiting.
    pub fn has_command(&mut self) -> bool {
        // peek without consuming
        matches!(self.cmd_rx.try_recv(), Ok(_) | Err(mpsc::error::TryRecvError::Empty))
    }

    /// Receive the next command (async, use in tokio context).
    pub async fn recv_command(&mut self) -> Option<BackendCommand> {
        self.cmd_rx.recv().await
    }

    /// Try to receive a command (non-blocking).
    pub fn try_recv_command(&mut self) -> Option<BackendCommand> {
        self.cmd_rx.try_recv().ok()
    }
}
