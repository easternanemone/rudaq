//! Stream integration for SignalPlotterPanel
//!
//! This module provides observable streaming subscription capabilities for the
//! signal plotter using a message-passing pattern that is safe for egui's
//! immediate mode architecture.
//!
//! ## Pattern
//!
//! 1. Get `ObservableUpdateSender` from `SignalPlotterPanel::get_sender()`
//! 2. Clone sender and pass to background Tokio task
//! 3. Background task sends `ObservableUpdate` messages via the sender
//! 4. Panel's `ui()` method calls `drain_updates()` each frame
//!
//! This avoids capturing `&mut self` across async boundaries.

// TODO(bd-yu38): Streaming integration not yet wired up to main UI
#![allow(dead_code)]

use crate::panels::signal_plotter::{ObservableUpdate, SignalPlotterPanel};
use daq_client::DaqClient;
use tokio::sync::mpsc as tokio_mpsc;

/// Stream subscription handle
///
/// Holds the cancellation signal for an active stream subscription.
/// Drop this to cancel the subscription.
pub struct StreamSubscription {
    cancel_tx: tokio_mpsc::Sender<()>,
    device_id: String,
    observable_name: String,
}

impl StreamSubscription {
    /// Cancel this subscription
    pub async fn cancel(self) {
        let _ = self.cancel_tx.send(()).await;
    }

    /// Get the device ID this subscription is for
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Get the observable name this subscription is for
    pub fn observable_name(&self) -> &str {
        &self.observable_name
    }
}

impl SignalPlotterPanel {
    /// Subscribe to observable stream with proper async integration
    ///
    /// This method:
    /// 1. Adds a trace to the plotter
    /// 2. Gets a clone of the update sender
    /// 3. Spawns a background task that streams updates via gRPC
    /// 4. Returns a handle to cancel the subscription
    ///
    /// # Arguments
    ///
    /// * `client` - DaqClient for gRPC communication
    /// * `runtime` - Tokio runtime handle for spawning async tasks
    /// * `device_id` - Device ID to monitor
    /// * `observable_name` - Observable name (e.g., "power_mw", "wavelength_nm")
    /// * `color` - Plot line color
    /// * `sample_rate_hz` - Desired sample rate (server may downsample)
    ///
    /// # Returns
    ///
    /// Returns `Some(StreamSubscription)` on success, `None` if sender unavailable.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let subscription = plotter.subscribe_observable(
    ///     &mut client,
    ///     &runtime,
    ///     "power_meter",
    ///     "power_mw",
    ///     egui::Color32::GREEN,
    ///     10, // 10 Hz
    /// );
    ///
    /// // Later, to cancel:
    /// if let Some(sub) = subscription {
    ///     runtime.spawn(sub.cancel());
    /// }
    /// ```
    pub fn subscribe_observable(
        &mut self,
        client: &DaqClient,
        runtime: &tokio::runtime::Runtime,
        device_id: &str,
        observable_name: &str,
        color: egui::Color32,
        sample_rate_hz: u32,
    ) -> Option<StreamSubscription> {
        // Add trace to plotter
        let label = format!("{}:{}", device_id, observable_name);
        self.add_trace(&label, device_id, observable_name, color);

        // Get sender for async updates
        let update_tx = self.get_sender()?;

        // Create cancellation channel
        let (cancel_tx, mut cancel_rx) = tokio_mpsc::channel::<()>(1);

        // Clone values for the async task
        let device_id_owned = device_id.to_string();
        let observable_name_owned = observable_name.to_string();
        let mut client = client.clone();

        // Spawn background task to stream updates
        runtime.spawn(async move {
            // Start the gRPC stream
            let stream_result = client
                .stream_observables(
                    vec![device_id_owned.clone()],
                    vec![observable_name_owned.clone()],
                    sample_rate_hz,
                )
                .await;

            let mut stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        "Failed to start observable stream for {}:{}: {}",
                        device_id_owned,
                        observable_name_owned,
                        e
                    );
                    return;
                }
            };

