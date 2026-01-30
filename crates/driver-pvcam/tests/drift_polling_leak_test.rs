//! Reproduction test for PVCAM drift polling resource leak (bd-qtd4)
//!
//! This test verifies that the drift polling background task properly exits when
//! the driver is dropped, allowing the SDK to be uninitialized.
//!
//! ## The Bug
//!
//! The drift polling task (lines 652-701 in lib.rs) captures a strong Arc to the
//! PvcamConnection, creating a reference cycle:
//!
//! 1. Task spawned with `connection.clone()` (strong Arc)
//! 2. Task runs in infinite loop with no exit condition
//! 3. Strong Arc prevents PvcamConnection from dropping
//! 4. SDK never uninitialized (pl_pvcam_uninit not called)
//!
//! ## Expected Behavior
//!
//! **Before fix (this test should FAIL):**
//! - SDK ref count stays at 1 after driver dropped
//! - Multiple driver creations increment ref count without decrement
//! - Second driver creation may fail or exhibit undefined behavior
//!
//! **After fix (this test should PASS):**
//! - SDK ref count returns to 0 after driver dropped
//! - Multiple driver creations work correctly
//! - Clean shutdown with proper SDK uninitialization
//!
//! ## Running
//!
//! ```bash
//! # Mock mode (safe to run locally)
//! cargo test -p driver-pvcam --test drift_polling_leak_test
//!
//! # With hardware (on maitai)
//! cargo test -p driver-pvcam --test drift_polling_leak_test --features pvcam_hardware
//! ```

use driver_pvcam::PvcamDriver;
use std::time::Duration;

/// Test that drift polling task exits when driver is dropped.
///
/// This test creates and drops a driver multiple times. Without the fix,
/// the drift polling task holds a strong Arc preventing SDK uninit.
#[tokio::test]
async fn test_drift_polling_exits_on_drop() {
    // Initialize tracing for debugging
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("debug")
        .try_init();

    #[cfg(feature = "pvcam_sdk")]
    {
        // Get initial SDK ref count (should be 0)
        let initial_count = driver_pvcam::components::connection::sdk_ref_count();
        tracing::info!("Initial SDK ref count: {}", initial_count);

        // Create and drop driver multiple times
        for iteration in 1..=3 {
            tracing::info!("=== Iteration {} ===", iteration);

            // Create driver
            let driver = PvcamDriver::new_async("pvcamUSB_0".to_string())
                .await
                .expect("Should create driver");

            let count_after_create = driver_pvcam::components::connection::sdk_ref_count();
            tracing::info!("SDK ref count after create: {}", count_after_create);
            assert_eq!(
                count_after_create, 1,
                "SDK ref count should be 1 after driver creation"
            );

            // Give drift polling task time to start
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Drop the driver
            drop(driver);
            tracing::info!("Driver dropped");

            // Give drift polling task time to detect drop and exit
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Check ref count after drop
            let count_after_drop = driver_pvcam::components::connection::sdk_ref_count();
            tracing::info!("SDK ref count after drop: {}", count_after_drop);

            // CRITICAL: This assertion will FAIL before the fix
            // The drift polling task holds a strong Arc, preventing ref count from reaching 0
            assert_eq!(
                count_after_drop, 0,
                "SDK ref count should return to 0 after driver drop (iteration {}). \
                 If this fails, the drift polling task is leaking a reference.",
                iteration
            );
        }
    }

    #[cfg(not(feature = "pvcam_sdk"))]
    {
        // In mock mode, we can still test that the driver creates/drops cleanly
        // but we can't verify SDK ref counting
        tracing::info!("Running in mock mode - testing driver lifecycle only");

        for iteration in 1..=3 {
            tracing::info!("=== Iteration {} (mock) ===", iteration);

            let driver = PvcamDriver::new_async("MockCamera".to_string())
                .await
                .expect("Should create driver");

            tokio::time::sleep(Duration::from_millis(100)).await;

            drop(driver);
            tracing::info!("Driver dropped (mock mode)");

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        tracing::info!("Mock mode test passed - driver lifecycle works");
    }
}

/// Test that drift polling task doesn't prevent rapid driver recreation.
///
/// This is a stress test that rapidly creates and drops drivers.
/// Without the fix, this will fail or hang.
#[tokio::test]
async fn test_rapid_driver_recreation() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("info")
        .try_init();

    #[cfg(feature = "pvcam_sdk")]
    {
        tracing::info!("Testing rapid driver recreation (10 iterations)");

        for i in 1..=10 {
            let driver = PvcamDriver::new_async("pvcamUSB_0".to_string())
                .await
                .expect("Should create driver");

            // Minimal delay
            tokio::time::sleep(Duration::from_millis(10)).await;

            drop(driver);

            // Give task time to exit
            tokio::time::sleep(Duration::from_millis(100)).await;

            if i % 3 == 0 {
                tracing::info!("Completed {} rapid recreations", i);
            }
        }

        // Final ref count check
        let final_count = driver_pvcam::components::connection::sdk_ref_count();
        assert_eq!(
            final_count, 0,
            "SDK ref count should be 0 after all rapid recreations"
        );

        tracing::info!("Rapid recreation test passed");
    }

    #[cfg(not(feature = "pvcam_sdk"))]
    {
        tracing::info!("Testing rapid driver recreation (mock mode)");

        for i in 1..=10 {
            let driver = PvcamDriver::new_async("MockCamera".to_string())
                .await
                .expect("Should create driver");

            tokio::time::sleep(Duration::from_millis(10)).await;
            drop(driver);
            tokio::time::sleep(Duration::from_millis(100)).await;

            if i % 3 == 0 {
                tracing::info!("Completed {} rapid recreations (mock)", i);
            }
        }

        tracing::info!("Mock mode rapid recreation test passed");
    }
}
