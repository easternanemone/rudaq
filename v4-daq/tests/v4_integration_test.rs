//! V4 Integration Tests (Working Actors Only)
//!
//! Tests SCPI and ESP300 actors to verify V4 architecture fundamentals.

use kameo::Actor;
use std::time::Duration;
use tokio::time::timeout;
use v4_daq::actors::esp300::{ESP300, Home, MoveAbsolute, ReadPosition, Stop};
use v4_daq::actors::scpi::{Identify, Query, ScpiActor};

/// Test SCPI actor lifecycle
#[tokio::test]
async fn test_scpi_lifecycle() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_scpi".to_string()));

    let idn = actor
        .ask(Identify)
        .await
        .expect("Send failed")
        .expect("Identify failed");
    assert!(idn.contains("Mock SCPI"));

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test ESP300 actor lifecycle
#[tokio::test]
async fn test_esp300_lifecycle() {
    let actor = ESP300::spawn(ESP300::mock("test_esp300".to_string(), 3));

    actor
        .ask(Home { axis: 0 })
        .await
        .expect("Send failed")
        .expect("Home failed");

    actor
        .ask(MoveAbsolute {
            axis: 0,
            position: 10.0,
        })
        .await
        .expect("Send failed")
        .expect("Move failed");

    let position = actor
        .ask(ReadPosition { axis: 0 })
        .await
        .expect("Send failed")
        .expect("Read position failed");
    assert!(position >= 0.0);

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test concurrent actors
#[tokio::test]
async fn test_concurrent_actors() {
    let scpi1 = ScpiActor::spawn(ScpiActor::mock("scpi1".to_string()));
    let scpi2 = ScpiActor::spawn(ScpiActor::mock("scpi2".to_string()));
    let esp = ESP300::spawn(ESP300::mock("esp1".to_string(), 3));

    let (r1, r2, r3) = tokio::join!(
        scpi1.ask(Identify),
        scpi2.ask(Identify),
        esp.ask(ReadPosition { axis: 0 })
    );

    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());

    scpi1.kill();
    scpi2.kill();
    esp.kill();

    tokio::join!(
        scpi1.wait_for_shutdown(),
        scpi2.wait_for_shutdown(),
        esp.wait_for_shutdown()
    );
}

/// Test message sequence
#[tokio::test]
async fn test_message_sequence() {
    let actor = ESP300::spawn(ESP300::mock("test_seq".to_string(), 3));

    for i in 0..10 {
        let pos = (i as f64) * 5.0;
        actor
            .ask(MoveAbsolute { axis: 0, position: pos })
            .await
            .expect("Send failed")
            .expect("Move failed");
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test timeout handling
#[tokio::test]
async fn test_timeout() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_timeout".to_string()));

    let result = timeout(Duration::from_secs(1), actor.ask(Identify)).await;
    assert!(result.is_ok(), "Should complete within timeout");

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test actor isolation
#[tokio::test]
async fn test_actor_isolation() {
    let actor1 = ScpiActor::spawn(ScpiActor::mock("actor1".to_string()));
    let actor2 = ScpiActor::spawn(ScpiActor::mock("actor2".to_string()));

    assert!(actor1.ask(Identify).await.is_ok());
    assert!(actor2.ask(Identify).await.is_ok());

    actor1.kill();
    actor1.wait_for_shutdown().await;

    // Actor2 should still work after actor1 is killed
    assert!(actor2.ask(Identify).await.is_ok());

    actor2.kill();
    actor2.wait_for_shutdown().await;
}

/// Test ESP300 boundary validation
#[tokio::test]
async fn test_esp300_validation() {
    let actor = ESP300::spawn(ESP300::mock("test_val".to_string(), 3));

    // Valid axis
    assert!(actor.ask(ReadPosition { axis: 0 }).await.is_ok());
    assert!(actor.ask(ReadPosition { axis: 2 }).await.is_ok());

    // Invalid axis
    let result = actor.ask(ReadPosition { axis: 5 }).await;
    match result {
        Ok(Err(_)) => {}, // Expected: actor returns error
        _ => panic!("Should return error for invalid axis"),
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test graceful shutdown
#[tokio::test]
async fn test_graceful_shutdown() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_shutdown".to_string()));

    // Send multiple concurrent messages
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let a = actor.clone();
            tokio::spawn(async move {
                a.ask(Query {
                    cmd: format!("TEST{}?", i),
                })
                .await
            })
        })
        .collect();

    tokio::time::sleep(Duration::from_millis(10)).await;

    actor.kill();
    actor.wait_for_shutdown().await;

    // Collect results (some may have completed)
    for handle in handles {
        let _ = handle.await;
    }
}

/// Test multi-axis ESP300 operations
#[tokio::test]
async fn test_esp300_multi_axis() {
    let actor = ESP300::spawn(ESP300::mock("test_multi".to_string(), 3));

    // Home all axes
    for axis in 0..3 {
        actor
            .ask(Home { axis })
            .await
            .expect("Send failed")
            .expect("Home failed");
    }

    // Move all axes independently
    for axis in 0..3 {
        actor
            .ask(MoveAbsolute {
                axis,
                position: (axis as f64) * 10.0,
            })
            .await
            .expect("Send failed")
            .expect("Move failed");
    }

    // Stop all axes
    actor
        .ask(Stop { axis: None })
        .await
        .expect("Send failed")
        .expect("Stop failed");

    actor.kill();
    actor.wait_for_shutdown().await;
}
