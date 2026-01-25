use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use daq_core::driver::{DeviceComponents, DeviceLifecycle, DriverFactory};
use daq_hardware::DeviceRegistry;

struct RecordingLifecycle {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl DeviceLifecycle for RecordingLifecycle {
    fn on_register(&self) -> futures::future::BoxFuture<'static, Result<()>> {
        let events = self.events.clone();
        Box::pin(async move {
            events.lock().unwrap().push("on_register");
            Err(anyhow!("on_register failure"))
        })
    }

    fn on_unregister(&self) -> futures::future::BoxFuture<'static, Result<()>> {
        let events = self.events.clone();
        Box::pin(async move {
            events.lock().unwrap().push("on_unregister");
            Ok(())
        })
    }
}

struct RecordingFactory {
    lifecycle: Arc<dyn DeviceLifecycle>,
}

impl DriverFactory for RecordingFactory {
    fn driver_type(&self) -> &'static str {
        "recording_factory"
    }

    fn name(&self) -> &'static str {
        "Recording Factory"
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
            let driver = Arc::new(daq_hardware::drivers::mock::MockStage::new());
            Ok(DeviceComponents::new()
                .with_movable(driver.clone())
                .with_parameterized(driver)
                .with_lifecycle(lifecycle))
        })
    }
}

#[tokio::test]
async fn lifecycle_failure_triggers_unregister_in_order() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let lifecycle = Arc::new(RecordingLifecycle {
        events: events.clone(),
    });

    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(RecordingFactory { lifecycle }));

    let result = registry
        .register_from_toml(
            "test-device",
            "Test Device",
            "recording_factory",
            toml::Value::Table(toml::map::Map::new()),
        )
        .await;

    assert!(result.is_err());
    assert!(!registry.contains("test-device"));

    let recorded = events.lock().unwrap().clone();
    assert_eq!(recorded, vec!["on_register", "on_unregister"]);
}
