#![cfg(feature = "gui_egui")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
#![cfg(not(target_arch = "wasm32"))]
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use rust_daq::gui::{app::DaqGuiApp, create_channels};

#[test]
fn test_gui_visual_elements() {
    let (channels, _backend_handle) = create_channels();
    let mut app = DaqGuiApp::new_with_channels(channels);

    let mut harness = Harness::new(move |ctx| {
        app.ui(ctx);
    });

    // Run a few frames to let the UI settle
    harness.step();

    // Verify the main window title/heading
    harness.get_by_label("rust-daq Control Panel");

    // Verify connection status text
    harness.get_by_label("Not connected. Enter daemon address and click Connect.");

    // Verify buttons exist
    harness.get_by_label("Connect");

    // Verify input field exists (by its value or label)
    harness.get_by_label("Daemon:");
}
