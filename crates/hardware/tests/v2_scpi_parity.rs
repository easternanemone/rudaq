use driver_mock::{MockLaser, MockMode, MockPowerMeter, MockStage};
use hardware::capabilities::{Movable, Readable, ShutterControl, WavelengthTunable};
use hardware::config::{
    CapabilityMapping, CommandProfile, ConnectionConfigV2, InstrumentManifest, MovableMapping,
    ReadableMapping, ResponseType, SettingsConfig, ShutterControlMapping, WavelengthTunableMapping,
};
use hardware::drivers::generic_scpi::{DynDeviceIo, GenericScpiDriver};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::sync::Mutex;

const TERMINATOR: &str = "\r\n";

fn build_power_meter_manifest() -> InstrumentManifest {
    let mut commands = HashMap::new();
    commands.insert(
        "read".to_string(),
        CommandProfile {
            template: "READ?".to_string(),
            response_type: ResponseType::Float,
            query: true,
        },
    );

    InstrumentManifest {
        schema_version: 2,
        name: "Mock Power Meter".to_string(),
        version: "1.0.0".to_string(),
        connection: ConnectionConfigV2::Serial {
            baud_rate: 9600,
            terminator: TERMINATOR.to_string(),
            timeout_ms: 1000,
        },
        settings: SettingsConfig::default(),
        commands,
        capabilities: CapabilityMapping {
            readable: Some(ReadableMapping {
                read: "read".to_string(),
            }),
            ..CapabilityMapping::default()
        },
        ui: None,
        instances: Vec::new(),
    }
}

fn build_laser_manifest() -> InstrumentManifest {
    let mut commands = HashMap::new();
    commands.insert(
        "set_wavelength".to_string(),
        CommandProfile {
            template: "WAVE {{ value }}".to_string(),
            response_type: ResponseType::None,
            query: false,
        },
    );
    commands.insert(
        "get_wavelength".to_string(),
        CommandProfile {
            template: "WAVE?".to_string(),
            response_type: ResponseType::Float,
            query: true,
        },
    );
    commands.insert(
        "open_shutter".to_string(),
        CommandProfile {
            template: "SHUT:OPEN".to_string(),
            response_type: ResponseType::None,
            query: false,
        },
    );
    commands.insert(
        "close_shutter".to_string(),
        CommandProfile {
            template: "SHUT:CLOSE".to_string(),
            response_type: ResponseType::None,
            query: false,
        },
    );
    commands.insert(
        "is_shutter_open".to_string(),
        CommandProfile {
            template: "SHUT?".to_string(),
            response_type: ResponseType::Boolean,
            query: true,
        },
    );

    InstrumentManifest {
        schema_version: 2,
        name: "Mock Laser".to_string(),
        version: "1.0.0".to_string(),
        connection: ConnectionConfigV2::Serial {
            baud_rate: 9600,
            terminator: TERMINATOR.to_string(),
            timeout_ms: 1000,
        },
        settings: SettingsConfig::default(),
        commands,
        capabilities: CapabilityMapping {
            wavelength_tunable: Some(WavelengthTunableMapping {
                set_wavelength: "set_wavelength".to_string(),
                get_wavelength: "get_wavelength".to_string(),
            }),
            shutter_control: Some(ShutterControlMapping {
                open_shutter: "open_shutter".to_string(),
                close_shutter: "close_shutter".to_string(),
                is_shutter_open: "is_shutter_open".to_string(),
            }),
            ..CapabilityMapping::default()
        },
        ui: None,
        instances: Vec::new(),
    }
}

