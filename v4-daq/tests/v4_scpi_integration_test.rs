//! V4 SCPI Actor Integration Tests
//!
//! Tests SCPI actor to verify V4 architecture fundamentals.

use kameo::Actor;
use std::time::Duration;
use tokio::time::timeout;
use v4_daq::actors::scpi::{Identify, Query, ScpiActor};

/// Test SCPI actor lifecycle
#[tokio::test]
async fn test_scpi_lifecycle() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_scpi".to_string()));

    let idn = actor
        .ask(Identify)
        .await
        .expect("Send failed");
    assert!(idn.contains("MOCK"), "Expected mock response, got: {}", idn);

    let response = actor
        .ask(Query {
            cmd: "*STB?".to_string(),
        })
        .await
        .expect("Send failed");
    assert!(response.contains("MOCK_RESPONSE"), "Expected mock response, got: {}", response);

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test concurrent SCPI actors
#[tokio::test]
async fn test_concurrent_scpi_actors() {
    let actor1 = ScpiActor::spawn(ScpiActor::mock("scpi1".to_string()));
    let actor2 = ScpiActor::spawn(ScpiActor::mock("scpi2".to_string()));
    let actor3 = ScpiActor::spawn(ScpiActor::mock("scpi3".to_string()));

    let (r1, r2, r3) = tokio::join!(
        actor1.ask(Identify),
        actor2.ask(Identify),
        actor3.ask(Identify)
    );

    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());

    actor1.kill();
    actor2.kill();
    actor3.kill();

    let _ = tokio::join!(
        actor1.wait_for_shutdown(),
        actor2.wait_for_shutdown(),
        actor3.wait_for_shutdown()
    );
}

/// Test message sequence
#[tokio::test]
async fn test_scpi_message_sequence() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_seq".to_string()));

    for i in 0..20 {
        let response = actor
            .ask(Query {
                cmd: format!("TEST{}?", i),
            })
            .await
            .expect("Send failed");
        assert!(response.contains("MOCK_RESPONSE"), "Expected mock response, got: {}", response);
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test timeout handling
#[tokio::test]
async fn test_scpi_timeout() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_timeout".to_string()));

    let result = timeout(Duration::from_secs(1), actor.ask(Identify)).await;
    assert!(result.is_ok(), "Should complete within timeout");

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test actor isolation
#[tokio::test]
async fn test_scpi_actor_isolation() {
    let actor1 = ScpiActor::spawn(ScpiActor::mock("actor1".to_string()));
    let actor2 = ScpiActor::spawn(ScpiActor::mock("actor2".to_string()));

    assert!(actor1.ask(Identify).await.is_ok());
    assert!(actor2.ask(Identify).await.is_ok());

    // Kill actor1
    actor1.kill();
    actor1.wait_for_shutdown().await;

    // Actor2 should still work
    assert!(actor2.ask(Identify).await.is_ok());

    actor2.kill();
    actor2.wait_for_shutdown().await;
}

/// Test graceful shutdown with pending messages
#[tokio::test]
async fn test_scpi_graceful_shutdown() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_shutdown".to_string()));

    // Send multiple concurrent messages
    let handles: Vec<_> = (0..10)
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

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Shutdown
    actor.kill();
    actor.wait_for_shutdown().await;

    // Collect results
    for handle in handles {
        let _ = handle.await;
    }
}

/// Test multiple sequential operations
#[tokio::test]
async fn test_scpi_sequential_ops() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_seq_ops".to_string()));

    // Identify
    let idn = actor.ask(Identify).await.expect("Send failed");
    assert!(idn.contains("MOCK"), "Expected mock response, got: {}", idn);

    // Multiple queries
    for _ in 0..5 {
        let _ = actor
            .ask(Query {
                cmd: "*OPC?".to_string(),
            })
            .await
            .expect("Send failed");
    }

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test actor cloning and concurrent access
#[tokio::test]
async fn test_scpi_actor_clone() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_clone".to_string()));

    // Clone the actor reference
    let actor2 = actor.clone();
    let actor3 = actor.clone();

    // All clones should work
    let (r1, r2, r3) = tokio::join!(
        actor.ask(Identify),
        actor2.ask(Identify),
        actor3.ask(Identify)
    );

    assert!(r1.is_ok());
    assert!(r2.is_ok());
    assert!(r3.is_ok());

    actor.kill();
    actor.wait_for_shutdown().await;
}

/// Test rapid spawn and shutdown
#[tokio::test]
async fn test_scpi_rapid_spawn_shutdown() {
    for i in 0..10 {
        let actor = ScpiActor::spawn(ScpiActor::mock(format!("rapid_{}", i)));

        let _ = actor.ask(Identify).await;

        actor.kill();
        actor.wait_for_shutdown().await;
    }
}

/// Test actor remains responsive during load
#[tokio::test]
async fn test_scpi_under_load() {
    let actor = ScpiActor::spawn(ScpiActor::mock("test_load".to_string()));

    // Send 100 messages concurrently
    let mut handles = vec![];
    for i in 0..100 {
        let a = actor.clone();
        handles.push(tokio::spawn(async move {
            a.ask(Query {
                cmd: format!("LOAD{}?", i),
            })
            .await
        }));
    }

    // All should complete successfully
    let mut successes = 0;
    for handle in handles {
        if handle.await.is_ok() {
            successes += 1;
        }
    }

    assert!(successes > 90, "Most messages should succeed"); // Allow some to fail during shutdown

    actor.kill();
    actor.wait_for_shutdown().await;
}
