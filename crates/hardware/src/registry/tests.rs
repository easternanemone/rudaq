//! Tests for the device registry.
//!
//! Uses deprecated `register_mock_factories`/`create_mock_registry` because these
//! tests exercise the low-level registry API (factory registration, TOML config
//! loading, duplicate detection, etc.) which requires direct factory control.
//! The canonical mock registry lives in `driver-registry` and can't be used here
//! (circular dep).

#![allow(deprecated)]

use super::*;
use anyhow::{Result, anyhow};

#[tokio::test]
async fn test_register_mock_devices() {
    let registry = create_mock_registry().await.unwrap();

    assert_eq!(registry.len(), 3);
    assert!(registry.contains("mock_stage"));
    assert!(registry.contains("mock_power_meter"));
    assert!(registry.contains("mock_camera"));
}

#[tokio::test]
async fn test_list_devices() {
    let registry = create_mock_registry().await.unwrap();
    let devices = registry.list_devices();

    assert_eq!(devices.len(), 3);

    let stage = devices.iter().find(|d| d.id == "mock_stage").unwrap();
    assert_eq!(stage.driver_type, "mock_stage");
    assert!(stage.capabilities.contains(&Capability::Movable));

    let meter = devices.iter().find(|d| d.id == "mock_power_meter").unwrap();
    assert_eq!(meter.driver_type, "mock_power_meter");
    assert!(meter.capabilities.contains(&Capability::Readable));

    let camera = devices.iter().find(|d| d.id == "mock_camera").unwrap();
    assert_eq!(camera.driver_type, "mock_camera");
    assert!(camera.capabilities.contains(&Capability::FrameProducer));
    assert!(camera.capabilities.contains(&Capability::Triggerable));
    assert!(camera.capabilities.contains(&Capability::ExposureControl));
}

#[tokio::test]
async fn test_legacy_toml_config_registers_mock_devices() {
    let toml_str = r#"
[[devices]]
id = "legacy_stage"
name = "Legacy Stage"
[devices.driver]
type = "mock_stage"
initial_position = 1.23

[[devices]]
id = "legacy_camera"
name = "Legacy Camera"
[devices.driver]
type = "mock_camera"
width = 320
height = 240
"#;

    let config: HardwareConfig = toml::from_str(toml_str).unwrap();
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    populate_registry_from_config(&registry, &config)
        .await
        .unwrap();

    let devices = registry.list_devices();
    assert_eq!(devices.len(), 2);

    let stage = devices.iter().find(|d| d.id == "legacy_stage").unwrap();
    assert_eq!(stage.driver_type, "mock_stage");
    assert!(stage.capabilities.contains(&Capability::Movable));

    let camera = devices.iter().find(|d| d.id == "legacy_camera").unwrap();
    assert_eq!(camera.driver_type, "mock_camera");
    assert!(camera.capabilities.contains(&Capability::FrameProducer));
    assert!(camera.capabilities.contains(&Capability::Triggerable));
    assert!(camera.capabilities.contains(&Capability::ExposureControl));

    assert!(registry.get_movable("legacy_stage").is_some());
    assert!(registry.get_frame_producer("legacy_camera").is_some());
}

#[tokio::test]
async fn test_factory_only_path() {
    // All devices must go through factory registration
    let toml_str = r#"
[[devices]]
id = "test_device"
name = "Test Device With Factory"

[devices.driver]
type = "mock_stage"
initial_position = 0.0
"#;

    let config: HardwareConfig = toml::from_str(toml_str).unwrap();

    // MockStageFactory is registered by register_mock_factories(),
    // so this should succeed
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    let result = populate_registry_from_config(&registry, &config).await;
    assert!(result.is_ok(), "Should succeed when factory exists");
    let devices = registry.list_devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, "test_device");
}

#[tokio::test]
async fn test_factory_path_logging() {
    // Test that logging distinguishes between factory and legacy paths
    // This is a smoke test - actual verification would require log capture
    let toml_str = r#"
[[devices]]
id = "factory_device"
name = "Device Using Factory"

[devices.driver]
type = "mock_stage"
initial_position = 0.0
"#;

    let config: HardwareConfig = toml::from_str(toml_str).unwrap();
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);
    populate_registry_from_config(&registry, &config)
        .await
        .unwrap();

    let devices = registry.list_devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, "factory_device");
}

