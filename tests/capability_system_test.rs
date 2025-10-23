//! Integration tests for the capability-based module instrument assignment system.
//!
//! Tests capability discovery, proxy creation, and command routing through
//! the InstrumentCommand::Capability variant.

use rust_daq::core::{Instrument, InstrumentCommand};
use rust_daq::instrument::capabilities::{
    create_proxy, power_measurement_capability_id, position_control_capability_id,
};
use rust_daq::instrument::mock::MockInstrument;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_capability_discovery() {
    // Create a mock instrument that advertises no capabilities by default
    let mock = MockInstrument::new();
    let capabilities = mock.capabilities();
    assert_eq!(
        capabilities.len(),
        0,
        "MockInstrument should have no capabilities by default"
    );

    // Note: Newport1830C and MaiTai now advertise PowerMeasurement capability
    // This would be tested with actual instrument instances in integration tests
}

#[tokio::test]
async fn test_position_control_proxy_creation() {
    let (tx, mut rx) = mpsc::channel(32);
    let proxy_handle = create_proxy(position_control_capability_id(), "stage", tx)
        .expect("Failed to create PositionControl proxy");

    let proxy = proxy_handle
        .as_position_control()
        .expect("Should be a PositionControl proxy");

    // Test move_absolute command
    proxy
        .move_absolute(1, 100.0)
        .await
        .expect("move_absolute should succeed");

    let command = rx.recv().await.expect("Should receive command");
    match command {
        InstrumentCommand::Capability {
            capability,
            operation,
            parameters,
        } => {
            assert_eq!(capability, position_control_capability_id());
            assert_eq!(operation, "move_absolute");
            assert_eq!(parameters.len(), 2);
            assert_eq!(parameters[0].as_i64(), Some(1));
            assert_eq!(parameters[1].as_f64(), Some(100.0));
        }
        _ => panic!("Expected Capability command, got {:?}", command),
    }
}

#[tokio::test]
async fn test_position_control_proxy_move_relative() {
    let (tx, mut rx) = mpsc::channel(32);
    let proxy_handle = create_proxy(position_control_capability_id(), "stage", tx)
        .expect("Failed to create PositionControl proxy");

    let proxy = proxy_handle
        .as_position_control()
        .expect("Should be a PositionControl proxy");

    proxy
        .move_relative(2, -5.5)
        .await
        .expect("move_relative should succeed");

    let command = rx.recv().await.expect("Should receive command");
    match command {
        InstrumentCommand::Capability {
            capability,
            operation,
            parameters,
        } => {
            assert_eq!(capability, position_control_capability_id());
            assert_eq!(operation, "move_relative");
            assert_eq!(parameters.len(), 2);
            assert_eq!(parameters[0].as_i64(), Some(2));
            assert_eq!(parameters[1].as_f64(), Some(-5.5));
        }
        _ => panic!("Expected Capability command, got {:?}", command),
    }
}

#[tokio::test]
async fn test_position_control_proxy_stop() {
    let (tx, mut rx) = mpsc::channel(32);
    let proxy_handle = create_proxy(position_control_capability_id(), "stage", tx)
        .expect("Failed to create PositionControl proxy");

    let proxy = proxy_handle
        .as_position_control()
        .expect("Should be a PositionControl proxy");

    proxy.stop(3).await.expect("stop should succeed");

    let command = rx.recv().await.expect("Should receive command");
    match command {
        InstrumentCommand::Capability {
            capability,
            operation,
            parameters,
        } => {
            assert_eq!(capability, position_control_capability_id());
            assert_eq!(operation, "stop");
            assert_eq!(parameters.len(), 1);
            assert_eq!(parameters[0].as_i64(), Some(3));
        }
        _ => panic!("Expected Capability command, got {:?}", command),
    }
}

#[tokio::test]
async fn test_power_measurement_proxy_creation() {
    let (tx, mut rx) = mpsc::channel(32);
    let proxy_handle = create_proxy(power_measurement_capability_id(), "power_meter", tx)
        .expect("Failed to create PowerMeasurement proxy");

    let proxy = proxy_handle
        .as_power_measurement()
        .expect("Should be a PowerMeasurement proxy");

    // Test start_sampling command
    proxy
        .start_sampling()
        .await
        .expect("start_sampling should succeed");

    let command = rx.recv().await.expect("Should receive command");
    match command {
        InstrumentCommand::Capability {
            capability,
            operation,
            parameters,
        } => {
            assert_eq!(capability, power_measurement_capability_id());
            assert_eq!(operation, "start_sampling");
            assert_eq!(parameters.len(), 0);
        }
        _ => panic!("Expected Capability command, got {:?}", command),
    }
}

