//! Shared state cache for the game loop broadcast system.
//!
//! Drivers write [`NodeStateUpdate`] values into an `mpsc` channel.
//! The game loop drains updates into a local snapshot map and broadcasts
//! a [`SystemStateSnapshot`] at a fixed tick rate via a `broadcast` channel.
//!
//! ```text
//! Driver A ─┐                     ┌─ gRPC subscriber 1
//! Driver B ─┤ mpsc → Game Loop → broadcast ─┤─ gRPC subscriber 2
//! Driver C ─┘   (accumulate)    (snapshot)  └─ gRPC subscriber 3
//! ```

use std::collections::HashMap;

/// A state update from a hardware driver.
#[derive(Debug, Clone)]
pub struct NodeStateUpdate {
    /// Device that produced this update.
    pub device_id: String,
    /// Acquisition timestamp in nanoseconds since Unix epoch.
    /// **Must** be the time the driver actually read the hardware.
    pub timestamp_ns: i64,
    /// The measured/reported value.
    pub value: NodeValue,
    /// Optional key-value metadata (units, channel, etc.).
    pub metadata: HashMap<String, String>,
}

/// Value types a node can report.
#[derive(Debug, Clone)]
pub enum NodeValue {
    /// Scalar analog reading (power, temperature, position, etc.).
    Analog(f64),
    /// Boolean state (shutter open/closed, laser emission on/off, etc.).
    Digital(bool),
    /// Free-form text (status messages, error strings, etc.).
    Text(String),
    /// Vector of values (spectrum, multi-channel DAQ, etc.).
    Vector(Vec<f64>),
}

/// A snapshot of all device states, assembled by the game loop.
#[derive(Debug, Clone)]
pub struct SystemStateSnapshot {
    /// Latest state for each device.
    pub nodes: Vec<NodeStateUpdate>,
    /// Timestamp when the game loop assembled this snapshot (broadcast time).
    pub broadcast_timestamp_ns: i64,
    /// Current tick rate.
    pub tick_rate_hz: u32,
}

/// Returns the current time as nanoseconds since Unix epoch.
pub fn now_ns() -> i64 {
    crate::time::now_ns_i64()
}

/// Configuration for the game loop.
#[derive(Debug, Clone)]
pub struct GameLoopConfig {
    /// Tick rate in Hz (default: 30, range: 1-100).
    pub tick_rate_hz: u32,
    /// Broadcast channel capacity (slow subscribers drop old messages).
    pub broadcast_capacity: usize,
}

impl Default for GameLoopConfig {
    fn default() -> Self {
        Self {
            tick_rate_hz: 30,
            broadcast_capacity: 16,
        }
    }
}