#[tokio::test]
async fn test_get_movable() {
    let registry = create_mock_registry().await.unwrap();

    let movable = registry.get_movable("mock_stage");
    assert!(movable.is_some());

    let not_movable = registry.get_movable("mock_power_meter");
    assert!(not_movable.is_none());
}

#[tokio::test]
async fn test_get_readable() {
    let registry = create_mock_registry().await.unwrap();

    let readable = registry.get_readable("mock_power_meter");
    assert!(readable.is_some());

    let not_readable = registry.get_readable("mock_stage");
    assert!(not_readable.is_none());
}

#[tokio::test]
async fn test_devices_with_capability() {
    let registry = create_mock_registry().await.unwrap();

    let movables = registry.devices_with_capability(Capability::Movable);
    assert_eq!(movables.len(), 1);
    assert!(movables.iter().any(|id| id == "mock_stage"));

    let readables = registry.devices_with_capability(Capability::Readable);
    assert_eq!(readables.len(), 1);
    assert!(readables.iter().any(|id| id == "mock_power_meter"));
}

#[tokio::test]
async fn test_duplicate_registration_fails() {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);

    registry
        .register_from_toml(
            "test",
            "Test Device",
            "mock_stage",
            toml::Value::Table({
                let mut m = toml::map::Map::new();
                m.insert("initial_position".into(), toml::Value::Float(0.0));
                m
            }),
        )
        .await
        .unwrap();

    let result = registry
        .register_from_toml(
            "test",
            "Duplicate",
            "mock_stage",
            toml::Value::Table({
                let mut m = toml::map::Map::new();
                m.insert("initial_position".into(), toml::Value::Float(0.0));
                m
            }),
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_unregister() {
    let registry = create_mock_registry().await.unwrap();

    assert!(registry.contains("mock_stage"));
    assert!(registry.unregister("mock_stage").await.unwrap());
    assert!(!registry.contains("mock_stage"));
    assert!(!registry.unregister("mock_stage").await.unwrap()); // Already removed
}

struct TestLifecycle {
    registered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    unregistered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl common::driver::DeviceLifecycle for TestLifecycle {
    fn on_register(&self) -> futures::future::BoxFuture<'static, Result<()>> {
        let counter = self.registered.clone();
        Box::pin(async move {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
    }

    fn on_unregister(&self) -> futures::future::BoxFuture<'static, Result<()>> {
        let counter = self.unregistered.clone();
        Box::pin(async move {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
    }
}

struct TestFactory {
    lifecycle: std::sync::Arc<dyn common::driver::DeviceLifecycle>,
}

impl common::driver::DriverFactory for TestFactory {
    fn driver_type(&self) -> &'static str {
        "test_factory"
    }

    fn name(&self) -> &'static str {
        "Test Factory"
    }

    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> futures::future::BoxFuture<'static, Result<DeviceComponents>> {
        let lifecycle = self.lifecycle.clone();
        Box::pin(async move {
            let driver = std::sync::Arc::new(crate::drivers::mock::MockStage::new());
            Ok(DeviceComponents::new()
                .with_movable(driver.clone())
                .with_parameterized(driver)
                .with_lifecycle(lifecycle))
        })
    }
}

struct LifecycleFactory {
    driver_type: &'static str,
    lifecycle: std::sync::Arc<dyn common::driver::DeviceLifecycle>,
}

impl common::driver::DriverFactory for LifecycleFactory {
    fn driver_type(&self) -> &'static str {
        self.driver_type
    }

    fn name(&self) -> &'static str {
        "Lifecycle Factory"
    }

    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> futures::future::BoxFuture<'static, Result<DeviceComponents>> {
        let lifecycle = self.lifecycle.clone();
        Box::pin(async move {
            let driver = std::sync::Arc::new(crate::drivers::mock::MockStage::new());
            Ok(DeviceComponents::new()
                .with_movable(driver.clone())
                .with_parameterized(driver)
                .with_lifecycle(lifecycle))
        })
    }
}

#[tokio::test]
async fn test_lifecycle_hooks_on_register_unregister() {
    let registered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let unregistered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let lifecycle = std::sync::Arc::new(TestLifecycle {
        registered: registered.clone(),
        unregistered: unregistered.clone(),
    });

    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(TestFactory {
        lifecycle: lifecycle.clone(),
    }));

    registry
        .register_from_toml(
            "test-device",
            "Test Device",
            "test_factory",
            toml::Value::Table(toml::map::Map::new()),
        )
        .await
        .unwrap();

    assert_eq!(registered.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(registry.unregister("test-device").await.unwrap());
    assert_eq!(unregistered.load(std::sync::atomic::Ordering::SeqCst), 1);
}

