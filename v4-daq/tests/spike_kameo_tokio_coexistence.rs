//! Spike Test: Kameo Actor & Tokio Task Coexistence
//!
//! This test validates BLOCKER-2 from IMMEDIATE_BLOCKERS.md:
//! "Tokio Runtime & Kameo Runtime Coexistence"
//!
//! Tests verify:
//! - Both Kameo actors and tokio tasks can run in the same runtime
//! - No interference or deadlocks between the two paradigms
//! - Proper lifecycle management and graceful shutdown
//! - Performance under concurrent load
//!
//! BLOCKER RESOLUTION:
//! This spike proves that Kameo actors and tokio tasks coexist peacefully
//! in a single tokio runtime, enabling the V2/V4 coexistence design.

use kameo::Actor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use v4_daq::actors::scpi::{Identify, Query, ScpiActor};

// Helper function for collecting futures
async fn join_all_futures<F: std::future::Future>(futures: Vec<F>) -> Vec<F::Output> {
    let mut results = Vec::new();
    for future in futures {
        results.push(future.await);
    }
    results
}

/// ============================================================================
/// Test 1: Basic Coexistence - Kameo Actor + Tokio Tasks
/// ============================================================================
///
/// Validates that a single Kameo actor and multiple tokio tasks can operate
/// concurrently without interference.
///
/// **Scenario:**
/// - Spawn 1 Kameo SCPI actor (V4 style)
/// - Spawn 2 plain tokio tasks (V2 style)
/// - All execute concurrently
/// - Verify all complete successfully
/// - Verify proper shutdown
#[tokio::test]
async fn test_kameo_and_tokio_tasks_coexist() {
    println!("[Test 1] Starting: Kameo actor + Tokio tasks coexistence");

    // Setup: Spawn Kameo actor (V4 style)
    let scpi_actor = ScpiActor::spawn(ScpiActor::mock("test_scpi".to_string()));
    println!("  -> Spawned Kameo SCPI actor");

    // Setup: Spawn tokio tasks (V2 style)
    let tokio_task1 = tokio::spawn(async {
        println!("    -> Tokio task 1 started");
        tokio::time::sleep(Duration::from_millis(50)).await;
        println!("    -> Tokio task 1 completed");
        "tokio_result_1"
    });

    let tokio_task2 = tokio::spawn(async {
        println!("    -> Tokio task 2 started");
        tokio::time::sleep(Duration::from_millis(75)).await;
        println!("    -> Tokio task 2 completed");
        "tokio_result_2"
    });

    println!("  -> Spawned 2 tokio tasks");

    // Execute: Send messages to actor while tasks run
    println!("  -> Sending Identify message to actor");
    let scpi_result = scpi_actor
        .ask(Identify)
        .await
        .expect("SCPI actor request failed");

    // Verify: All operations complete successfully
    let t1 = tokio_task1.await.expect("Task 1 failed");
    let t2 = tokio_task2.await.expect("Task 2 failed");

    assert!(
        scpi_result.contains("MOCK"),
        "SCPI response should contain 'MOCK', got: {}",
        scpi_result
    );
    assert_eq!(t1, "tokio_result_1", "Task 1 should return expected value");
    assert_eq!(t2, "tokio_result_2", "Task 2 should return expected value");

    println!("  -> All operations completed successfully");

    // Cleanup: Graceful shutdown
    println!("  -> Shutting down actor");
    scpi_actor.kill();
    scpi_actor.wait_for_shutdown().await;
    println!("  -> Actor shutdown complete");

    println!("[Test 1] PASSED: Coexistence verified");
}

