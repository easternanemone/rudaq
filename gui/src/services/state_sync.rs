//! State synchronization service
//!
//! Handles real-time device state streaming from the daemon.
//! Extracts the streaming logic from connection handlers for better testability.

use crate::services::DaqClient;
use crate::state::SharedState;
use crate::ui::UiAdapter;
use tracing::error;

/// Start the device state subscription stream
///
/// Subscribes to device state updates from the daemon and routes updates
/// to the UI via the UiAdapter. Stores the task handle in state for cleanup.
pub async fn start_state_stream(state: SharedState, client: DaqClient, ui: UiAdapter) {
    match client
        .subscribe_device_state_with_reconnect(vec![], 5, true)
        .await
    {
        Ok(mut rx) => {
            let handle = tokio::spawn(async move {
                while let Some(update) = rx.recv().await {
                    let device_id = update.device_id.clone();

                    // Update position if available
                    if let Some(pos_str) = update.fields_json.get("position") {
                        if let Ok(pos) = pos_str.parse::<f64>() {
                            ui.update_device_position(device_id.clone(), pos as f32);
                        }
                    }

                    // Update reading if available
                    if let Some(read_str) = update.fields_json.get("reading") {
                        if let Ok(read) = read_str.parse::<f64>() {
                            ui.update_device_reading(device_id.clone(), read as f32);
                        }
                    }
                }
            });

            // Save handle to state for cleanup on disconnect
            let mut guard = state.lock().await;
            guard.state_stream_handle = Some(handle);
        }
        Err(e) => {
            error!("State stream failed to start: {}", e);
        }
    }
}