struct FailingLifecycle {
    unregistered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl common::driver::DeviceLifecycle for FailingLifecycle {
    fn on_register(&self) -> futures::future::BoxFuture<'static, Result<()>> {
        Box::pin(async { Err(anyhow!("boom")) })
    }

    fn on_unregister(&self) -> futures::future::BoxFuture<'static, Result<()>> {
        let counter = self.unregistered.clone();
        Box::pin(async move {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test]
async fn test_failed_lifecycle_register_cleans_up() {
    let unregistered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let lifecycle = std::sync::Arc::new(FailingLifecycle {
        unregistered: unregistered.clone(),
    });

    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(TestFactory { lifecycle }));

    let result = registry
        .register_from_toml(
            "test-device",
            "Test Device",
            "test_factory",
            toml::Value::Table(toml::map::Map::new()),
        )
        .await;

    assert!(matches!(result, Err(DaqError::Driver(_))));
    assert!(!registry.contains("test-device"));
    assert_eq!(unregistered.load(std::sync::atomic::Ordering::SeqCst), 1);
}

struct CountingLifecycle {
    unregistered: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    fail_on_unregister: bool,
}

impl common::driver::DeviceLifecycle for CountingLifecycle {
    fn on_unregister(&self) -> futures::future::BoxFuture<'static, Result<()>> {
        let counter = self.unregistered.clone();
        let fail = self.fail_on_unregister;
        Box::pin(async move {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fail { Err(anyhow!("boom")) } else { Ok(()) }
        })
    }
}

#[tokio::test]
async fn test_shutdown_all_attempts_all_unregister_hooks() {
    let ok_unregistered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fail_unregistered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let ok_lifecycle = std::sync::Arc::new(CountingLifecycle {
        unregistered: ok_unregistered.clone(),
        fail_on_unregister: false,
    });
    let fail_lifecycle = std::sync::Arc::new(CountingLifecycle {
        unregistered: fail_unregistered.clone(),
        fail_on_unregister: true,
    });

    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(LifecycleFactory {
        driver_type: "test_factory_ok",
        lifecycle: ok_lifecycle,
    }));
    registry.register_factory(Box::new(LifecycleFactory {
        driver_type: "test_factory_fail",
        lifecycle: fail_lifecycle,
    }));

    registry
        .register_from_toml(
            "test-device-ok",
            "Test Device Ok",
            "test_factory_ok",
            toml::Value::Table(toml::map::Map::new()),
        )
        .await
        .unwrap();
    registry
        .register_from_toml(
            "test-device-fail",
            "Test Device Fail",
            "test_factory_fail",
            toml::Value::Table(toml::map::Map::new()),
        )
        .await
        .unwrap();

    let result = registry.shutdown_all().await;
    let Err(DaqError::ShutdownFailed(errors)) = result else {
        panic!("Expected ShutdownFailed error");
    };

    assert_eq!(errors.len(), 1);
    assert!(!registry.contains("test-device-ok"));
    assert!(!registry.contains("test-device-fail"));
    assert_eq!(ok_unregistered.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        fail_unregistered.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn test_capability_access() {
    let registry = create_mock_registry().await.unwrap();

    // Test that we can use the movable interface
    let movable = registry.get_movable("mock_stage").unwrap();
    movable.move_abs(10.0).await.unwrap();
    let pos = movable.position().await.unwrap();
    assert!((pos - 10.0).abs() < 0.001);

    // Test that we can use the readable interface
    // MockPowerMeter noise model: shot_noise = 0.01 * sqrt(power) = 0.01 * sqrt(1e-6) = 1e-5
    // Use fixed tolerance of 1.5e-5 (1.5x max shot noise) to account for thermal floor
    let readable = registry.get_readable("mock_power_meter").unwrap();
    let reading = readable.read().await.unwrap();
    assert!(
        (reading - 1e-6).abs() < 1.5e-5,
        "Reading {reading} deviates more than 1.5e-5 from base 1e-6"
    );
}

#[tokio::test]
async fn test_snapshot_all_parameters() {
    let registry = create_mock_registry().await.unwrap();

    // Snapshot all parameters
    let snapshot = registry.snapshot_all_parameters();

    // Should have parameters from both mock devices
    assert!(!snapshot.is_empty(), "Snapshot should not be empty");

    // Mock devices implement Parameterized, so they should have parameters
    assert!(
        snapshot.contains_key("mock_stage") || snapshot.contains_key("mock_power_meter"),
        "Snapshot should contain at least one device"
    );

    // If a device is present, its parameters should be serializable JSON values
    for (device_id, params) in &snapshot {
        assert!(
            !params.is_empty(),
            "Device {device_id} should have parameters"
        );
        for (param_name, value) in params {
            assert!(
                value.is_number() || value.is_string() || value.is_boolean() || value.is_object(),
                "Parameter {device_id}.{param_name} should be a valid JSON value"
            );
        }
    }
}

#[cfg(feature = "serial")]
#[tokio::test]
async fn test_plugin_device_registration() {
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // Create a plugin factory and registry
    let factory = Arc::new(RwLock::new(
        crate::manifest_driver::registry::PluginFactory::new(),
    ));
    let registry = DeviceRegistry::with_plugin_factory(factory.clone());

    // Note: This test verifies that the plugin infrastructure is wired up correctly.
    // Actual plugin loading requires YAML files, which would be in integration tests.

    // Verify that we can access the plugin factory
    let factory_ref = registry.plugin_factory();
    assert!(Arc::ptr_eq(&factory, &factory_ref));

    // Verify that the registry starts empty
    assert_eq!(registry.len(), 0);
}

#[tokio::test]
async fn test_register_fails_on_unknown_driver_type() {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);

    let result = registry
        .register_from_toml(
            "invalid_device",
            "Invalid Device",
            "nonexistent_driver_type",
            toml::Value::Table(Default::default()),
        )
        .await;

    assert!(result.is_err());

    // Registry should remain empty
    assert_eq!(registry.len(), 0);
}

