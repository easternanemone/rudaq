use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::Movable;
use common::driver::{DeviceComponents, DriverFactory};
use common::error::DaqError;
use daq_hardware::DeviceRegistry;
use daq_server::grpc::{HardwareService, HardwareServiceImpl, proto::MoveRequest};
use futures::future::BoxFuture;
use tonic::{Code, Request};

struct FailingMovable;

#[async_trait]
impl Movable for FailingMovable {
    async fn move_abs(&self, _position: f64) -> Result<()> {
        Err(DaqError::Instrument("boom".into()).into())
    }

    async fn move_rel(&self, _distance: f64) -> Result<()> {
        Ok(())
    }

    async fn position(&self) -> Result<f64> {
        Ok(0.0)
    }

    async fn wait_settled(&self) -> Result<()> {
        Ok(())
    }
}

struct FailingMovableFactory;

impl DriverFactory for FailingMovableFactory {
    fn driver_type(&self) -> &'static str {
        "failing_movable"
    }

    fn name(&self) -> &'static str {
        "Failing Movable"
    }

    fn validate(&self, _config: &toml::Value) -> Result<()> {
        Ok(())
    }

    fn build(&self, _config: toml::Value) -> BoxFuture<'static, Result<DeviceComponents>> {
        Box::pin(async move {
            let driver = Arc::new(FailingMovable);
            Ok(DeviceComponents::new().with_movable(driver))
        })
    }
}

#[tokio::test]
async fn hardware_service_maps_daq_errors_via_central_mapping() {
    let registry = DeviceRegistry::new();
    registry.register_factory(Box::new(FailingMovableFactory));
    registry
        .register_from_toml(
            "test-device",
            "Test Device",
            "failing_movable",
            toml::Value::Table(toml::map::Map::new()),
        )
        .await
        .unwrap();

    let service = HardwareServiceImpl::new(Arc::new(registry));

    let request = Request::new(MoveRequest {
        device_id: "test-device".to_string(),
        value: 1.0,
        wait_for_completion: None,
        timeout_ms: None,
    });

    let status = service.move_absolute(request).await.unwrap_err();

    assert_eq!(status.code(), Code::Unavailable);
    let error_kind = status
        .metadata()
        .get("x-daq-error-kind")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("<missing>");
    assert_eq!(error_kind, "instrument");
}
