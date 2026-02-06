use hardware::config::{load_device_manifest_v2, InstrumentManifest, ResponseType};
use hardware::drivers::generic_scpi::GenericScpiDriver;
use hardware::drivers::mock_serial;
use hardware::drivers::parsing::{parse_response, ParsedValue};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

fn build_manifest() -> InstrumentManifest {
    let mut commands = HashMap::new();
    commands.insert(
        "read_value".to_string(),
        hardware::config::CommandProfile {
            template: "READ?".to_string(),
            response_type: ResponseType::Float,
            query: true,
        },
    );
    commands.insert(
        "set_value".to_string(),
        hardware::config::CommandProfile {
            template: "SET {{ value }}".to_string(),
            response_type: ResponseType::None,
            query: false,
        },
    );

    InstrumentManifest {
        schema_version: 2,
        name: "Example".to_string(),
        version: "1.0.0".to_string(),
        connection: hardware::config::ConnectionConfigV2::Serial {
            baud_rate: 9600,
            terminator: "\r\n".to_string(),
            timeout_ms: 1000,
        },
        settings: hardware::config::SettingsConfig::default(),
        commands,
        capabilities: hardware::config::CapabilityMapping {
            readable: Some(hardware::config::ReadableMapping {
                read: "read_value".to_string(),
            }),
            ..hardware::config::CapabilityMapping::default()
        },
        instances: Vec::new(),
        ui: None,
    }
}

#[tokio::test]
async fn test_generic_scpi_exec_and_parse() {
    let manifest = build_manifest();
    let (port, mut harness) = mock_serial::new();
    let shared = Arc::new(Mutex::new(
        Box::new(port) as hardware::drivers::generic_scpi::DynDeviceIo
    ));
    let driver = GenericScpiDriver::new(manifest, shared).unwrap();

    let task = tokio::spawn(async move { driver.exec("read_value", ()).await.unwrap() });

    harness.expect_write(b"READ?\r\n").await;
    harness.send_response(b"1.23\r\n").unwrap();

    let response = task.await.unwrap();
    assert_eq!(response, "1.23");

    let parsed = parse_response(&response, ResponseType::Float).unwrap();
    assert_eq!(parsed, ParsedValue::Float(1.23));
}

#[tokio::test]
async fn test_generic_scpi_set_command() {
    let mut manifest = build_manifest();
    manifest.capabilities.readable = None;
    let (port, mut harness) = mock_serial::new();
    let shared = Arc::new(Mutex::new(
        Box::new(port) as hardware::drivers::generic_scpi::DynDeviceIo
    ));
    let driver = GenericScpiDriver::new(manifest, shared).unwrap();

    let task = tokio::spawn(async move {
        driver
            .exec("set_value", serde_json::json!({ "value": 5.0 }))
            .await
            .unwrap();
    });

    harness.expect_write(b"SET 5.0\r\n").await;
    task.await.unwrap();
}

#[test]
fn test_manifest_loader_reads_v2_example() {
    let mut path = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    path.pop();
    path.pop();
    path.push("config/devices/minimal_scpi_template.scpi.toml");
    let manifest = load_device_manifest_v2(&path).unwrap();
    assert_eq!(manifest.schema_version, 2);
}