#[tokio::test]
async fn test_mock_camera_in_registry() {
    let registry = create_mock_registry().await.unwrap();

    // Verify mock_camera is registered
    assert!(registry.contains("mock_camera"));

    // Verify it has the expected capabilities through capability getters
    let frame_producer = registry.get_frame_producer("mock_camera");
    assert!(
        frame_producer.is_some(),
        "MockCamera should be retrievable as FrameProducer"
    );

    let triggerable = registry.get_triggerable("mock_camera");
    assert!(
        triggerable.is_some(),
        "MockCamera should be retrievable as Triggerable"
    );

    let exposure_control = registry.get_exposure_control("mock_camera");
    assert!(
        exposure_control.is_some(),
        "MockCamera should be retrievable as ExposureControl"
    );

    // Verify device info includes all capabilities
    let device_info = registry.get_device_info("mock_camera").unwrap();
    assert!(
        device_info
            .capabilities
            .contains(&Capability::FrameProducer)
    );
    assert!(device_info.capabilities.contains(&Capability::Triggerable));
    assert!(
        device_info
            .capabilities
            .contains(&Capability::ExposureControl)
    );
    assert_eq!(device_info.driver_type, "mock_camera");

    // Test that we can get parameters (bd-pf31: use get_parameterized)
    let parameterized = registry.get_parameterized("mock_camera").unwrap();
    let params = parameterized.parameters();
    assert!(params.get("exposure_s").is_some());
    assert!(params.get("armed").is_some());
    assert!(params.get("streaming").is_some());
    assert!(params.get("staged").is_some());
}

#[tokio::test]
async fn test_get_device_info_nonexistent() {
    let registry = create_mock_registry().await.unwrap();
    let info = registry.get_device_info("nonexistent");
    assert!(info.is_none());
}

