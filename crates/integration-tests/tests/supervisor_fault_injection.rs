#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::new_without_default,
    clippy::must_use_candidate,
    clippy::panic,
    deprecated,
    unsafe_code,
    unused_mut,
    unused_imports,
    missing_docs
)]
//! Fault injection tests for DeviceSupervisor recovery paths (bd-jwpu).
//!
//! These tests verify that the DeviceSupervisor correctly detects faulted
//! devices and attempts restart with exponential backoff. Uses custom
//! DriverFactory implementations that inject failures at various points:
//!
//! - **SDK init failure**: Factory `build()` returns an error
//! - **TCP timeout/disconnect**: Device faults after registration, supervisor restarts
//! - **Hard crash**: Factory `build()` panics (caught by registry's spawn)
//! - **Reconnection recovery**: Supervisor detects fault, restarts, device recovers
//! - **Max restart attempts**: Supervisor gives up after configured limit
//! - **Buffer overrun**: Device operation produces more data than expected
//!
//! Run with: cargo nextest run -p integration-tests --test supervisor_fault_injection

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::Readable;
use common::driver::{Capability, DeviceComponents, DriverFactory};
use common::health::DeviceHealth;
use futures::future::BoxFuture;
use hardware::registry::DeviceRegistry;
use hardware::supervisor::{SupervisorConfig, run_device_supervisor};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// =============================================================================
// Test Factories: Custom DriverFactory implementations for fault injection
// =============================================================================

/// A factory that always fails to build (simulates SDK init failure).
struct AlwaysFailFactory;

impl DriverFactory for AlwaysFailFactory {
    fn driver_type(&self) -> &'static str {
        "always_fail"
    }
    fn name(&self) -> &'static str {
        "Always Fail Factory"
    }
    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Readable]
    }
    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }
    fn build(&self, _config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async {
            Err(anyhow::anyhow!(
                "SDK initialization failed: device not found"
            ))
        })
    }
}

/// A factory that panics during build (simulates hard crash).
struct PanicFactory;

impl DriverFactory for PanicFactory {
    fn driver_type(&self) -> &'static str {
        "panic_factory"
    }
    fn name(&self) -> &'static str {
        "Panic Factory"
    }
    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Readable]
    }
    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }
    fn build(&self, _config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async { panic!("Segfault in native SDK during initialization") })
    }
}

/// A factory that fails N times, then succeeds (simulates transient connection failure).
struct TransientFailFactory {
    fail_count: Arc<AtomicU32>,
    max_failures: u32,
}

impl TransientFailFactory {
    fn new(max_failures: u32) -> Self {
        Self {
            fail_count: Arc::new(AtomicU32::new(0)),
            max_failures,
        }
    }
}

impl DriverFactory for TransientFailFactory {
    fn driver_type(&self) -> &'static str {
        "transient_fail"
    }
    fn name(&self) -> &'static str {
        "Transient Fail Factory"
    }
    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Readable]
    }
    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }
    fn build(&self, _config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        let fail_count = self.fail_count.clone();
        let max_failures = self.max_failures;
        Box::pin(async move {
            let current = fail_count.fetch_add(1, Ordering::SeqCst);
            if current < max_failures {
                Err(anyhow::anyhow!(
                    "TCP connection refused (attempt {})",
                    current + 1
                ))
            } else {
                // Success: return a minimal readable device
                let reader = Arc::new(SimpleReader);
                Ok(DeviceComponents {
                    readable: Some(reader),
                    ..Default::default()
                })
            }
        })
    }
}

/// A factory that succeeds on every build and tracks build invocations.
/// Used to verify the supervisor triggers restart after fault reporting.
struct RebuildTrackingFactory {
    build_count: Arc<AtomicU32>,
}

impl RebuildTrackingFactory {
    fn new() -> Self {
        Self {
            build_count: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl DriverFactory for RebuildTrackingFactory {
    fn driver_type(&self) -> &'static str {
        "rebuild_tracking"
    }
    fn name(&self) -> &'static str {
        "Rebuild Tracking Factory"
    }
    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Readable]
    }
    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }
    fn build(&self, _config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        let build_count = self.build_count.clone();
        Box::pin(async move {
            build_count.fetch_add(1, Ordering::SeqCst);
            let reader = Arc::new(SimpleReader);
            Ok(DeviceComponents {
                readable: Some(reader),
                ..Default::default()
            })
        })
    }
}