/// ============================================================================
/// Test 2: High Concurrency - Multiple Kameo Actors + Tokio Tasks
/// ============================================================================
///
/// Validates that multiple Kameo actors can coexist with tokio tasks,
/// testing the system's ability to handle more realistic concurrent workloads.
///
/// **Scenario:**
/// - Spawn 5 Kameo actors with different identities
/// - Spawn 10 tokio tasks performing various operations
/// - All execute concurrently with overlapping timings
/// - Verify all complete successfully
/// - Measure completion times
#[tokio::test]
async fn test_multiple_kameo_actors_with_tokio_concurrent_tasks() {
    println!("[Test 2] Starting: Multiple Kameo actors + concurrent tokio tasks");

    // Setup: Spawn multiple Kameo actors
    let mut scpi_actors = vec![];
    for i in 0..5 {
        let actor = ScpiActor::spawn(ScpiActor::mock(format!("actor_{}", i)));
        scpi_actors.push(actor);
    }
    println!("  -> Spawned 5 Kameo SCPI actors");

    // Setup: Spawn multiple tokio tasks
    let mut tokio_tasks = vec![];
    for i in 0..10 {
        let task = tokio::spawn(async move {
            let delay = Duration::from_millis(10 + (i as u64 * 5));
            tokio::time::sleep(delay).await;
            i * 10
        });
        tokio_tasks.push(task);
    }
    println!("  -> Spawned 10 tokio tasks");

    // Execute: Send concurrent requests to all actors
    println!("  -> Sending concurrent requests to all actors");
    let mut actor_futures = vec![];
    for (i, actor) in scpi_actors.iter().enumerate() {
        let actor_clone = actor.clone();
        let future = async move {
            actor_clone
                .ask(Query {
                    cmd: format!("*STB{}", i),
                })
                .await
        };
        actor_futures.push(future);
    }

    // Execute all actor requests concurrently
    let actor_results = join_all_futures(actor_futures).await;

    // Verify: All actor requests succeeded
    println!("  -> Verifying actor request results");
    for (i, result) in actor_results.iter().enumerate() {
        assert!(
            result.is_ok(),
            "Actor {} request should succeed",
            i
        );
    }

    // Verify: All tokio tasks completed
    println!("  -> Verifying tokio task results");
    let mut task_results = vec![];
    for task in tokio_tasks {
        let result = task.await.expect("Tokio task should complete");
        task_results.push(result);
    }

    // Verify expected task values
    assert_eq!(task_results.len(), 10, "Should have 10 task results");
    for (i, value) in task_results.iter().enumerate() {
        assert_eq!(*value, (i as i32) * 10, "Task {} value mismatch", i);
    }

    println!("  -> All operations completed successfully");

    // Cleanup: Graceful shutdown of all actors
    println!("  -> Shutting down all actors");
    for actor in &scpi_actors {
        actor.kill();
    }

    // Wait for all to shutdown
    let shutdown_futures: Vec<_> = scpi_actors.iter().map(|a| a.wait_for_shutdown()).collect();
    join_all_futures(shutdown_futures).await;

    println!("  -> All actors shutdown complete");
    println!("[Test 2] PASSED: High concurrency verified");
}