#[tokio::test]
async fn test_registry_len() {
    let registry = DeviceRegistry::new();
    assert_eq!(registry.len(), 0);

    register_mock_factories(&registry);
    registry
        .register_from_toml(
            "test1",
            "Test 1",
            "mock_stage",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();
    assert_eq!(registry.len(), 1);

    registry
        .register_from_toml(
            "test2",
            "Test 2",
            "mock_stage",
            toml::Value::Table(Default::default()),
        )
        .await
        .unwrap();
    assert_eq!(registry.len(), 2);

    registry.unregister("test1").await.unwrap();
    assert_eq!(registry.len(), 1);
}

#[tokio::test]
async fn test_capability_getters_return_none_for_wrong_type() {
    let registry = create_mock_registry().await.unwrap();

    // mock_stage is not readable
    assert!(registry.get_readable("mock_stage").is_none());

    // mock_power_meter is not movable
    assert!(registry.get_movable("mock_power_meter").is_none());

    // mock_stage is not a frame producer
    assert!(registry.get_frame_producer("mock_stage").is_none());
}

#[tokio::test]
async fn test_devices_with_capability_empty_result() {
    let registry = create_mock_registry().await.unwrap();

    // No devices with WavelengthTunable capability in mock registry
    let tunable = registry.devices_with_capability(Capability::WavelengthTunable);
    assert_eq!(tunable.len(), 0);
}

#[tokio::test]
async fn test_shutdown_all_empty_registry() {
    let registry = DeviceRegistry::new();
    let result = registry.shutdown_all().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_multiple_capabilities_on_single_device() {
    let registry = create_mock_registry().await.unwrap();

    // mock_camera has multiple capabilities
    let frame_producer = registry.get_frame_producer("mock_camera");
    let triggerable = registry.get_triggerable("mock_camera");
    let exposure_control = registry.get_exposure_control("mock_camera");
    let parameterized = registry.get_parameterized("mock_camera");

    assert!(frame_producer.is_some());
    assert!(triggerable.is_some());
    assert!(exposure_control.is_some());
    assert!(parameterized.is_some());
}

#[tokio::test]
async fn test_factory_validation_failure() {
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);

    // Try to register with invalid config (driver expects valid types)
    let invalid_config = toml::Value::try_from(toml::toml! {
        invalid_field = "this_should_not_exist"
    })
    .unwrap();

    // The validation should happen before registration
    let result = registry
        .register_from_toml("invalid", "Invalid Device", "mock_stage", invalid_config)
        .await;

    // Result depends on whether the factory validates strictly
    // If validation passes, device should be registered
    if result.is_ok() {
        assert!(registry.contains("invalid"));
    } else {
        assert!(!registry.contains("invalid"));
    }
}

#[tokio::test]
async fn test_get_parameterized_for_all_devices() {
    let registry = create_mock_registry().await.unwrap();

    // All mock devices should implement Parameterized
    let devices = registry.list_devices();
    for device in devices {
        let parameterized = registry.get_parameterized(&device.id);
        assert!(
            parameterized.is_some(),
            "Device {} should be parameterized",
            device.id
        );
    }
}

#[tokio::test]
async fn test_list_devices_empty_registry() {
    let registry = DeviceRegistry::new();
    let devices = registry.list_devices();
    assert_eq!(devices.len(), 0);
}

#[tokio::test]
async fn test_unregister_nonexistent_device() {
    let registry = DeviceRegistry::new();
    let result = registry.unregister("nonexistent").await.unwrap();
    assert!(!result, "Should return false for nonexistent device");
}

// ── HeartbeatConfig deserialization tests (bd-nfav) ─────────────

#[test]
fn test_heartbeat_config_all_fields() {
    let toml_str = r#"
        enabled = false
        device = "/dev/comedi0"
        subdevice = 2
        channel = 7
        interval_ms = 50
    "#;

    let config: super::HeartbeatConfig =
        toml::from_str(toml_str).expect("should deserialize HeartbeatConfig with all fields");
    assert!(!config.enabled);
    assert_eq!(config.device, "/dev/comedi0");
    assert_eq!(config.subdevice, Some(2));
    assert_eq!(config.channel, 7);
    assert_eq!(config.interval_ms, 50);
}

#[test]
fn test_heartbeat_config_required_fields_only() {
    let toml_str = r#"
        device = "/dev/comedi0"
        channel = 3
    "#;

    let config: super::HeartbeatConfig = toml::from_str(toml_str)
        .expect("should deserialize HeartbeatConfig with required fields only");
    assert!(config.enabled, "enabled should default to true");
    assert_eq!(config.device, "/dev/comedi0");
    assert_eq!(config.subdevice, None, "subdevice should default to None");
    assert_eq!(config.channel, 3);
    assert_eq!(config.interval_ms, 100, "interval_ms should default to 100");
}

#[test]
fn test_hardware_config_without_heartbeat() {
    let toml_str = r#"
        [[devices]]
        id = "test_stage"
        name = "Test Stage"
        [devices.driver]
        type = "mock_stage"
    "#;

    let config: super::HardwareConfig = toml::from_str(toml_str)
        .expect("should deserialize HardwareConfig without safety_heartbeat");
    assert!(
        config.safety_heartbeat.is_none(),
        "safety_heartbeat should be None when section is absent"
    );
}

#[test]
fn test_hardware_config_with_heartbeat() {
    let toml_str = r#"
        [[devices]]
        id = "test_stage"
        name = "Test Stage"
        [devices.driver]
        type = "mock_stage"

        [safety_heartbeat]
        device = "/dev/comedi0"
        channel = 5
    "#;

    let config: super::HardwareConfig =
        toml::from_str(toml_str).expect("should deserialize HardwareConfig with safety_heartbeat");
    let hb = config
        .safety_heartbeat
        .expect("safety_heartbeat should be Some");
    assert!(hb.enabled, "enabled should default to true");
    assert_eq!(hb.device, "/dev/comedi0");
    assert_eq!(hb.channel, 5);
    assert_eq!(hb.interval_ms, 100, "interval_ms should default to 100");
}

#[test]
fn test_heartbeat_config_defaults() {
    // Verify the default helper functions return expected values
    assert!(super::types::default_heartbeat_enabled());
    assert_eq!(super::types::default_heartbeat_interval_ms(), 100);
}

#[tokio::test]
async fn test_device_metadata_preserved() {
    let registry = create_mock_registry().await.unwrap();
    let device_info = registry.get_device_info("mock_stage").unwrap();

    assert_eq!(device_info.id, "mock_stage");
    assert_eq!(device_info.name, "Mock Stage");
    assert_eq!(device_info.driver_type, "mock_stage");
}

// =========================================================================
// Health broadcast tests (bd-vgrj)
// =========================================================================

#[tokio::test]
async fn test_health_broadcast_on_failure() {
    let registry = create_mock_registry().await.unwrap();
    let mut rx = registry.subscribe_health_changes();

    // First failure transitions Healthy -> Degraded
    registry.report_device_failure("mock_stage", "test error");

    let event = rx.try_recv().expect("should receive health event");
    assert_eq!(event.device_id, "mock_stage");
    assert_eq!(event.old_state, DeviceHealth::Healthy);
    assert_eq!(event.new_state, DeviceHealth::Degraded);
    assert_eq!(event.consecutive_failures, 1);
}

#[tokio::test]
async fn test_health_broadcast_on_success() {
    let registry = create_mock_registry().await.unwrap();
    let mut rx = registry.subscribe_health_changes();

    // Cause degradation first
    registry.report_device_failure("mock_stage", "test error");
    let _ = rx.try_recv(); // consume the Healthy->Degraded event

    // Success should transition Degraded -> Healthy
    registry.report_device_success("mock_stage");

    let event = rx.try_recv().expect("should receive health event");
    assert_eq!(event.device_id, "mock_stage");
    assert_eq!(event.old_state, DeviceHealth::Degraded);
    assert_eq!(event.new_state, DeviceHealth::Healthy);
}

#[tokio::test]
async fn test_health_broadcast_no_event_when_unchanged() {
    let registry = create_mock_registry().await.unwrap();
    let mut rx = registry.subscribe_health_changes();

    // Success on already-healthy device should not emit
    registry.report_device_success("mock_stage");
    assert!(
        rx.try_recv().is_err(),
        "no event expected when health unchanged"
    );
}

#[tokio::test]
async fn test_health_no_subscribers_no_error() {
    let registry = create_mock_registry().await.unwrap();
    // No subscribers — should not panic or error
    registry.report_device_failure("mock_stage", "test error");
    registry.report_device_success("mock_stage");
}

#[tokio::test]
async fn test_subscribe_health_changes_returns_receiver() {
    let registry = create_mock_registry().await.unwrap();
    let _rx1 = registry.subscribe_health_changes();
    let _rx2 = registry.subscribe_health_changes();
    // Multiple subscriptions should work without issue
}

// =========================================================================
// StateRefreshable / restart_device integration tests (bd-47p2)
// =========================================================================

/// Shared counter for tracking refresh_state calls across factory rebuilds.
type SharedCallCount = Arc<std::sync::atomic::AtomicU32>;

/// A mock device that tracks refresh_state calls via a shared counter.
struct MockRefreshableDevice {
    call_count: SharedCallCount,
}

#[async_trait::async_trait]
impl Movable for MockRefreshableDevice {
    async fn move_abs(&self, _position: f64) -> Result<()> {
        Ok(())
    }
    async fn move_rel(&self, _distance: f64) -> Result<()> {
        Ok(())
    }
    async fn position(&self) -> Result<f64> {
        Ok(42.0)
    }
    async fn stop(&self) -> Result<()> {
        Ok(())
    }
    async fn wait_settled(&self) -> Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl StateRefreshable for MockRefreshableDevice {
    async fn refresh_state(&self) -> Result<HashMap<String, serde_json::Value>> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let mut state = HashMap::new();
        state.insert("position".to_string(), serde_json::json!(42.0));
        Ok(state)
    }
}

/// A factory that produces MockRefreshableDevice instances and shares a
/// call counter so tests can verify refresh_state was invoked.
struct MockRefreshableFactory {
    call_count: SharedCallCount,
}

impl MockRefreshableFactory {
    fn new(call_count: SharedCallCount) -> Self {
        Self { call_count }
    }
}

impl DriverFactory for MockRefreshableFactory {
    fn driver_type(&self) -> &'static str {
        "mock_refreshable"
    }

    fn name(&self) -> &'static str {
        "Mock Refreshable Device"
    }

    fn capabilities(&self) -> &'static [Capability] {
        &[Capability::Movable, Capability::StateRefreshable]
    }

    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }

    fn build(
        &self,
        _config: toml::Value,
    ) -> futures::future::BoxFuture<'static, Result<DeviceComponents>> {
        let call_count = self.call_count.clone();
        Box::pin(async move {
            let device = Arc::new(MockRefreshableDevice { call_count });
            let components = DeviceComponents::new()
                .with_movable(device.clone() as Arc<dyn Movable>)
                .with_state_refreshable(device as Arc<dyn StateRefreshable>);
            Ok(components)
        })
    }
}