/// A factory that succeeds on first build but produces a device that
/// generates oversized frames (buffer overrun simulation).
struct BufferOverrunFactory;

impl DriverFactory for BufferOverrunFactory {
    fn driver_type(&self) -> &'static str {
        "buffer_overrun"
    }
    fn name(&self) -> &'static str {
        "Buffer Overrun Factory"
    }
    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Readable]
    }
    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }
    fn build(&self, _config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async {
            let reader = Arc::new(OverrunReader);
            Ok(DeviceComponents {
                readable: Some(reader),
                ..Default::default()
            })
        })
    }
}

// =============================================================================
// Mock Readable implementations for test factories
// =============================================================================

/// A simple reader that returns a constant value.
struct SimpleReader;

#[async_trait]
impl Readable for SimpleReader {
    async fn read(&self) -> Result<f64> {
        Ok(42.0)
    }
}

/// A reader that simulates a buffer overrun by returning unexpectedly large values
/// and then erroring (simulating a device that produces more data than expected).
struct OverrunReader;

#[async_trait]
impl Readable for OverrunReader {
    async fn read(&self) -> Result<f64> {
        // Simulate: device returned more data than the protocol expected,
        // causing a parse/framing error
        Err(anyhow::anyhow!(
            "Buffer overrun: received 8192 bytes, expected 4096"
        ))
    }
}

// =============================================================================
// Helper: wait for a condition with timeout (polling)
// =============================================================================

/// Poll a condition with a deadline. Returns true if the condition was met.
async fn wait_for(timeout: Duration, interval: Duration, condition: impl Fn() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(interval).await;
    }
}

// =============================================================================
// Test: SDK init failure -- factory build() returns error
// =============================================================================

/// When a factory's build() returns an error, the device should not be
/// registered and the error should be propagated cleanly.
#[tokio::test]
async fn test_sdk_init_failure_prevents_registration() {
    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(AlwaysFailFactory));

    let result = registry
        .register_from_toml(
            "broken_device",
            "Broken Device",
            "always_fail",
            toml::Value::Table(Default::default()),
        )
        .await;

    assert!(
        result.is_err(),
        "Registration should fail when build() errors"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("SDK initialization failed"),
        "Error should contain factory error message, got: {err_msg}"
    );

    // Device should not be in the registry
    assert!(
        !registry.contains("broken_device"),
        "Failed device should not be registered"
    );
    assert_eq!(
        registry.len(),
        0,
        "Registry should be empty after failed registration"
    );
}

// =============================================================================
// Test: Hard crash -- factory build() panics
// =============================================================================

/// When a factory's build() panics, the registry should catch it via
/// tokio::task::spawn and convert it to an error (not crash the caller).
#[tokio::test]
async fn test_factory_panic_caught_as_error() {
    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(PanicFactory));

    let result = registry
        .register_from_toml(
            "panic_device",
            "Panic Device",
            "panic_factory",
            toml::Value::Table(Default::default()),
        )
        .await;

    assert!(
        result.is_err(),
        "Registration should fail when build() panics"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("panicked"),
        "Error should indicate a panic, got: {err_msg}"
    );

    // Device should not be in the registry
    assert!(
        !registry.contains("panic_device"),
        "Panicked device should not be registered"
    );
}

// =============================================================================
// Test: Supervisor detects faulted device and attempts restart
// =============================================================================