            // Process stream until cancelled or stream ends
            loop {
                tokio::select! {
                    // Check for cancellation
                    _ = cancel_rx.recv() => {
                        tracing::debug!(
                            "Observable stream cancelled for {}:{}",
                            device_id_owned, observable_name_owned
                        );
                        break;
                    }
                    // Process next stream item
                    item = stream.next() => {
                        match item {
                            Some(Ok(value)) => {
                                let update = ObservableUpdate::new(
                                    &value.device_id,
                                    &value.observable_name,
                                    value.value,
                                );
                                // Send to UI thread (non-blocking, may drop if full)
                                if update_tx.send(update).is_err() {
                                    // Receiver dropped, panel closed
                                    tracing::debug!(
                                        "Observable stream receiver closed for {}:{}",
                                        device_id_owned, observable_name_owned
                                    );
                                    break;
                                }
                            }
                            Some(Err(e)) => {
                                tracing::warn!(
                                    "Observable stream error for {}:{}: {}",
                                    device_id_owned, observable_name_owned, e
                                );
                                // Continue on transient errors
                            }
                            None => {
                                // Stream ended
                                tracing::debug!(
                                    "Observable stream ended for {}:{}",
                                    device_id_owned, observable_name_owned
                                );
                                break;
                            }
                        }
                    }
                }
            }
        });

        Some(StreamSubscription {
            cancel_tx,
            device_id: device_id.to_string(),
            observable_name: observable_name.to_string(),
        })
    }

    /// Subscribe to observable with polling fallback
    ///
    /// Use this when streaming is not available. Polls the device
    /// at the specified interval.
    ///
    /// # Arguments
    ///
    /// * `client` - DaqClient for gRPC communication
    /// * `runtime` - Tokio runtime handle
    /// * `device_id` - Device ID to monitor
    /// * `observable_name` - Observable name
    /// * `color` - Plot line color
    /// * `poll_interval_ms` - Polling interval in milliseconds
    pub fn subscribe_observable_polling(
        &mut self,
        client: &DaqClient,
        runtime: &tokio::runtime::Runtime,
        device_id: &str,
        observable_name: &str,
        color: egui::Color32,
        poll_interval_ms: u64,
    ) -> Option<StreamSubscription> {
        // Add trace to plotter
        let label = format!("{}:{}", device_id, observable_name);
        self.add_trace(&label, device_id, observable_name, color);

        // Get sender for async updates
        let update_tx = self.get_sender()?;

        // Create cancellation channel
        let (cancel_tx, mut cancel_rx) = tokio_mpsc::channel::<()>(1);

        // Clone values for the async task
        let device_id_owned = device_id.to_string();
        let observable_name_owned = observable_name.to_string();
        let mut client = client.clone();
        let interval = std::time::Duration::from_millis(poll_interval_ms);

        // Spawn polling task
        runtime.spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                tokio::select! {
                    _ = cancel_rx.recv() => {
                        tracing::debug!(
                            "Polling cancelled for {}:{}",
                            device_id_owned, observable_name_owned
                        );
                        break;
                    }
                    _ = ticker.tick() => {
                        // Poll the device
                        match client.read_value(&device_id_owned).await {
                            Ok(response) if response.success => {
                                let update = ObservableUpdate::new(
                                    &device_id_owned,
                                    &observable_name_owned,
                                    response.value,
                                );
                                if update_tx.send(update).is_err() {
                                    break; // Receiver closed
                                }
                            }
                            Ok(response) => {
                                tracing::trace!(
                                    "Poll failed for {}:{}: {}",
                                    device_id_owned, observable_name_owned, response.error_message
                                );
                            }
                            Err(e) => {
                                tracing::trace!(
                                    "Poll error for {}:{}: {}",
                                    device_id_owned, observable_name_owned, e
                                );
                            }
                        }
                    }
                }
            }
        });

        Some(StreamSubscription {
            cancel_tx,
            device_id: device_id.to_string(),
            observable_name: observable_name.to_string(),
        })
    }
}

// Import for stream operations
use futures::StreamExt;