#[tokio::test]
async fn test_restart_device_triggers_state_refresh() {
    let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(MockRefreshableFactory::new(call_count.clone())));

    // Register the device
    registry
        .register_from_toml(
            "refreshable_dev",
            "Refreshable Device",
            "mock_refreshable",
            toml::Value::Table(Default::default()),
        )
        .await
        .expect("registration should succeed");

    // Verify device is registered with StateRefreshable
    assert!(registry.get_state_refreshable("refreshable_dev").is_some());
    assert!(registry.get_movable("refreshable_dev").is_some());

    // No refresh calls yet (registration does not trigger refresh)
    assert_eq!(call_count.load(Ordering::SeqCst), 0);

    // Fault the device so restart_device will attempt restart
    registry.set_fault_threshold(1);
    registry.report_device_failure("refreshable_dev", "simulated fault");

    // Verify device is faulted
    let health = registry
        .get_device_health("refreshable_dev")
        .expect("health should exist");
    assert_eq!(health.health, DeviceHealth::Faulted);

    // Restart the device — should trigger state refresh
    let result = registry.restart_device("refreshable_dev").await;
    assert!(result.is_ok());
    assert!(result.unwrap(), "restart should succeed and return true");

    // refresh_state should have been called exactly once on the new instance
    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "refresh_state should be called once after restart"
    );

    // Capabilities should still be wired up after restart
    assert!(registry.get_state_refreshable("refreshable_dev").is_some());
    assert!(registry.get_movable("refreshable_dev").is_some());
}

