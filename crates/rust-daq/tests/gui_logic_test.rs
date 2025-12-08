#![cfg(not(target_arch = "wasm32"))]
use rust_daq::gui::{
    app::DaqGuiApp,
    create_channels, BackendCommand, BackendEvent, ConnectionStatus, DeviceInfo,
};

#[test]
fn test_gui_app_initialization() {
    let (channels, _backend_handle) = create_channels();
    let app = DaqGuiApp::new_with_channels(channels);

    assert_eq!(app.daemon_addr, "127.0.0.1:50051");
    assert!(matches!(app.connection_status, ConnectionStatus::Disconnected));
    assert!(app.devices.is_empty());
}

#[test]
fn test_gui_app_device_refresh() {
    let (channels, mut backend_handle) = create_channels();
    let mut app = DaqGuiApp::new_with_channels(channels);

    // Simulate backend sending devices
    let devices = vec![
        DeviceInfo {
            id: "dev1".to_string(),
            name: "Test Device".to_string(),
            driver_type: "mock".to_string(),
            capabilities: vec!["Readable".to_string()],
            is_connected: true,
        }
    ];

    // Send event from "backend"
    backend_handle.send_event(BackendEvent::DevicesRefreshed { devices: devices.clone() });

    // Process events in app
    app.process_backend_events();

    assert_eq!(app.devices.len(), 1);
    assert_eq!(app.devices[0].id, "dev1");
    assert_eq!(app.status_line, "Loaded 1 devices");
}

#[test]
fn test_gui_app_connection_status() {
    let (ui_channels, mut backend_handle) = create_channels();
    let mut app = DaqGuiApp::new_with_channels(ui_channels);

    backend_handle.send_event(BackendEvent::ConnectionChanged { 
        status: ConnectionStatus::Connected 
    });

    app.process_backend_events();

    assert!(matches!(app.connection_status, ConnectionStatus::Connected));
    assert_eq!(app.status_line, "Connected");
}
