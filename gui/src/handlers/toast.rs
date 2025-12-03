//! Toast notification handlers
//!
//! Handles toast display and dismissal callbacks.

use crate::state::TOAST_ID_COUNTER;
use crate::ui::{ComponentHandle, MainWindow, ToastMessage, UiAdapter, VecModel, Weak};
use slint::Model;
use std::sync::atomic::Ordering;

/// Register toast-related callbacks
pub fn register(ui: &MainWindow, adapter: UiAdapter) {
    let ui_weak = adapter.weak();
    register_dismiss_toast(ui, ui_weak.clone());
    register_show_toast(ui, ui_weak);
}

fn register_dismiss_toast(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_dismiss_toast(move |toast_id| {
        let ui_weak = ui_weak.clone();

        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
            let toasts_model = ui.get_toasts();
            if let Some(vec_model) = toasts_model
                .as_any()
                .downcast_ref::<VecModel<ToastMessage>>()
            {
                for i in 0..vec_model.row_count() {
                    if let Some(toast) = vec_model.row_data(i) {
                        if toast.id == toast_id {
                            vec_model.remove(i);
                            break;
                        }
                    }
                }
            }
        });
    });
}

fn register_show_toast(ui: &MainWindow, ui_weak: Weak<MainWindow>) {
    ui.on_show_toast(move |severity, title, message| {
        let ui_weak = ui_weak.clone();
        let toast_id = TOAST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        let auto_dismiss = severity.as_str() != "error";

        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
            let toasts_model = ui.get_toasts();
            if let Some(vec_model) = toasts_model
                .as_any()
                .downcast_ref::<VecModel<ToastMessage>>()
            {
                vec_model.push(ToastMessage {
                    id: toast_id,
                    severity: severity.clone(),
                    title: title.clone(),
                    message: message.clone(),
                    auto_dismiss,
                    timestamp_ms: 0,
                });

                // Auto-dismiss non-error toasts after 5 seconds
                if auto_dismiss {
                    let ui_weak_dismiss = ui.as_weak();
                    let dismiss_id = toast_id;
                    slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
                        let _ = ui_weak_dismiss.upgrade_in_event_loop(move |ui| {
                            let toasts = ui.get_toasts();
                            if let Some(vm) =
                                toasts.as_any().downcast_ref::<VecModel<ToastMessage>>()
                            {
                                for i in 0..vm.row_count() {
                                    if let Some(t) = vm.row_data(i) {
                                        if t.id == dismiss_id {
                                            vm.remove(i);
                                            break;
                                        }
                                    }
                                }
                            }
                        });
                    });
                }
            }
        });
    });
}
