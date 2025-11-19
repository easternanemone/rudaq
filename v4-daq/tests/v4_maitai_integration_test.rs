//! V4 MaiTai Laser Integration Tests
//!
//! Comprehensive test suite for MaiTai tunable laser V4 actor.
//! Tests wavelength control, shutter management, and measurement collection.

use kameo::actor::Actor;
use std::time::Duration;
use v4_daq::actors::maitai::{
    CloseShutter, GetShutterState, GetWavelength, MaiTai, Measure, OpenShutter, ReadPower,
    SetWavelength,
};
use v4_daq::traits::tunable_laser::Wavelength;

/// Test MaiTai actor lifecycle (spawn, run, shutdown)
#[tokio::test]
async fn test_maitai_lifecycle() {
    let actor = MaiTai::spawn(MaiTai::new());

    // Verify actor is alive
    assert!(actor.is_alive());

    // Verify we can send a message
    let wavelength = actor
        .ask(GetWavelength)
        .await
        .expect("GetWavelength failed");
    assert_eq!(wavelength.nm, 800.0); // Default Ti:Sapphire wavelength

    // Clean shutdown
    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test wavelength control (set and read)
#[tokio::test]
async fn test_wavelength_control() {
    let actor = MaiTai::spawn(MaiTai::new());

    // Set wavelength to 850 nm
    actor
        .ask(SetWavelength {
            wavelength: Wavelength { nm: 850.0 },
        })
        .await
        .expect("SetWavelength failed");

    // Read wavelength back
    let wavelength = actor
        .ask(GetWavelength)
        .await
        .expect("GetWavelength failed");
    assert_eq!(wavelength.nm, 850.0);

    // Set another wavelength
    actor
        .ask(SetWavelength {
            wavelength: Wavelength { nm: 920.0 },
        })
        .await
        .expect("SetWavelength failed");

    let wavelength = actor
        .ask(GetWavelength)
        .await
        .expect("GetWavelength failed");
    assert_eq!(wavelength.nm, 920.0);

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test shutter control (open and close)
#[tokio::test]
async fn test_shutter_control() {
    use v4_daq::traits::tunable_laser::ShutterState;

    let actor = MaiTai::spawn(MaiTai::new());

    // Verify shutter starts closed
    let state = actor
        .ask(GetShutterState)
        .await
        .expect("GetShutterState failed");
    match state {
        ShutterState::Closed => (), // Expected
        _ => panic!("Shutter should start closed"),
    }

    // Open shutter
    actor
        .ask(OpenShutter)
        .await
        .expect("OpenShutter failed");

    let state = actor
        .ask(GetShutterState)
        .await
        .expect("GetShutterState failed");
    match state {
        ShutterState::Open => (), // Expected
        _ => panic!("Shutter should be open"),
    }

    // Close shutter
    actor
        .ask(CloseShutter)
        .await
        .expect("CloseShutter failed");

    let state = actor
        .ask(GetShutterState)
        .await
        .expect("GetShutterState failed");
    match state {
        ShutterState::Closed => (), // Expected
        _ => panic!("Shutter should be closed"),
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test power reading
#[tokio::test]
async fn test_power_reading() {
    let actor = MaiTai::spawn(MaiTai::new());

    // Read power (mock mode returns 0.0)
    let power = actor
        .ask(ReadPower)
        .await
        .expect("ReadPower failed");

    // In mock mode, should return 0.0
    assert_eq!(power, 0.0);

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test measurement collection
#[tokio::test]
async fn test_measure_collection() {
    use v4_daq::traits::tunable_laser::ShutterState;

    let actor = MaiTai::spawn(MaiTai::new());

    // Set wavelength first
    actor
        .ask(SetWavelength {
            wavelength: Wavelength { nm: 880.0 },
        })
        .await
        .expect("SetWavelength failed");

    // Open shutter
    actor
        .ask(OpenShutter)
        .await
        .expect("OpenShutter failed");

    // Collect measurement
    let measurement = actor
        .ask(Measure)
        .await
        .expect("Measure failed");

    // Verify measurement fields
    assert_eq!(measurement.wavelength.nm, 880.0);
    assert_eq!(measurement.power_watts, 0.0); // Mock mode
    assert!(measurement.timestamp_ns > 0);

    match measurement.shutter {
        ShutterState::Open => (), // Expected
        _ => panic!("Shutter should be open in measurement"),
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test concurrent wavelength and shutter operations
#[tokio::test]
async fn test_concurrent_operations() {
    use v4_daq::traits::tunable_laser::ShutterState;

    let actor = MaiTai::spawn(MaiTai::new());

    // Perform concurrent operations
    let (r1, r2, r3) = tokio::join!(
        async {
            actor
                .ask(SetWavelength {
                    wavelength: Wavelength { nm: 950.0 },
                })
                .await
        },
        async { actor.ask(OpenShutter).await },
        async { actor.ask(ReadPower).await }
    );

    // All operations should succeed
    r1.expect("SetWavelength failed");
    r2.expect("OpenShutter failed");
    r3.expect("ReadPower failed");

    // Verify final state
    let wavelength = actor
        .ask(GetWavelength)
        .await
        .expect("GetWavelength failed");
    assert_eq!(wavelength.nm, 950.0);

    let shutter = actor
        .ask(GetShutterState)
        .await
        .expect("GetShutterState failed");
    match shutter {
        ShutterState::Open => (), // Expected
        _ => panic!("Shutter should be open"),
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test repeated measurements
#[tokio::test]
async fn test_repeated_measurements() {
    let actor = MaiTai::spawn(MaiTai::new());

    // Set up initial state
    actor
        .ask(SetWavelength {
            wavelength: Wavelength { nm: 800.0 },
        })
        .await
        .expect("SetWavelength failed");

    actor
        .ask(OpenShutter)
        .await
        .expect("OpenShutter failed");

    // Collect multiple measurements
    let mut measurements = Vec::new();
    for _ in 0..5 {
        let measurement = actor
            .ask(Measure)
            .await
            .expect("Measure failed");

        measurements.push(measurement);

        // Small delay between measurements
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Verify we got all measurements
    assert_eq!(measurements.len(), 5);

    // Verify measurements are in ascending timestamp order
    for i in 1..measurements.len() {
        assert!(measurements[i].timestamp_ns >= measurements[i - 1].timestamp_ns);
    }

    // All should have same wavelength (since we didn't change it)
    for measurement in &measurements {
        assert_eq!(measurement.wavelength.nm, 800.0);
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}
