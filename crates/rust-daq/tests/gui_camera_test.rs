#![cfg(feature = "gui_egui")]
#![allow(
    clippy::unwrap_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    missing_docs
)]
#![cfg(not(target_arch = "wasm32"))]

use rust_daq::gui::{app::DaqGuiApp, create_channels, BackendEvent};

#[test]
fn test_camera_stream_logic_u16() {
    let (channels, mut backend_handle) = create_channels();
    let mut app = DaqGuiApp::new_with_channels(channels);
    app.auto_scale = false; // Disable auto-scale for raw MSB test

    // Simulate 16-bit image (2x2 pixels)
    // Pixel values: 0x0100 (256), 0x0200 (512), 0xFF00 (65280), 0x00FF (255 - low byte only)
    // Little endian buffer: [0x00, 0x01, 0x00, 0x02, 0x00, 0xFF, 0xFF, 0x00]
    let data_u16: Vec<u8> = vec![
        0x00, 0x01, // 256 -> MSB 1
        0x00, 0x02, // 512 -> MSB 2
        0x00, 0xFF, // 65280 -> MSB 255
        0xFF, 0x00, // 255 -> MSB 0
    ];

    let device_id = "cam1";

    backend_handle.send_event(BackendEvent::ImageReceived {
        device_id: device_id.to_string(),
        size: [2, 2],
        data: data_u16,
    });

    app.process_backend_events();

    // Verify active stream tracking
    assert_eq!(app.active_video_stream, Some(device_id.to_string()));

    // Verify image conversion
    let (image, _) = app.images.get(device_id).expect("Image should be stored");
    assert_eq!(image.width(), 2);
    assert_eq!(image.height(), 2);

    // Check pixel values (MSB only)
    // ColorImage pixels are Color32 (RGBA).
    // from_gray sets R=G=B=val.
    let pixels = &image.pixels;
    assert_eq!(pixels[0].r(), 1);
    assert_eq!(pixels[1].r(), 2);
    assert_eq!(pixels[2].r(), 255);
    assert_eq!(pixels[3].r(), 0);
}

#[test]
fn test_camera_auto_scale() {
    let (channels, mut backend_handle) = create_channels();
    let mut app = DaqGuiApp::new_with_channels(channels);
    // app.auto_scale is true by default

    // Input: [100, 200] (2 pixels)
    // Min: 100, Max: 200, Range: 100
    // Val 100 -> 0
    // Val 200 -> 255
    let data_u16: Vec<u8> = vec![
        100, 0, // 100
        200, 0, // 200
    ];

    let device_id = "cam_scale";
    backend_handle.send_event(BackendEvent::ImageReceived {
        device_id: device_id.to_string(),
        size: [2, 1],
        data: data_u16,
    });

    app.process_backend_events();

    let (image, _) = app.images.get(device_id).expect("Image should be stored");
    let pixels = &image.pixels;

    // Verify contrast stretch
    assert_eq!(pixels[0].r(), 0, "Min value should map to 0");
    assert_eq!(pixels[1].r(), 255, "Max value should map to 255");
}