/// After a device is registered and then manually faulted (simulating a
/// TCP disconnect), the supervisor should detect it and attempt restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_supervisor_restarts_faulted_device() {
    let registry = Arc::new(DeviceRegistry::new());
    let factory = RebuildTrackingFactory::new();
    let build_count = factory.build_count.clone();
    registry.register_factory(Box::new(factory));

    // Register device successfully (first build)
    registry
        .register_from_toml(
            "tcp_device",
            "TCP Device",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Initial registration should succeed");

    assert_eq!(
        build_count.load(Ordering::SeqCst),
        1,
        "Should have built once"
    );
    assert!(registry.contains("tcp_device"));

    // Simulate TCP disconnect: report failures until device is faulted
    let fault_threshold = registry.fault_threshold();
    for i in 0..fault_threshold {
        registry.report_device_failure("tcp_device", format!("TCP timeout (attempt {})", i + 1));
    }

    // Verify device is faulted
    let health = registry
        .get_device_health("tcp_device")
        .expect("Health state should exist");
    assert_eq!(
        health.health,
        DeviceHealth::Faulted,
        "Device should be faulted after {} failures",
        fault_threshold
    );

    // Start supervisor with fast check interval
    let cancel = CancellationToken::new();
    let config = SupervisorConfig {
        check_interval: Duration::from_millis(50),
        base_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_secs(1),
        max_restart_attempts: 5,
        fault_threshold,
    };

    let registry_clone = registry.clone();
    let cancel_clone = cancel.clone();
    let supervisor = tokio::spawn(async move {
        run_device_supervisor(registry_clone, config, cancel_clone).await;
    });

    // Wait for supervisor to attempt a restart
    let bc = build_count.clone();
    let recovered = wait_for(
        Duration::from_secs(5),
        Duration::from_millis(50),
        move || bc.load(Ordering::SeqCst) > 1,
    )
    .await;
    assert!(recovered, "Supervisor should have attempted rebuild");

    // Device should be healthy again after successful restart
    let reg = registry.clone();
    let healthy = wait_for(
        Duration::from_secs(2),
        Duration::from_millis(50),
        move || {
            reg.get_device_health("tcp_device")
                .is_some_and(|h| h.health == DeviceHealth::Healthy)
        },
    )
    .await;
    assert!(healthy, "Device should be healthy after successful restart");

    cancel.cancel();
    supervisor.await.expect("Supervisor should exit cleanly");
}

// =============================================================================
// Test: Supervisor respects max restart attempts
// =============================================================================

/// When a device fails repeatedly and exceeds max_restart_attempts,
/// the supervisor should stop trying to restart it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_supervisor_respects_max_restart_attempts() {
    let registry = Arc::new(DeviceRegistry::new());

    // Register with transient_fail factory: first build succeeds (max_failures=0)
    let once_factory = TransientFailFactory::new(0);
    registry.register_factory(Box::new(once_factory));

    registry
        .register_from_toml(
            "doomed_device",
            "Doomed Device",
            "transient_fail",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("First registration should succeed");

    // Replace factory so all future builds always fail
    let always_fail = TransientFailFactory::new(u32::MAX);
    registry.register_factory(Box::new(always_fail));

    // Fault the device
    registry.set_fault_threshold(1);
    registry.report_device_failure("doomed_device", "permanent hardware fault");

    let health = registry
        .get_device_health("doomed_device")
        .expect("should have health");
    assert_eq!(health.health, DeviceHealth::Faulted);

    // Start supervisor with max_restart_attempts = 3
    let cancel = CancellationToken::new();
    let config = SupervisorConfig {
        check_interval: Duration::from_millis(50),
        base_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(100),
        max_restart_attempts: 3,
        fault_threshold: 1,
    };

    let reg = registry.clone();
    let cancel_clone = cancel.clone();
    let supervisor = tokio::spawn(async move {
        run_device_supervisor(reg, config, cancel_clone).await;
    });

    // Wait for at least 3 restart attempts
    let registry_poll = registry.clone();
    let reached_limit = wait_for(
        Duration::from_secs(10),
        Duration::from_millis(100),
        move || {
            registry_poll
                .get_device_health("doomed_device")
                .is_some_and(|h| h.restart_attempts >= 3)
        },
    )
    .await;

    assert!(
        reached_limit,
        "Should have attempted at least 3 restarts, got {}",
        registry
            .get_device_health("doomed_device")
            .map_or(0, |h| h.restart_attempts)
    );

    let health = registry
        .get_device_health("doomed_device")
        .expect("should have health");
    assert_eq!(
        health.health,
        DeviceHealth::Faulted,
        "Device should remain faulted after all restart attempts fail"
    );

    cancel.cancel();
    supervisor.await.expect("Supervisor should exit cleanly");
}

// =============================================================================
// Test: Transient failure recovery -- device recovers after N failures
// =============================================================================