#[tokio::test]
async fn test_restart_device_without_state_refreshable_succeeds() {
    // Ensure restart works fine for devices that don't implement StateRefreshable
    let registry = create_mock_registry().await.unwrap();

    // Mock stage does not implement StateRefreshable
    assert!(registry.get_state_refreshable("mock_stage").is_none());

    // Fault and restart
    registry.set_fault_threshold(1);
    registry.report_device_failure("mock_stage", "simulated fault");
    let result = registry.restart_device("mock_stage").await;
    assert!(result.is_ok());
    assert!(result.unwrap(), "restart should succeed");

    // Device should be healthy again
    let health = registry
        .get_device_health("mock_stage")
        .expect("health should exist");
    assert_eq!(health.health, DeviceHealth::Healthy);
}

// =========================================================================
// Universal manifest resolution tests
// =========================================================================

#[test]
fn resolve_universal_factory_name_from_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest_path = dir.path().join("test_device.toml");
    std::fs::write(
        &manifest_path,
        r#"
schema_version = 3

[device]
name = "Siglent SDG1025"
protocol = "siglent_sdg"
capabilities = ["Settable"]

[connection]
type = "serial"
baud_rate = 115200
"#,
    )
    .expect("write manifest");

    let manifest_str = manifest_path.to_str().expect("valid utf-8 path");
    let mut config = toml::map::Map::new();
    config.insert(
        "manifest".to_string(),
        toml::Value::String(manifest_str.to_string()),
    );
    config.insert(
        "port".to_string(),
        toml::Value::String("/dev/ttyUSB7".to_string()),
    );

    let derived =
        resolve_universal_factory_name(&toml::Value::Table(config)).expect("should resolve");
    assert_eq!(derived, "universal_siglent_sdg1025");
}

