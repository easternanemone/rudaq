//! Common handler utilities
//!
//! Provides helper functions to reduce boilerplate in UI callback handlers.

use crate::services::DaqClient;
use crate::state::SharedState;
use crate::ui::{MainWindow, SharedString, Weak};
use std::future::Future;

/// Spawn an async RPC action with proper state and UI handling
///
/// This helper eliminates the common boilerplate pattern of:
/// 1. Cloning state and ui_weak
/// 2. Spawning a tokio task
/// 3. Locking state to get client
/// 4. Calling the async action
///
/// # Example
/// ```ignore
/// spawn_rpc(ui_weak, state, |client, ui_weak| async move {
///     match client.list_devices().await {
///         Ok(devices) => { /* update UI */ }
///         Err(e) => { /* show error */ }
///     }
/// });
/// ```
pub fn spawn_rpc<F, Fut>(ui_weak: Weak<MainWindow>, state: SharedState, action: F)
where
    F: FnOnce(DaqClient, Weak<MainWindow>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        let client = {
            let guard = state.lock().await;
            guard.get_client()
        };

        if let Some(client) = client {
            action(client, ui_weak).await;
        }
    });
}

/// Spawn an async action that needs mutable state access
///
/// Use this when the action needs to modify AppState (e.g., storing a scan ID).
pub fn spawn_with_state<F, Fut>(ui_weak: Weak<MainWindow>, state: SharedState, action: F)
where
    F: FnOnce(SharedState, Weak<MainWindow>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        action(state, ui_weak).await;
    });
}

/// Show a toast notification via the UI
///
/// Helper to reduce boilerplate when showing toasts from async handlers.
pub fn show_toast(ui_weak: &Weak<MainWindow>, severity: &str, title: &str, message: &str) {
    let severity = SharedString::from(severity);
    let title = SharedString::from(title);
    let message = SharedString::from(message);

    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        ui.invoke_show_toast(severity, title, message);
    });
}

/// Show an error toast
pub fn show_error(ui_weak: &Weak<MainWindow>, title: &str, error: &impl std::fmt::Display) {
    show_toast(ui_weak, "error", title, &error.to_string());
}

/// Show a success toast
pub fn show_success(ui_weak: &Weak<MainWindow>, title: &str, message: &str) {
    show_toast(ui_weak, "success", title, message);
}

/// Show a warning toast
pub fn show_warning(ui_weak: &Weak<MainWindow>, title: &str, message: &str) {
    show_toast(ui_weak, "warning", title, message);
}

/// Show an info toast
pub fn show_info(ui_weak: &Weak<MainWindow>, title: &str, message: &str) {
    show_toast(ui_weak, "info", title, message);
}