/// A device that fails transiently (e.g., TCP connection refused) should
/// eventually be restarted successfully by the supervisor when the
/// underlying issue resolves.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_transient_failure_recovery() {
    let registry = Arc::new(DeviceRegistry::new());

    // First build succeeds (max_failures=0)
    let factory = TransientFailFactory::new(0);
    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "flaky_device",
            "Flaky Device",
            "transient_fail",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Initial registration should succeed");

    // Replace factory: fail 2 times, then succeed on third rebuild
    let flaky_factory = TransientFailFactory::new(2);
    registry.register_factory(Box::new(flaky_factory));

    // Fault the device
    registry.set_fault_threshold(1);
    registry.report_device_failure("flaky_device", "TCP connection reset by peer");

    assert_eq!(
        registry.get_device_health("flaky_device").unwrap().health,
        DeviceHealth::Faulted
    );

    // Start supervisor
    let cancel = CancellationToken::new();
    let config = SupervisorConfig {
        check_interval: Duration::from_millis(50),
        base_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_secs(1),
        max_restart_attempts: 10,
        fault_threshold: 1,
    };

    let reg = registry.clone();
    let cancel_clone = cancel.clone();
    let supervisor = tokio::spawn(async move {
        run_device_supervisor(reg, config, cancel_clone).await;
    });

    // Wait for device to recover
    let registry_poll = registry.clone();
    let recovered = wait_for(
        Duration::from_secs(10),
        Duration::from_millis(100),
        move || {
            registry_poll
                .get_device_health("flaky_device")
                .is_some_and(|h| h.health == DeviceHealth::Healthy)
        },
    )
    .await;

    assert!(
        recovered,
        "Device should recover after transient failures, but health = {:?}",
        registry.get_device_health("flaky_device").map(|h| h.health)
    );

    cancel.cancel();
    supervisor.await.expect("Supervisor should exit cleanly");
}

// =============================================================================
// Test: Health event broadcasting during fault/recovery cycle
// =============================================================================

/// The registry should broadcast health events as a device transitions
/// through Healthy -> Degraded -> Faulted -> Recovering -> Healthy.
#[tokio::test]
async fn test_health_event_broadcast_during_fault_cycle() {
    let registry = DeviceRegistry::new();
    let factory = RebuildTrackingFactory::new();
    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "broadcast_device",
            "Broadcast Device",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Registration should succeed");

    // Subscribe to health events
    let mut rx = registry.subscribe_health_changes();

    // Report failures to trigger state transitions
    registry.set_fault_threshold(2);

    // First failure: Healthy -> Degraded
    registry.report_device_failure("broadcast_device", "minor glitch");
    let event1 = rx.try_recv().expect("Should receive Degraded event");
    assert_eq!(event1.device_id, "broadcast_device");
    assert_eq!(event1.old_state, DeviceHealth::Healthy);
    assert_eq!(event1.new_state, DeviceHealth::Degraded);

    // Second failure: Degraded -> Faulted
    registry.report_device_failure("broadcast_device", "connection lost");
    let event2 = rx.try_recv().expect("Should receive Faulted event");
    assert_eq!(event2.old_state, DeviceHealth::Degraded);
    assert_eq!(event2.new_state, DeviceHealth::Faulted);

    // Restart: Faulted -> Recovering -> Healthy
    let result = registry.restart_device("broadcast_device").await;
    assert!(result.is_ok(), "Restart should succeed");

    let event3 = rx.try_recv().expect("Should receive Recovering event");
    assert_eq!(event3.old_state, DeviceHealth::Faulted);
    assert_eq!(event3.new_state, DeviceHealth::Recovering);

    let event4 = rx.try_recv().expect("Should receive Healthy event");
    assert_eq!(event4.old_state, DeviceHealth::Recovering);
    assert_eq!(event4.new_state, DeviceHealth::Healthy);
}

// =============================================================================
// Test: Buffer overrun -- device produces unexpected data
// =============================================================================