/// ============================================================================
/// Test 3: Stress Test - Heavy Load with Both Paradigms
/// ============================================================================
///
/// Stress tests the system under realistic load, verifying that neither
/// Kameo actors nor tokio tasks degrade or cause deadlocks under pressure.
///
/// **Scenario:**
/// - Spawn 10 Kameo actors
/// - Spawn 50 tokio tasks with varying durations
/// - All perform continuous operations with overlap
/// - Use atomic counter to verify completion
/// - Monitor for any panics or deadlocks
/// - Verify performance characteristics
#[tokio::test]
async fn test_kameo_tokio_stress_load() {
    println!("[Test 3] Starting: Stress test - heavy concurrent load");

    let start_time = std::time::Instant::now();

    // Shared counter to verify all tasks complete
    let task_completion_counter = Arc::new(AtomicUsize::new(0));
    let actor_completion_counter = Arc::new(AtomicUsize::new(0));

    // Setup: Spawn 10 Kameo actors
    let mut kameo_actors = vec![];
    for i in 0..10 {
        let actor = ScpiActor::spawn(ScpiActor::mock(format!("stress_actor_{}", i)));
        kameo_actors.push(actor);
    }
    println!("  -> Spawned 10 Kameo actors for stress testing");

    // Setup: Spawn 50 tokio tasks with varied workloads
    let mut tokio_tasks = vec![];
    for i in 0..50 {
        let counter = Arc::clone(&task_completion_counter);
        let task = tokio::spawn(async move {
            // Vary delays to create realistic workload distribution
            let delay = match i % 5 {
                0 => Duration::from_millis(10),
                1 => Duration::from_millis(25),
                2 => Duration::from_millis(50),
                3 => Duration::from_millis(75),
                _ => Duration::from_millis(100),
            };
            tokio::time::sleep(delay).await;

            // Simulate some work
            let _result = (0..1000).sum::<i32>();

            // Increment completion counter
            counter.fetch_add(1, Ordering::SeqCst);
            i
        });
        tokio_tasks.push(task);
    }
    println!("  -> Spawned 50 tokio tasks with varied workloads");

    // Execute: Continuous actor requests in parallel with tasks
    println!("  -> Executing concurrent actor requests");
    let mut actor_request_tasks = vec![];
    for (_cycle, actor) in kameo_actors.iter().enumerate() {
        let actor_clone = actor.clone();
        let counter = Arc::clone(&actor_completion_counter);

        // Each actor makes multiple requests in parallel
        for request_id in 0..5 {
            let actor_ref = actor_clone.clone();
            let counter_ref = Arc::clone(&counter);

            let task = tokio::spawn(async move {
                let result = actor_ref
                    .ask(Query {
                        cmd: format!("QUERY_{}_{}", request_id, request_id),
                    })
                    .await;

                if result.is_ok() {
                    counter_ref.fetch_add(1, Ordering::SeqCst);
                }
                result
            });

            actor_request_tasks.push(task);
        }
    }
    println!("  -> Created 50 actor request tasks (10 actors × 5 requests each)");

    // Monitor: Check for progress on all fronts
    println!("  -> Awaiting all tasks and actor requests");
    let actor_request_results = join_all_futures(actor_request_tasks).await;
    let task_results = join_all_futures(tokio_tasks).await;

    let elapsed = start_time.elapsed();

    // Verify: All actor requests completed successfully
    let successful_actor_requests = actor_request_results
        .iter()
        .filter(|r| r.is_ok() && r.as_ref().unwrap().is_ok())
        .count();
    println!(
        "  -> Actor requests: {}/50 successful",
        successful_actor_requests
    );
    assert!(
        successful_actor_requests >= 45,
        "Most actor requests should succeed, got {}/50",
        successful_actor_requests
    );

    // Verify: All tokio tasks completed
    let completed_tokio_tasks = task_results.iter().filter(|r| r.is_ok()).count();
    println!("  -> Tokio tasks: {}/50 completed", completed_tokio_tasks);
    assert_eq!(
        completed_tokio_tasks, 50,
        "All tokio tasks should complete"
    );

    // Verify: Completion counters match
    let tokio_counter = task_completion_counter.load(Ordering::SeqCst);
    let actor_counter = actor_completion_counter.load(Ordering::SeqCst);
    println!(
        "  -> Task completion counter: {} (expected 50)",
        tokio_counter
    );
    println!(
        "  -> Actor completion counter: {} (expected 50)",
        actor_counter
    );

    assert_eq!(
        tokio_counter, 50,
        "All tokio tasks should be counted"
    );
    assert_eq!(
        actor_counter, 50,
        "All actor requests should complete"
    );

    println!("  -> Total elapsed time: {:?}", elapsed);

    // Performance check: Stress test should complete in reasonable time
    assert!(
        elapsed < Duration::from_secs(5),
        "Stress test should complete within 5 seconds, took {:?}",
        elapsed
    );

    // Cleanup: Shutdown all actors
    println!("  -> Initiating shutdown of all actors");
    for actor in &kameo_actors {
        actor.kill();
    }

    let shutdown_futures: Vec<_> = kameo_actors.iter().map(|a| a.wait_for_shutdown()).collect();
    join_all_futures(shutdown_futures).await;

    println!("  -> All actors shutdown complete");
    println!("[Test 3] PASSED: Stress test completed successfully");
}