fn build_stage_manifest() -> InstrumentManifest {
    let mut commands = HashMap::new();
    commands.insert(
        "move_abs".to_string(),
        CommandProfile {
            template: "MOVE:ABS {{ value }}".to_string(),
            response_type: ResponseType::None,
            query: false,
        },
    );
    commands.insert(
        "move_rel".to_string(),
        CommandProfile {
            template: "MOVE:REL {{ value }}".to_string(),
            response_type: ResponseType::None,
            query: false,
        },
    );
    commands.insert(
        "position".to_string(),
        CommandProfile {
            template: "POS?".to_string(),
            response_type: ResponseType::Float,
            query: true,
        },
    );
    commands.insert(
        "stop".to_string(),
        CommandProfile {
            template: "STOP".to_string(),
            response_type: ResponseType::None,
            query: false,
        },
    );
    commands.insert(
        "wait_settled".to_string(),
        CommandProfile {
            template: "WAIT".to_string(),
            response_type: ResponseType::None,
            query: false,
        },
    );

    InstrumentManifest {
        schema_version: 2,
        name: "Mock Stage".to_string(),
        version: "1.0.0".to_string(),
        connection: ConnectionConfigV2::Serial {
            baud_rate: 9600,
            terminator: TERMINATOR.to_string(),
            timeout_ms: 1000,
        },
        settings: SettingsConfig::default(),
        commands,
        capabilities: CapabilityMapping {
            movable: Some(MovableMapping {
                move_abs: "move_abs".to_string(),
                move_rel: "move_rel".to_string(),
                position: "position".to_string(),
                stop: "stop".to_string(),
                wait_settled: Some("wait_settled".to_string()),
            }),
            ..CapabilityMapping::default()
        },
        ui: None,
        instances: Vec::new(),
    }
}

async fn run_power_meter_device(device: DuplexStream, meter: MockPowerMeter) {
    let mut reader = BufReader::new(device);
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer).await.unwrap();
        if read == 0 {
            break;
        }

        let command = String::from_utf8_lossy(&buffer)
            .trim_end_matches(&['\r', '\n'][..])
            .to_string();
        if command.is_empty() {
            continue;
        }

        match command.as_str() {
            "READ?" => {
                let reading = meter.read().await.unwrap();
                let response = format!("{}{}", reading, TERMINATOR);
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
                reader.get_mut().flush().await.unwrap();
            }
            _ => panic!("Unexpected command for power meter: {command}"),
        }
    }
}

async fn run_laser_device(device: DuplexStream, laser: MockLaser) {
    let mut reader = BufReader::new(device);
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer).await.unwrap();
        if read == 0 {
            break;
        }

        let command = String::from_utf8_lossy(&buffer)
            .trim_end_matches(&['\r', '\n'][..])
            .to_string();
        if command.is_empty() {
            continue;
        }

        if let Some(value) = command.strip_prefix("WAVE ") {
            let parsed: f64 = value.trim().parse().unwrap();
            laser.set_wavelength(parsed).await.unwrap();
            continue;
        }

        match command.as_str() {
            "WAVE?" => {
                let wavelength = laser.get_wavelength().await.unwrap();
                let response = format!("{}{}", wavelength, TERMINATOR);
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
                reader.get_mut().flush().await.unwrap();
            }
            "SHUT:OPEN" => {
                laser.open_shutter().await.unwrap();
            }
            "SHUT:CLOSE" => {
                laser.close_shutter().await.unwrap();
            }
            "SHUT?" => {
                let open = laser.is_shutter_open().await.unwrap();
                let response = format!("{}{}", u8::from(open), TERMINATOR);
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
                reader.get_mut().flush().await.unwrap();
            }
            _ => panic!("Unexpected command for laser: {command}"),
        }
    }
}

async fn run_stage_device(device: DuplexStream, stage: MockStage) {
    let mut reader = BufReader::new(device);
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer).await.unwrap();
        if read == 0 {
            break;
        }

        let command = String::from_utf8_lossy(&buffer)
            .trim_end_matches(&['\r', '\n'][..])
            .to_string();
        if command.is_empty() {
            continue;
        }

        if let Some(value) = command.strip_prefix("MOVE:ABS ") {
            let parsed: f64 = value.trim().parse().unwrap();
            stage.move_abs(parsed).await.unwrap();
            continue;
        }

        if let Some(value) = command.strip_prefix("MOVE:REL ") {
            let parsed: f64 = value.trim().parse().unwrap();
            stage.move_rel(parsed).await.unwrap();
            continue;
        }

        match command.as_str() {
            "POS?" => {
                let position = stage.position().await.unwrap();
                let response = format!("{}{}", position, TERMINATOR);
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
                reader.get_mut().flush().await.unwrap();
            }
            "STOP" => {
                stage.stop().await.unwrap();
            }
            "WAIT" => {
                stage.wait_settled().await.unwrap();
            }
            _ => panic!("Unexpected command for stage: {command}"),
        }
    }
}