/// A device that produces a buffer overrun error on read should transition
/// to Faulted through the normal health reporting path, and the supervisor
/// should attempt restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_buffer_overrun_fault_and_recovery() {
    let registry = Arc::new(DeviceRegistry::new());
    registry.register_factory(Box::new(BufferOverrunFactory));

    registry
        .register_from_toml(
            "overrun_device",
            "Overrun Device",
            "buffer_overrun",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Registration should succeed");

    // Get the device's readable interface and try to read
    let readable = registry
        .get_readable("overrun_device")
        .expect("Device should be readable");

    let result = readable.read().await;
    assert!(result.is_err(), "Read should fail with buffer overrun");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Buffer overrun"),
        "Error should indicate buffer overrun, got: {err_msg}"
    );

    // Report the failure to the registry (as the server layer would)
    registry.set_fault_threshold(1);
    registry.report_device_failure("overrun_device", &err_msg);

    let health = registry
        .get_device_health("overrun_device")
        .expect("should have health");
    assert_eq!(health.health, DeviceHealth::Faulted);

    // The buffer_overrun factory always builds successfully (the read is what
    // fails, not the build), so the supervisor restart should succeed.

    // Start supervisor to trigger restart
    let cancel = CancellationToken::new();
    let config = SupervisorConfig {
        check_interval: Duration::from_millis(50),
        base_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_secs(1),
        max_restart_attempts: 3,
        fault_threshold: 1,
    };

    let reg = registry.clone();
    let cancel_clone = cancel.clone();
    let supervisor = tokio::spawn(async move {
        run_device_supervisor(reg, config, cancel_clone).await;
    });

    // Wait for device to become healthy after restart
    let registry_poll = registry.clone();
    let recovered = wait_for(
        Duration::from_secs(5),
        Duration::from_millis(50),
        move || {
            registry_poll
                .get_device_health("overrun_device")
                .is_some_and(|h| h.health == DeviceHealth::Healthy)
        },
    )
    .await;

    assert!(
        recovered,
        "Device should be healthy after restart (build succeeds)"
    );

    cancel.cancel();
    supervisor.await.expect("Supervisor should exit cleanly");
}

// =============================================================================
// Test: Supervisor exponential backoff timing
// =============================================================================

/// Verify that the supervisor respects exponential backoff between restart
/// attempts for a persistently faulted device.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_supervisor_exponential_backoff() {
    let registry = Arc::new(DeviceRegistry::new());

    // First build succeeds
    let factory = TransientFailFactory::new(0);
    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "backoff_device",
            "Backoff Device",
            "transient_fail",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Initial registration should succeed");

    // Replace factory with one that always fails
    let always_fail = TransientFailFactory::new(u32::MAX);
    registry.register_factory(Box::new(always_fail));

    // Fault the device
    registry.set_fault_threshold(1);
    registry.report_device_failure("backoff_device", "permanent failure");

    let cancel = CancellationToken::new();
    let config = SupervisorConfig {
        check_interval: Duration::from_millis(50),
        base_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_secs(5),
        max_restart_attempts: 0, // unlimited
        fault_threshold: 1,
    };

    let reg = registry.clone();
    let cancel_clone = cancel.clone();
    let supervisor = tokio::spawn(async move {
        run_device_supervisor(reg, config, cancel_clone).await;
    });

    // Wait for at least 1 restart attempt
    let registry_poll = registry.clone();
    let got_attempt = wait_for(
        Duration::from_secs(5),
        Duration::from_millis(50),
        move || {
            registry_poll
                .get_device_health("backoff_device")
                .is_some_and(|h| h.restart_attempts >= 1)
        },
    )
    .await;

    assert!(
        got_attempt,
        "Should have at least 1 restart attempt, got {}",
        registry
            .get_device_health("backoff_device")
            .map_or(0, |h| h.restart_attempts)
    );

    let attempts_early = registry
        .get_device_health("backoff_device")
        .map_or(0, |h| h.restart_attempts);

    // Wait more time for additional attempts
    tokio::time::sleep(Duration::from_secs(3)).await;

    let attempts_later = registry
        .get_device_health("backoff_device")
        .map_or(0, |h| h.restart_attempts);

    assert!(
        attempts_later > attempts_early,
        "Should have more restart attempts after waiting ({} vs {})",
        attempts_later,
        attempts_early
    );

    // With 100ms base backoff and exponential growth (100, 200, 400, 800, 1600, 3200...),
    // the total time for N attempts grows quickly. After ~8s total, we should
    // have a bounded number of attempts.
    assert!(
        attempts_later <= 15,
        "Exponential backoff should limit attempts, got {}",
        attempts_later
    );

    cancel.cancel();
    supervisor.await.expect("Supervisor should exit cleanly");
}

// =============================================================================
// Test: Multiple devices -- one faults, others unaffected
// =============================================================================