#[tokio::test]
async fn test_power_measurement_proxy_stop_sampling() {
    let (tx, mut rx) = mpsc::channel(32);
    let proxy_handle = create_proxy(power_measurement_capability_id(), "power_meter", tx)
        .expect("Failed to create PowerMeasurement proxy");

    let proxy = proxy_handle
        .as_power_measurement()
        .expect("Should be a PowerMeasurement proxy");

    proxy
        .stop_sampling()
        .await
        .expect("stop_sampling should succeed");

    let command = rx.recv().await.expect("Should receive command");
    match command {
        InstrumentCommand::Capability {
            capability,
            operation,
            parameters,
        } => {
            assert_eq!(capability, power_measurement_capability_id());
            assert_eq!(operation, "stop_sampling");
            assert_eq!(parameters.len(), 0);
        }
        _ => panic!("Expected Capability command, got {:?}", command),
    }
}

#[tokio::test]
async fn test_power_measurement_proxy_set_range() {
    let (tx, mut rx) = mpsc::channel(32);
    let proxy_handle = create_proxy(power_measurement_capability_id(), "power_meter", tx)
        .expect("Failed to create PowerMeasurement proxy");

    let proxy = proxy_handle
        .as_power_measurement()
        .expect("Should be a PowerMeasurement proxy");

    proxy
        .set_range(0.001)
        .await
        .expect("set_range should succeed");

    let command = rx.recv().await.expect("Should receive command");
    match command {
        InstrumentCommand::Capability {
            capability,
            operation,
            parameters,
        } => {
            assert_eq!(capability, power_measurement_capability_id());
            assert_eq!(operation, "set_range");
            assert_eq!(parameters.len(), 1);
            assert_eq!(parameters[0].as_f64(), Some(0.001));
        }
        _ => panic!("Expected Capability command, got {:?}", command),
    }
}

#[tokio::test]
async fn test_capability_type_matching() {
    let (tx, _rx) = mpsc::channel(32);

    // Create PositionControl proxy
    let pos_proxy = create_proxy(position_control_capability_id(), "stage", tx.clone())
        .expect("Failed to create PositionControl proxy");

    assert!(
        pos_proxy.as_position_control().is_some(),
        "Should cast to PositionControl"
    );
    assert!(
        pos_proxy.as_power_measurement().is_none(),
        "Should not cast to PowerMeasurement"
    );

    // Create PowerMeasurement proxy
    let power_proxy = create_proxy(power_measurement_capability_id(), "meter", tx)
        .expect("Failed to create PowerMeasurement proxy");

    assert!(
        power_proxy.as_power_measurement().is_some(),
        "Should cast to PowerMeasurement"
    );
    assert!(
        power_proxy.as_position_control().is_none(),
        "Should not cast to PositionControl"
    );
}

#[tokio::test]
async fn test_capability_proxy_handle_capability_id() {
    let (tx, _rx) = mpsc::channel(32);

    let pos_proxy = create_proxy(position_control_capability_id(), "stage", tx.clone())
        .expect("Failed to create PositionControl proxy");
    assert_eq!(
        pos_proxy.capability_id(),
        position_control_capability_id(),
        "PositionControl proxy should report correct capability ID"
    );

    let power_proxy = create_proxy(power_measurement_capability_id(), "meter", tx)
        .expect("Failed to create PowerMeasurement proxy");
    assert_eq!(
        power_proxy.capability_id(),
        power_measurement_capability_id(),
        "PowerMeasurement proxy should report correct capability ID"
    );
}

#[tokio::test]
async fn test_unsupported_capability_creation() {
    let (tx, _rx) = mpsc::channel(32);

    // Try to create a proxy with an invalid TypeId
    let invalid_type_id = std::any::TypeId::of::<String>();
    let result = create_proxy(invalid_type_id, "test", tx);

    assert!(
        result.is_err(),
        "Should fail to create proxy for unsupported capability"
    );
    if let Err(err) = result {
        assert!(
            err.to_string().contains("Unsupported capability"),
            "Error should mention unsupported capability, got: {}",
            err
        );
    }
}

#[cfg(feature = "instrument_serial")]
#[tokio::test]
async fn test_newport_1830c_advertises_power_measurement() {
    use rust_daq::instrument::newport_1830c::Newport1830C;

    let newport = Newport1830C::new("test_newport");
    let capabilities = newport.capabilities();

    assert_eq!(
        capabilities.len(),
        1,
        "Newport1830C should advertise 1 capability"
    );
    assert_eq!(
        capabilities[0],
        power_measurement_capability_id(),
        "Newport1830C should advertise PowerMeasurement capability"
    );
}

#[cfg(feature = "instrument_serial")]
#[tokio::test]
async fn test_maitai_advertises_power_measurement() {
    use rust_daq::instrument::maitai::MaiTai;

    let maitai = MaiTai::new("test_maitai");
    let capabilities = maitai.capabilities();

    assert_eq!(
        capabilities.len(),
        1,
        "MaiTai should advertise 1 capability"
    );
    assert_eq!(
        capabilities[0],
        power_measurement_capability_id(),
        "MaiTai should advertise PowerMeasurement capability"
    );
}