#[tokio::test]
async fn test_scpi_power_meter_parity() {
    let manifest = build_power_meter_manifest();
    let (client, device) = tokio::io::duplex(1024);
    let shared = Arc::new(Mutex::new(Box::new(client) as DynDeviceIo));
    let driver = GenericScpiDriver::new(manifest, shared).unwrap();

    let device_meter = MockPowerMeter::builder()
        .base_power(1.234)
        .rng_seed(42)
        .build();
    let reference_meter = MockPowerMeter::builder()
        .base_power(1.234)
        .rng_seed(42)
        .build();

    let device_task = tokio::spawn(run_power_meter_device(device, device_meter));

    let scpi_reading = driver.read().await.unwrap();
    let expected_reading = reference_meter.read().await.unwrap();

    assert!(
        (scpi_reading - expected_reading).abs() < 1e-9,
        "Expected {expected_reading}, got {scpi_reading}"
    );

    drop(driver);
    device_task.await.unwrap();
}

#[tokio::test]
#[allow(clippy::float_cmp)]
async fn test_scpi_laser_parity() {
    let manifest = build_laser_manifest();
    let (client, device) = tokio::io::duplex(2048);
    let shared = Arc::new(Mutex::new(Box::new(client) as DynDeviceIo));
    let driver = GenericScpiDriver::new(manifest, shared).unwrap();

    let device_laser = MockLaser::new();
    let reference_laser = MockLaser::new();

    let device_task = tokio::spawn(run_laser_device(device, device_laser));

    driver.set_wavelength(750.0).await.unwrap();
    reference_laser.set_wavelength(750.0).await.unwrap();
    let scpi_wavelength = driver.get_wavelength().await.unwrap();
    let expected_wavelength = reference_laser.get_wavelength().await.unwrap();
    assert_eq!(scpi_wavelength, expected_wavelength);

    driver.open_shutter().await.unwrap();
    reference_laser.open_shutter().await.unwrap();
    let scpi_open = driver.is_shutter_open().await.unwrap();
    let expected_open = reference_laser.is_shutter_open().await.unwrap();
    assert_eq!(scpi_open, expected_open);

    driver.close_shutter().await.unwrap();
    reference_laser.close_shutter().await.unwrap();
    let scpi_closed = driver.is_shutter_open().await.unwrap();
    let expected_closed = reference_laser.is_shutter_open().await.unwrap();
    assert_eq!(scpi_closed, expected_closed);

    drop(driver);
    device_task.await.unwrap();
}

#[tokio::test]
#[allow(clippy::float_cmp)]
async fn test_scpi_stage_parity() {
    let manifest = build_stage_manifest();
    let (client, device) = tokio::io::duplex(2048);
    let shared = Arc::new(Mutex::new(Box::new(client) as DynDeviceIo));
    let driver = GenericScpiDriver::new(manifest, shared).unwrap();

    let device_stage = MockStage::builder().mode(MockMode::Instant).build();
    let reference_stage = MockStage::builder().mode(MockMode::Instant).build();

    let device_task = tokio::spawn(run_stage_device(device, device_stage));

    driver.move_abs(10.0).await.unwrap();
    reference_stage.move_abs(10.0).await.unwrap();
    let scpi_position = driver.position().await.unwrap();
    let expected_position = reference_stage.position().await.unwrap();
    assert_eq!(scpi_position, expected_position);

    driver.move_rel(-2.5).await.unwrap();
    reference_stage.move_rel(-2.5).await.unwrap();
    let scpi_position = driver.position().await.unwrap();
    let expected_position = reference_stage.position().await.unwrap();
    assert_eq!(scpi_position, expected_position);

    driver.wait_settled().await.unwrap();
    reference_stage.wait_settled().await.unwrap();

    driver.stop().await.unwrap();
    reference_stage.stop().await.unwrap();

    drop(driver);
    device_task.await.unwrap();
}