#[test]
fn resolve_universal_factory_name_missing_manifest_field() {
    let mut config = toml::map::Map::new();
    config.insert(
        "port".to_string(),
        toml::Value::String("/dev/ttyUSB7".to_string()),
    );

    let err = resolve_universal_factory_name(&toml::Value::Table(config))
        .expect_err("should fail without manifest");
    let msg = err.to_string();
    assert!(
        msg.contains("requires a 'manifest' field"),
        "unexpected error: {msg}"
    );
}

#[test]
fn resolve_universal_factory_name_missing_file() {
    let mut config = toml::map::Map::new();
    config.insert(
        "manifest".to_string(),
        toml::Value::String("/nonexistent/path/device.toml".to_string()),
    );

    let err = resolve_universal_factory_name(&toml::Value::Table(config))
        .expect_err("should fail with missing file");
    let msg = err.to_string();
    assert!(msg.contains("Failed to read"), "unexpected error: {msg}");
}

#[test]
fn resolve_universal_factory_name_missing_device_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest_path = dir.path().join("bad.toml");
    std::fs::write(
        &manifest_path,
        r#"
schema_version = 3

[connection]
type = "serial"
"#,
    )
    .expect("write manifest");

    let manifest_str = manifest_path.to_str().expect("valid utf-8 path");
    let mut config = toml::map::Map::new();
    config.insert(
        "manifest".to_string(),
        toml::Value::String(manifest_str.to_string()),
    );

    let err = resolve_universal_factory_name(&toml::Value::Table(config))
        .expect_err("should fail without device.name");
    let msg = err.to_string();
    assert!(
        msg.contains("missing [device].name"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn register_from_toml_resolves_universal_type() {
    // Set up a temporary manifest file
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest_path = dir.path().join("test_device.toml");
    std::fs::write(
        &manifest_path,
        r#"
schema_version = 3

[device]
name = "Test Device"
protocol = "test"
capabilities = ["Movable"]

[connection]
type = "serial"
baud_rate = 9600
"#,
    )
    .expect("write manifest");

    // Create a registry and register a factory under the derived name
    // "universal_test_device" to simulate what load_all_factories would do.
    let registry = DeviceRegistry::new();
    register_mock_factories(&registry);

    let noop_lifecycle = std::sync::Arc::new(TestLifecycle {
        registered: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        unregistered: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });
    registry.register_factory(Box::new(LifecycleFactory {
        driver_type: "universal_test_device",
        lifecycle: noop_lifecycle,
    }));

    // Now attempt to register a device with type = "universal" + manifest
    let manifest_str = manifest_path.to_str().expect("valid utf-8 path");
    let mut config_table = toml::map::Map::new();
    config_table.insert(
        "manifest".to_string(),
        toml::Value::String(manifest_str.to_string()),
    );
    config_table.insert(
        "port".to_string(),
        toml::Value::String("/dev/ttyUSB0".to_string()),
    );
    config_table.insert("mock".to_string(), toml::Value::Boolean(true));

    let result = registry
        .register_from_toml(
            "my_device",
            "My Device",
            "universal",
            toml::Value::Table(config_table),
        )
        .await;

    // The factory lookup should succeed (resolving universal -> universal_test_device).
    // The build uses LifecycleFactory which creates a MockStage, so it should succeed.
    assert!(
        result.is_ok(),
        "register_from_toml should succeed but got: {:?}",
        result.err()
    );

    let info = registry.list_devices();
    let dev = info.iter().find(|d| d.id == "my_device");
    assert!(dev.is_some(), "device should be registered");
}
