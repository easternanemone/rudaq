//! V4 Actor Integration Tests
//!
//! Tests for V4 Kameo actors to verify:
//! - Actor lifecycle (spawn, message handling, shutdown)
//! - Concurrent actor operation
//! - Error handling and supervision
//! - Data flow between actors

use kameo::Actor;
use std::time::Duration;
use tokio::time::timeout;
use v4_daq::actors::esp300::{ESP300, Home, MoveAbsolute, ReadPosition, Stop};
use v4_daq::actors::pvcam::{GetCapabilities, PVCAMActor, SetGain, SnapFrame};
use v4_daq::actors::scpi::{Identify, Query, ScpiActor};
use v4_daq::traits::camera_sensor::{CameraTiming, PixelFormat, TriggerMode};

/// Test SCPI actor lifecycle
#[tokio::test]
async fn test_scpi_actor_lifecycle() {
    // Spawn actor
    let actor = ScpiActor::spawn(ScpiActor::mock("test_scpi".to_string()));

    // Send messages
    let idn = actor
        .ask(Identify)
        .await
        .expect("Identify failed");
    assert!(idn.contains("MOCK"), "Expected mock response, got: {}", idn);

    let response = actor
        .ask(Query {
            cmd: "*STB?".to_string(),
        })
        .await
        .expect("Query failed");
    assert!(response.contains("MOCK_RESPONSE"), "Expected mock response, got: {}", response);

    // Shutdown
    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test ESP300 actor lifecycle
#[tokio::test]
async fn test_esp300_actor_lifecycle() {
    // Spawn actor
    let actor = ESP300::spawn(ESP300::mock("test_esp300".to_string(), 3));

    // Home axis
    actor
        .ask(Home { axis: 0 })
        .await
        .expect("Home failed");

    // Move axis
    actor
        .ask(MoveAbsolute {
            axis: 0,
            position: 10.0,
        })
        .await
        .expect("Move failed");

    // Read position
    let position = actor
        .ask(ReadPosition { axis: 0 })
        .await
        .expect("Read position failed");
    assert!(position >= 0.0); // Mock returns 0.0

    // Stop
    actor
        .ask(Stop { axis: None })
        .await
        .expect("Stop failed");

    // Shutdown
    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test PVCAM actor lifecycle
#[tokio::test]
async fn test_pvcam_actor_lifecycle() {
    // Spawn actor
    let actor = PVCAMActor::spawn(PVCAMActor::mock(
        "test_pvcam".to_string(),
        "PrimeBSI".to_string(),
    ));

    // Get capabilities
    let caps = actor
        .ask(GetCapabilities)
        .await
        .expect("Get capabilities failed");
    assert_eq!(caps.sensor_width, 2048);
    assert_eq!(caps.sensor_height, 2048);

    // Set gain
    actor
        .ask(SetGain { gain: 10 })
        .await
        .expect("Set gain failed");

    // Snap frame
    let frame = actor
        .ask(SnapFrame {
            timing: CameraTiming {
                exposure_us: 50_000,
                frame_period_ms: 55.0,
                trigger_mode: TriggerMode::Internal,
            },
        })
        .await
        .expect("Snap failed");

    assert!(frame.width > 0);
    assert!(frame.height > 0);
    assert!(!frame.pixel_data.is_empty());

    // Shutdown
    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test concurrent actors
#[tokio::test]
async fn test_concurrent_actors() {
    // Spawn multiple actors concurrently
    let scpi_actor = ScpiActor::spawn(ScpiActor::mock("scpi1".to_string()));
    let esp_actor = ESP300::spawn(ESP300::mock("esp1".to_string(), 3));
    let pvcam_actor = PVCAMActor::spawn(PVCAMActor::mock(
        "pvcam1".to_string(),
        "Camera1".to_string(),
    ));

    // Send messages to all actors concurrently
    let (scpi_result, esp_result, pvcam_result) = tokio::join!(
        scpi_actor.ask(Identify),
        esp_actor.ask(ReadPosition { axis: 0 }),
        pvcam_actor.ask(GetCapabilities)
    );

    // Verify all succeeded
    assert!(scpi_result.is_ok());
    assert!(esp_result.is_ok());
    assert!(pvcam_result.is_ok());

    // Shutdown all
    scpi_actor.kill();
    esp_actor.kill();
    pvcam_actor.kill();

    tokio::join!(
        scpi_actor.wait_for_shutdown(),
        esp_actor.wait_for_shutdown(),
        pvcam_actor.wait_for_shutdown()
    );
}

/// Test actor timeout handling
#[tokio::test]
async fn test_actor_timeout() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_timeout".to_string()));

    // Normal operation should complete quickly
    let result = timeout(Duration::from_secs(1), actor.ask(Identify)).await;
    assert!(result.is_ok(), "Normal operation should not timeout");

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test multiple message sequences
#[tokio::test]
async fn test_message_sequence() {
    let actor = ESP300::spawn(ESP300::mock("test_sequence".to_string(), 3));

    // Sequence of operations
    for i in 0..10 {
        let position = (i as f64) * 5.0;
        actor
            .ask(MoveAbsolute { axis: 0, position })
            .await
            .expect("Move failed");

        let current_pos = actor
            .ask(ReadPosition { axis: 0 })
            .await
            .expect("Read failed");

        assert!(current_pos >= 0.0);
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test graceful shutdown with active operations
#[tokio::test]
async fn test_graceful_shutdown() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_shutdown".to_string()));

    // Send multiple messages
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let actor_clone = actor.clone();
            tokio::spawn(async move {
                actor_clone
                    .ask(Query {
                        cmd: format!("TEST{}?", i),
                    })
                    .await
            })
        })
        .collect();

    // Wait a bit for messages to be processing
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Shutdown
    actor.kill();
    actor.wait_for_shutdown().await;

    // Some messages may have completed before shutdown
    for handle in handles {
        let _ = handle.await;
    }
}

/// Test actor isolation (one actor error doesn't affect others)
#[tokio::test]
async fn test_actor_isolation() {
    let actor1 = ScpiActor::spawn(ScpiActor::mock("actor1".to_string()));
    let actor2 = ScpiActor::spawn(ScpiActor::mock("actor2".to_string()));

    // Both actors should work independently
    let result1 = actor1.ask(Identify).await;
    let result2 = actor2.ask(Identify).await;

    assert!(result1.is_ok());
    assert!(result2.is_ok());

    // Kill actor1
    actor1.kill();
    actor1.wait_for_shutdown().await;

    // Actor2 should still work
    let result3 = actor2.ask(Identify).await;
    assert!(result3.is_ok());

    actor2.kill();
    actor2.wait_for_shutdown().await;
}

/// Test ESP300 axis boundary validation
#[tokio::test]
async fn test_esp300_axis_validation() {
    let actor = ESP300::spawn(ESP300::mock("test_validation".to_string(), 3));

    // Valid axis (0, 1, 2)
    let result = actor.ask(ReadPosition { axis: 0 }).await;
    assert!(result.is_ok());

    // Invalid axis (out of range)
    let result = actor.ask(ReadPosition { axis: 5 }).await;
    // The actor.ask returns Result<f64, SendError>, but ESP300 validates axis and returns Err
    // So we expect the Result to be Err (not Ok(Err(_)))
    assert!(result.is_err(), "Should have returned error for invalid axis");

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test PVCAM capabilities query
#[tokio::test]
async fn test_pvcam_capabilities() {
    let actor = PVCAMActor::spawn(PVCAMActor::mock(
        "test_caps".to_string(),
        "TestCamera".to_string(),
    ));

    let caps = actor.ask(GetCapabilities).await.expect("Get capabilities failed");

    // Verify expected capabilities
    assert_eq!(caps.sensor_width, 2048);
    assert_eq!(caps.sensor_height, 2048);
    assert!(caps.pixel_formats.contains(&PixelFormat::Mono16));
    assert_eq!(caps.max_binning_x, 8);
    assert_eq!(caps.max_binning_y, 8);
    assert!(caps.min_exposure_us > 0);
    assert!(caps.max_exposure_us > caps.min_exposure_us);

    actor.kill();
    actor.wait_for_shutdown().await;
}