/// When one device in a multi-device registry faults, the supervisor
/// should only restart that device. Healthy devices remain untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_fault_isolation_multiple_devices() {
    let registry = Arc::new(DeviceRegistry::new());

    let factory = RebuildTrackingFactory::new();
    let build_count = factory.build_count.clone();

    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "device_a",
            "Device A",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Device A registration should succeed");

    registry
        .register_from_toml(
            "device_b",
            "Device B",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Device B registration should succeed");

    // Both devices built once (same factory, count = 2)
    assert_eq!(
        build_count.load(Ordering::SeqCst),
        2,
        "Both devices should have built"
    );

    // Only fault device_a
    registry.set_fault_threshold(1);
    registry.report_device_failure("device_a", "device_a connection lost");

    let health_a = registry.get_device_health("device_a").unwrap();
    assert_eq!(health_a.health, DeviceHealth::Faulted);

    let health_b = registry.get_device_health("device_b").unwrap();
    assert_eq!(health_b.health, DeviceHealth::Healthy);

    // Start supervisor
    let cancel = CancellationToken::new();
    let config = SupervisorConfig {
        check_interval: Duration::from_millis(50),
        base_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_secs(1),
        max_restart_attempts: 5,
        fault_threshold: 1,
    };

    let reg = registry.clone();
    let cancel_clone = cancel.clone();
    let supervisor = tokio::spawn(async move {
        run_device_supervisor(reg, config, cancel_clone).await;
    });

    // Wait for device_a to be rebuilt
    let bc = build_count.clone();
    let rebuilt = wait_for(
        Duration::from_secs(5),
        Duration::from_millis(50),
        move || bc.load(Ordering::SeqCst) > 2,
    )
    .await;

    assert!(
        rebuilt,
        "Factory should have rebuilt device_a, build_count = {}",
        build_count.load(Ordering::SeqCst)
    );

    // Device B should still be healthy
    let health_b = registry.get_device_health("device_b").unwrap();
    assert_eq!(
        health_b.health,
        DeviceHealth::Healthy,
        "Device B should remain healthy"
    );
    assert_eq!(
        health_b.restart_attempts, 0,
        "Device B should have zero restart attempts"
    );

    cancel.cancel();
    supervisor.await.expect("Supervisor should exit cleanly");
}

// =============================================================================
// Test: Restart preserves restart_attempts counter across cycles
// =============================================================================

/// After a successful restart, the restart_attempts counter should be
/// preserved so the supervisor can track cumulative restart history.
#[tokio::test]
async fn test_restart_preserves_attempt_counter() {
    let registry = Arc::new(DeviceRegistry::new());
    let factory = RebuildTrackingFactory::new();
    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "counter_device",
            "Counter Device",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Registration should succeed");

    // Fault and restart manually
    registry.set_fault_threshold(1);
    registry.report_device_failure("counter_device", "first fault");
    assert_eq!(
        registry.get_device_health("counter_device").unwrap().health,
        DeviceHealth::Faulted
    );

    let result = registry.restart_device("counter_device").await;
    assert!(result.is_ok());
    assert!(result.unwrap());

    let health = registry.get_device_health("counter_device").unwrap();
    assert_eq!(health.health, DeviceHealth::Healthy);
    assert_eq!(
        health.restart_attempts, 1,
        "Should have 1 restart attempt after first recovery"
    );

    // Fault and restart again
    registry.report_device_failure("counter_device", "second fault");
    let _ = registry.restart_device("counter_device").await;

    let health = registry.get_device_health("counter_device").unwrap();
    assert_eq!(
        health.restart_attempts, 2,
        "Should have 2 restart attempts after second recovery"
    );
}

// =============================================================================
// Test: Non-faulted device restart is a no-op
// =============================================================================

/// Calling restart_device on a healthy device should return Ok(false)
/// without modifying the device.
#[tokio::test]
async fn test_restart_healthy_device_is_noop() {
    let registry = DeviceRegistry::new();
    let factory = RebuildTrackingFactory::new();
    let build_count = factory.build_count.clone();
    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "healthy_device",
            "Healthy Device",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Registration should succeed");

    assert_eq!(build_count.load(Ordering::SeqCst), 1);

    let result = registry.restart_device("healthy_device").await;
    assert!(result.is_ok());
    assert!(
        !result.unwrap(),
        "Restarting a healthy device should return false"
    );

    assert_eq!(
        build_count.load(Ordering::SeqCst),
        1,
        "Factory should not have been called again for a healthy device"
    );
}

// =============================================================================
// Test: Restart nonexistent device returns Ok(false)
// =============================================================================

/// Calling restart_device on a device that does not exist should return
/// Ok(false) without errors.
#[tokio::test]
async fn test_restart_nonexistent_device() {
    let registry = DeviceRegistry::new();

    let result = registry.restart_device("ghost_device").await;
    assert!(result.is_ok());
    assert!(
        !result.unwrap(),
        "Restarting a nonexistent device should return false"
    );
}