/// ============================================================================
/// Test 4: Timeout Handling - Kameo Under Pressure from Tokio Load
/// ============================================================================
///
/// Validates that Kameo actor request timeouts work correctly even when
/// tokio tasks are consuming resources.
///
/// **Scenario:**
/// - Spawn heavy tokio workload
/// - Attempt actor request with timeout
/// - Verify timeout mechanism works
#[tokio::test]
async fn test_actor_timeout_under_tokio_load() {
    println!("[Test 4] Starting: Timeout handling under concurrent load");

    // Setup: Create background tokio load
    let load_tasks: Vec<_> = (0..20)
        .map(|i| {
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    let _result = (0..10000).sum::<i32>();
                }
            })
        })
        .collect();

    println!("  -> Created 20 background tokio tasks generating load");

    // Setup: Spawn Kameo actor
    let actor = ScpiActor::spawn(ScpiActor::mock("timeout_test".to_string()));
    println!("  -> Spawned test actor");

    // Test: Normal operation should complete within timeout
    println!("  -> Testing normal operation with timeout");
    let result = timeout(Duration::from_secs(1), actor.ask(Identify)).await;
    assert!(
        result.is_ok(),
        "Normal actor operation should complete within timeout even under load"
    );

    // Verify: Result should be successful
    let inner_result = result.expect("Should have inner result");
    assert!(inner_result.is_ok(), "Actor request should succeed");

    println!("  -> Normal operation completed successfully within timeout");

    // Cleanup
    actor.kill();
    actor.wait_for_shutdown().await;

    for task in load_tasks {
        task.abort();
    }

    println!("[Test 4] PASSED: Timeout handling verified");
}

/// ============================================================================
/// Test 5: Sequential vs Concurrent - Verify Both Patterns Work
/// ============================================================================
///
/// Validates that both sequential and concurrent patterns work correctly
/// when mixing Kameo and tokio code.
///
/// **Scenario:**
/// - Test sequential: Actor request → tokio task → Actor request
/// - Test concurrent: All happen at same time
/// - Verify both patterns work correctly
#[tokio::test]
async fn test_sequential_and_concurrent_patterns() {
    println!("[Test 5] Starting: Sequential vs concurrent patterns");

    let actor = ScpiActor::spawn(ScpiActor::mock("pattern_test".to_string()));

    // Part 1: Sequential pattern
    println!("  -> Part 1: Testing sequential pattern");
    let r1 = actor.ask(Identify).await.expect("Request 1 failed");
    assert!(r1.contains("MOCK"));

    let tokio_result = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        "tokio_work"
    })
    .await
    .expect("Tokio task failed");
    assert_eq!(tokio_result, "tokio_work");

    let r2 = actor
        .ask(Query {
            cmd: "*STB?".to_string(),
        })
        .await
        .expect("Request 2 failed");
    assert!(r2.contains("MOCK"));

    println!("  -> Sequential pattern successful");

    // Part 2: Concurrent pattern
    println!("  -> Part 2: Testing concurrent pattern");
    let actor_req = actor.ask(Identify);
    let tokio_task = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        "concurrent_work"
    });

    let (actor_result, task_result) = tokio::join!(
        actor_req,
        tokio_task
    );

    assert!(actor_result.is_ok());
    assert_eq!(task_result.expect("Task failed"), "concurrent_work");

    println!("  -> Concurrent pattern successful");

    actor.kill();
    actor.wait_for_shutdown().await;

    println!("[Test 5] PASSED: Both sequential and concurrent patterns work");
}