/// Run the game loop: drain driver updates, build snapshots, broadcast.
///
/// # Arguments
/// - `update_rx` — receives `NodeStateUpdate` from drivers
/// - `broadcast_tx` — sends `SystemStateSnapshot` to gRPC subscribers
/// - `config` — tick rate and capacity settings
/// - `shutdown` — cancellation token for graceful shutdown
///
/// The loop runs at `config.tick_rate_hz` Hz. On each tick it:
/// 1. Drains all pending updates from `update_rx` into a local map
/// 2. Assembles a snapshot from the latest value per device
/// 3. Broadcasts the snapshot (slow subscribers drop old snapshots)
pub async fn run_game_loop(
    mut update_rx: tokio::sync::mpsc::Receiver<NodeStateUpdate>,
    broadcast_tx: tokio::sync::broadcast::Sender<SystemStateSnapshot>,
    config: GameLoopConfig,
    shutdown: tokio_util::sync::CancellationToken,
) {
    use tracing::info;

    let tick_rate = config.tick_rate_hz.clamp(1, 100);
    let interval = std::time::Duration::from_secs_f64(1.0 / tick_rate as f64);
    let mut tick = tokio::time::interval(interval);
    let mut latest: HashMap<String, NodeStateUpdate> = HashMap::new();

    info!(tick_rate_hz = tick_rate, "game loop started");

    loop {
        tokio::select! {
            () = shutdown.cancelled() => {
                info!("game loop shutting down");
                break;
            }
            _ = tick.tick() => {
                // Drain all pending updates.
                while let Ok(update) = update_rx.try_recv() {
                    latest.insert(update.device_id.clone(), update);
                }

                if latest.is_empty() {
                    continue;
                }

                let snapshot = SystemStateSnapshot {
                    nodes: latest.values().cloned().collect(),
                    broadcast_timestamp_ns: now_ns(),
                    tick_rate_hz: tick_rate,
                };

                // Broadcast — ignores error if no subscribers.
                let _ = broadcast_tx.send(snapshot);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{broadcast, mpsc};
    use tokio_util::sync::CancellationToken;

    fn analog_update(device_id: &str, value: f64) -> NodeStateUpdate {
        NodeStateUpdate {
            device_id: device_id.into(),
            timestamp_ns: now_ns(),
            value: NodeValue::Analog(value),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_game_loop_broadcasts_snapshot() {
        let (update_tx, update_rx) = mpsc::channel(64);
        let (broadcast_tx, mut broadcast_rx) = broadcast::channel(16);
        let shutdown = CancellationToken::new();

        // High tick rate for test speed.
        let config = GameLoopConfig {
            tick_rate_hz: 100,
            broadcast_capacity: 16,
        };

        let shutdown_clone = shutdown.clone();
        let handle = tokio::spawn(run_game_loop(
            update_rx,
            broadcast_tx,
            config,
            shutdown_clone,
        ));

        // Send an update.
        update_tx.send(analog_update("pm_1", 42.0)).await.unwrap();

        // Wait for at least one broadcast.
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(1), broadcast_rx.recv())
            .await
            .expect("timeout")
            .expect("recv");

        assert_eq!(snapshot.nodes.len(), 1);
        assert_eq!(snapshot.nodes[0].device_id, "pm_1");
        assert!(
            matches!(snapshot.nodes[0].value, NodeValue::Analog(v) if (v - 42.0).abs() < f64::EPSILON)
        );
        assert!(snapshot.broadcast_timestamp_ns > 0);
        assert_eq!(snapshot.tick_rate_hz, 100);

        shutdown.cancel();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_game_loop_accumulates_latest() {
        let (update_tx, update_rx) = mpsc::channel(64);
        let (broadcast_tx, mut broadcast_rx) = broadcast::channel(16);
        let shutdown = CancellationToken::new();

        let config = GameLoopConfig {
            tick_rate_hz: 100,
            broadcast_capacity: 16,
        };

        let shutdown_clone = shutdown.clone();
        let handle = tokio::spawn(run_game_loop(
            update_rx,
            broadcast_tx,
            config,
            shutdown_clone,
        ));

        // Send multiple updates for the same device — latest wins.
        update_tx.send(analog_update("pm_1", 1.0)).await.unwrap();
        update_tx.send(analog_update("pm_1", 2.0)).await.unwrap();
        update_tx.send(analog_update("pm_1", 3.0)).await.unwrap();
        update_tx.send(analog_update("pm_2", 99.0)).await.unwrap();

        // Wait for broadcast.
        let snapshot = tokio::time::timeout(std::time::Duration::from_secs(1), broadcast_rx.recv())
            .await
            .expect("timeout")
            .expect("recv");

        // Should have 2 devices (pm_1 and pm_2).
        assert_eq!(snapshot.nodes.len(), 2);

        let pm1 = snapshot
            .nodes
            .iter()
            .find(|n| n.device_id == "pm_1")
            .unwrap();
        assert!(matches!(pm1.value, NodeValue::Analog(v) if (v - 3.0).abs() < f64::EPSILON));

        let pm2 = snapshot
            .nodes
            .iter()
            .find(|n| n.device_id == "pm_2")
            .unwrap();
        assert!(matches!(pm2.value, NodeValue::Analog(v) if (v - 99.0).abs() < f64::EPSILON));

        shutdown.cancel();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_game_loop_skips_empty_ticks() {
        let (_update_tx, update_rx) = mpsc::channel::<NodeStateUpdate>(64);
        let (broadcast_tx, mut broadcast_rx) = broadcast::channel(16);
        let shutdown = CancellationToken::new();

        let config = GameLoopConfig {
            tick_rate_hz: 100,
            broadcast_capacity: 16,
        };

        let shutdown_clone = shutdown.clone();
        let handle = tokio::spawn(run_game_loop(
            update_rx,
            broadcast_tx,
            config,
            shutdown_clone,
        ));

        // No updates sent — should not receive any broadcasts.
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), broadcast_rx.recv()).await;
        assert!(result.is_err(), "should timeout — no data to broadcast");

        shutdown.cancel();
        handle.await.unwrap();
    }
}