// =============================================================================
// Test: Critical fault bypasses degradation threshold
// =============================================================================

/// A critical fault (e.g., hardware disconnect) should immediately
/// transition a device to Faulted, bypassing the normal degradation path.
#[tokio::test]
async fn test_critical_fault_immediate_transition() {
    let registry = DeviceRegistry::new();
    let factory = RebuildTrackingFactory::new();
    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "critical_device",
            "Critical Device",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Registration should succeed");

    // Set a high fault threshold -- normal failures would need many attempts
    registry.set_fault_threshold(10);

    // Verify that record_critical_fault on the health state goes straight to Faulted
    if let Some(mut health) = registry.get_device_health("critical_device") {
        health.record_critical_fault("USB device disappeared from /dev");
        assert_eq!(health.health, DeviceHealth::Faulted);
    }

    // Verify via the report_device_failure path: even one failure with threshold=10
    // should leave the device as Degraded, not Faulted
    let registry2 = DeviceRegistry::new();
    let factory2 = RebuildTrackingFactory::new();
    registry2.register_factory(Box::new(factory2));

    registry2
        .register_from_toml(
            "normal_device",
            "Normal Device",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();

    registry2.set_fault_threshold(10);
    registry2.report_device_failure("normal_device", "minor timeout");

    let health = registry2.get_device_health("normal_device").unwrap();
    assert_eq!(
        health.health,
        DeviceHealth::Degraded,
        "Single failure with threshold=10 should only degrade, not fault"
    );
}

// =============================================================================
// Test: Success reporting resets failure counters
// =============================================================================

/// After a device has accumulated failures (Degraded state), a successful
/// operation should reset the failure counter and return to Healthy.
#[tokio::test]
async fn test_success_resets_degraded_to_healthy() {
    let registry = DeviceRegistry::new();
    let factory = RebuildTrackingFactory::new();
    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "recoverable_device",
            "Recoverable Device",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Registration should succeed");

    registry.set_fault_threshold(5);

    // Report 3 failures (below threshold) -- should be Degraded
    for i in 0..3 {
        registry.report_device_failure("recoverable_device", format!("error {}", i + 1));
    }

    let health = registry.get_device_health("recoverable_device").unwrap();
    assert_eq!(health.health, DeviceHealth::Degraded);
    assert_eq!(health.consecutive_failures, 3);

    // Report success -- should reset to Healthy
    registry.report_device_success("recoverable_device");

    let health = registry.get_device_health("recoverable_device").unwrap();
    assert_eq!(health.health, DeviceHealth::Healthy);
    assert_eq!(health.consecutive_failures, 0);
}

// =============================================================================
// Test: Repeated fault/recover cycles with supervisor
// =============================================================================

/// Verify the supervisor can handle multiple fault/recover cycles for
/// the same device without getting stuck or leaking state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_repeated_fault_recover_cycles() {
    let registry = Arc::new(DeviceRegistry::new());
    let factory = RebuildTrackingFactory::new();
    let build_count = factory.build_count.clone();
    registry.register_factory(Box::new(factory));

    registry
        .register_from_toml(
            "cycle_device",
            "Cycle Device",
            "rebuild_tracking",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("Registration should succeed");

    registry.set_fault_threshold(1);

    // Run 3 fault/recover cycles manually (not via supervisor, for determinism)
    for cycle in 1_u32..=3 {
        registry.report_device_failure("cycle_device", format!("fault cycle {cycle}"));
        assert_eq!(
            registry.get_device_health("cycle_device").unwrap().health,
            DeviceHealth::Faulted,
            "Should be faulted in cycle {cycle}"
        );

        let result = registry.restart_device("cycle_device").await;
        assert!(result.is_ok(), "Restart should succeed in cycle {cycle}");
        assert!(
            result.unwrap(),
            "Restart should return true in cycle {cycle}"
        );

        let health = registry.get_device_health("cycle_device").unwrap();
        assert_eq!(
            health.health,
            DeviceHealth::Healthy,
            "Should be healthy after restart in cycle {cycle}"
        );
        assert_eq!(
            health.restart_attempts, cycle,
            "Should have {cycle} restart attempts"
        );
    }

    // Total builds: 1 initial + 3 restarts = 4
    assert_eq!(
        build_count.load(Ordering::SeqCst),
        4,
        "Should have 4 total builds (1 initial + 3 restarts)"
    );
}
