//! gRPC ConfigService implementation (bd-itsc).
//!
//! Provides CRUD access to the SurrealDB control plane database.
//! Changes made via this service are automatically detected by the watch
//! reconciler (LIVE SELECT) and reconciled to the hardware registry.

use db::DaqDb;
use db::config_store::{DbDriver, DbInstrument};
use hardware::registry::DeviceRegistry;
use std::collections::{HashMap, HashSet};
use tonic::{Request, Response, Status};
use tracing::instrument;

use crate::grpc::proto::config_service_server::ConfigService;
use crate::grpc::proto::{
    ConfigChangeEvent, DbInfoResponse, DeleteInstrumentRequest, DeleteInstrumentResponse,
    DriverConfig, ExportConfigRequest, ExportConfigResponse, GetDbInfoRequest,
    GetInstrumentRequest, ImportConfigRequest, ImportConfigResponse, InstrumentConfig,
    ListDriversRequest, ListDriversResponse, ListInstrumentsRequest, ListInstrumentsResponse,
    SubscribeConfigRequest, UpsertInstrumentRequest, UpsertInstrumentResponse,
};

/// ConfigService gRPC handler backed by SurrealDB.
pub struct ConfigServiceImpl {
    db: DaqDb,
    registry: Option<DeviceRegistry>,
}

impl ConfigServiceImpl {
    pub fn new(db: DaqDb, registry: Option<DeviceRegistry>) -> Self {
        Self { db, registry }
    }

    async fn drivers_from_config(
        &self,
        hw_config: &hardware::registry::HardwareConfig,
    ) -> Result<Vec<DbDriver>, Status> {
        let existing_by_driver_type: HashMap<String, DbDriver> = self
            .db
            .get_all_drivers()
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .into_iter()
            .map(|driver| (driver.driver_type.clone(), driver))
            .collect();

        let mut seen = HashSet::new();
        let drivers = hw_config
            .devices
            .iter()
            .filter(|device| seen.insert(device.driver.driver_type.clone()))
            .map(|device| {
                let driver_type = device.driver.driver_type.clone();
                if let Some(info) = self
                    .registry
                    .as_ref()
                    .and_then(|registry| registry.factory_info(&driver_type))
                {
                    DbDriver {
                        driver_type,
                        name: info.name,
                        capabilities: info.capabilities,
                        commands: info.available_commands,
                    }
                } else if let Some(existing) = existing_by_driver_type.get(&driver_type) {
                    existing.clone()
                } else {
                    DbDriver {
                        driver_type: driver_type.clone(),
                        name: driver_type,
                        capabilities: vec![],
                        commands: vec![],
                    }
                }
            })
            .collect();

        Ok(drivers)
    }
}

// ---------------------------------------------------------------------------
// Conversions: DB types <-> proto types
// ---------------------------------------------------------------------------

fn db_instrument_to_proto(inst: &DbInstrument) -> InstrumentConfig {
    InstrumentConfig {
        device_id: inst.device_id.clone(),
        name: inst.name.clone(),
        driver_type: inst.driver_type.clone(),
        config_json: inst.config.to_string(),
        enabled: inst.enabled,
    }
}

#[allow(clippy::result_large_err)] // tonic::Status is the standard gRPC error type
fn proto_to_db_instrument(proto: &InstrumentConfig) -> Result<DbInstrument, Status> {
    let config: serde_json::Value = if proto.config_json.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&proto.config_json)
            .map_err(|e| Status::invalid_argument(format!("invalid config_json: {e}")))?
    };

    Ok(DbInstrument {
        device_id: proto.device_id.clone(),
        name: proto.name.clone(),
        driver_type: proto.driver_type.clone(),
        config,
        enabled: proto.enabled,
    })
}

fn db_driver_to_proto(drv: &DbDriver) -> DriverConfig {
    DriverConfig {
        driver_type: drv.driver_type.clone(),
        name: drv.name.clone(),
        capabilities: drv.capabilities.clone(),
        commands: drv.commands.clone(),
    }
}

// ---------------------------------------------------------------------------
// ConfigService trait implementation
// ---------------------------------------------------------------------------

#[tonic::async_trait]
impl ConfigService for ConfigServiceImpl {
    #[instrument(skip(self, _req))]
    async fn list_instruments(
        &self,
        _req: Request<ListInstrumentsRequest>,
    ) -> Result<Response<ListInstrumentsResponse>, Status> {
        let instruments = self
            .db
            .get_all_instruments()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ListInstrumentsResponse {
            instruments: instruments.iter().map(db_instrument_to_proto).collect(),
        }))
    }

    #[instrument(skip(self, req), fields(device_id))]
    async fn get_instrument(
        &self,
        req: Request<GetInstrumentRequest>,
    ) -> Result<Response<InstrumentConfig>, Status> {
        let device_id = &req.into_inner().device_id;
        tracing::Span::current().record("device_id", device_id);

        let instrument = self
            .db
            .get_instrument(device_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?
            .ok_or_else(|| Status::not_found(format!("instrument '{device_id}' not found")))?;

        Ok(Response::new(db_instrument_to_proto(&instrument)))
    }

    #[instrument(skip(self, req), fields(device_id))]
    async fn upsert_instrument(
        &self,
        req: Request<UpsertInstrumentRequest>,
    ) -> Result<Response<UpsertInstrumentResponse>, Status> {
        let proto = req
            .into_inner()
            .instrument
            .ok_or_else(|| Status::invalid_argument("instrument required"))?;

        tracing::Span::current().record("device_id", &proto.device_id);

        if proto.device_id.is_empty() {
            return Err(Status::invalid_argument("device_id must not be empty"));
        }
        if proto.driver_type.is_empty() {
            return Err(Status::invalid_argument("driver_type must not be empty"));
        }

        let db_inst = proto_to_db_instrument(&proto)?;
        let report = self
            .db
            .upsert_instruments(&[db_inst])
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        if report.errors.is_empty() {
            Ok(Response::new(UpsertInstrumentResponse {
                success: true,
                message: format!("upserted '{}'", proto.device_id),
            }))
        } else {
            Err(Status::internal(report.errors.join("; ")))
        }
    }

    #[instrument(skip(self, req), fields(device_id))]
    async fn delete_instrument(
        &self,
        req: Request<DeleteInstrumentRequest>,
    ) -> Result<Response<DeleteInstrumentResponse>, Status> {
        let device_id = req.into_inner().device_id;
        tracing::Span::current().record("device_id", &device_id);

        let deleted = self
            .db
            .delete_instrument(&device_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        if deleted {
            Ok(Response::new(DeleteInstrumentResponse {
                success: true,
                message: format!("deleted '{device_id}'"),
            }))
        } else {
            Err(Status::not_found(format!(
                "instrument '{device_id}' not found"
            )))
        }
    }

    #[instrument(skip(self, _req))]
    async fn list_drivers(
        &self,
        _req: Request<ListDriversRequest>,
    ) -> Result<Response<ListDriversResponse>, Status> {
        let drivers = self
            .db
            .get_all_drivers()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ListDriversResponse {
            drivers: drivers.iter().map(db_driver_to_proto).collect(),
        }))
    }

    #[instrument(skip(self, req))]
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    async fn import_config(
        &self,
        req: Request<ImportConfigRequest>,
    ) -> Result<Response<ImportConfigResponse>, Status> {
        let toml_content = &req.into_inner().toml_content;

        // Parse the TOML as HardwareConfig
        let hw_config: hardware::registry::HardwareConfig = toml::from_str(toml_content)
            .map_err(|e| Status::invalid_argument(format!("invalid TOML hardware config: {e}")))?;

        // Convert to DB types (same logic as db_bridge)
        let instruments: Vec<DbInstrument> = hw_config
            .devices
            .iter()
            .map(|d| DbInstrument {
                device_id: d.id.clone(),
                name: d.name.clone(),
                driver_type: d.driver.driver_type.clone(),
                config: db::config_store::toml_to_json(&d.driver.config),
                enabled: d.enabled,
            })
            .collect();

        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: value is bounded and fits in target type
        let drivers = self.drivers_from_config(&hw_config).await?;

        let driver_count = self
            .db
            .upsert_drivers(&drivers)
            .await
            .map_err(|e| Status::internal(e.to_string()))? as u32;

        let report = self
            .db
            .upsert_instruments(&instruments)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ImportConfigResponse {
            success: report.errors.is_empty(),
            message: if report.errors.is_empty() {
                "import successful".into()
            } else {
                report.errors.join("; ")
            },
            drivers_imported: driver_count,
            instruments_imported: report.instruments_upserted as u32,
        }))
    }

    #[instrument(skip(self, _req))]
    async fn export_config(
        &self,
        _req: Request<ExportConfigRequest>,
    ) -> Result<Response<ExportConfigResponse>, Status> {
        let instruments = self
            .db
            .get_all_instruments()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Convert DB instruments to HardwareConfig TOML
        let devices: Vec<hardware::registry::DeviceConfig> = instruments
            .iter()
            .map(|inst| hardware::registry::DeviceConfig {
                id: inst.device_id.clone(),
                name: inst.name.clone(),
                driver: hardware::registry::DriverConfig::new(
                    inst.driver_type.clone(),
                    db::config_store::json_to_toml(&inst.config),
                ),
                enabled: inst.enabled,
            })
            .collect();

        let hw_config = hardware::registry::HardwareConfig {
            plugin_paths: vec![],
            devices,
            safety_heartbeat: None,
        };

        let toml_content = toml::to_string_pretty(&hw_config)
            .map_err(|e| Status::internal(format!("failed to serialize config: {e}")))?;

        Ok(Response::new(ExportConfigResponse { toml_content }))
    }

    #[instrument(skip(self, _req))]
    async fn get_db_info(
        &self,
        _req: Request<GetDbInfoRequest>,
    ) -> Result<Response<DbInfoResponse>, Status> {
        let info = self.db.info().await;
        Ok(Response::new(DbInfoResponse {
            engine: info.engine,
            namespace: info.namespace,
            database: info.database,
            schema_version: info.schema_version,
            uptime_secs: info.uptime_secs,
            healthy: info.healthy,
            driver_count: info.driver_count,
            instrument_count: info.instrument_count,
        }))
    }

    type SubscribeConfigChangesStream =
        tokio_stream::wrappers::ReceiverStream<Result<ConfigChangeEvent, Status>>;

    #[instrument(skip(self, _req))]
    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: value is bounded and fits in target type
    async fn subscribe_config_changes(
        &self,
        _req: Request<SubscribeConfigRequest>,
    ) -> Result<Response<Self::SubscribeConfigChangesStream>, Status> {
        use futures::StreamExt;

        let db = self.db.clone();

        let (tx, rx) = tokio::sync::mpsc::channel(64);

        let live_stream = db.live_instruments().await.map_err(|e| {
            tracing::error!("failed to start live query: {e}");
            Status::internal(e.to_string())
        })?;

        tokio::spawn(async move {
            futures::pin_mut!(live_stream);
            while let Some(result) = live_stream.next().await {
                let event = match result {
                    Ok(notification) => {
                        use db::surrealdb::Action;
                        let (change_type, device_id, instrument) = match notification.action {
                            Action::Create | Action::Update => {
                                let inst = &notification.data;
                                (
                                    match notification.action {
                                        Action::Create => "create",
                                        _ => "upsert",
                                    },
                                    inst.device_id.clone(),
                                    Some(db_instrument_to_proto(inst)),
                                )
                            }
                            Action::Delete => {
                                let inst = &notification.data;
                                ("delete", inst.device_id.clone(), None)
                            }
                            _ => continue,
                        };
                        ConfigChangeEvent {
                            change_type: change_type.to_string(),
                            device_id,
                            instrument,
                            timestamp_ns: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos() as u64,
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "live query error");
                        continue;
                    }
                };

                if tx.send(Ok(event)).await.is_err() {
                    break; // Client disconnected
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[cfg(feature = "db-surreal")]
mod tests {
    use super::*;
    use db::DbConfig;
    use tonic::Request;

    async fn test_service() -> ConfigServiceImpl {
        let db = DaqDb::init(DbConfig::in_memory()).await.unwrap();
        ConfigServiceImpl::new(db, None)
    }

    fn sample_instrument() -> InstrumentConfig {
        InstrumentConfig {
            device_id: "test_device".into(),
            name: "Test Device".into(),
            driver_type: "mock".into(),
            config_json: r#"{"port":"/dev/null"}"#.into(),
            enabled: true,
        }
    }

    #[tokio::test]
    async fn test_list_instruments_empty() {
        let svc = test_service().await;
        let resp = svc
            .list_instruments(Request::new(ListInstrumentsRequest {}))
            .await
            .unwrap();
        assert!(resp.into_inner().instruments.is_empty());
    }

    #[tokio::test]
    async fn test_upsert_and_get_roundtrip() {
        let svc = test_service().await;

        // Upsert
        let resp = svc
            .upsert_instrument(Request::new(UpsertInstrumentRequest {
                instrument: Some(sample_instrument()),
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().success);

        // Get
        let resp = svc
            .get_instrument(Request::new(GetInstrumentRequest {
                device_id: "test_device".into(),
            }))
            .await
            .unwrap();
        let inst = resp.into_inner();
        assert_eq!(inst.device_id, "test_device");
        assert_eq!(inst.driver_type, "mock");
        assert!(inst.enabled);
    }

    #[tokio::test]
    async fn test_upsert_twice_updates_not_duplicates() {
        let svc = test_service().await;

        let mut inst = sample_instrument();
        svc.upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(inst.clone()),
        }))
        .await
        .unwrap();

        inst.name = "Updated Name".into();
        svc.upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(inst),
        }))
        .await
        .unwrap();

        let resp = svc
            .list_instruments(Request::new(ListInstrumentsRequest {}))
            .await
            .unwrap();
        let instruments = resp.into_inner().instruments;
        assert_eq!(instruments.len(), 1);
        assert_eq!(instruments[0].name, "Updated Name");
    }

    #[tokio::test]
    async fn test_delete_instrument() {
        let svc = test_service().await;

        // Insert then delete
        svc.upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(sample_instrument()),
        }))
        .await
        .unwrap();

        let resp = svc
            .delete_instrument(Request::new(DeleteInstrumentRequest {
                device_id: "test_device".into(),
            }))
            .await
            .unwrap();
        assert!(resp.into_inner().success);

        // Verify gone
        let resp = svc
            .list_instruments(Request::new(ListInstrumentsRequest {}))
            .await
            .unwrap();
        assert!(resp.into_inner().instruments.is_empty());
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let svc = test_service().await;
        let result = svc
            .delete_instrument(Request::new(DeleteInstrumentRequest {
                device_id: "does_not_exist".into(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let svc = test_service().await;
        let result = svc
            .get_instrument(Request::new(GetInstrumentRequest {
                device_id: "does_not_exist".into(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn test_upsert_empty_device_id_rejected() {
        let svc = test_service().await;
        let mut inst = sample_instrument();
        inst.device_id = String::new();
        let result = svc
            .upsert_instrument(Request::new(UpsertInstrumentRequest {
                instrument: Some(inst),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_upsert_missing_instrument() {
        let svc = test_service().await;
        let result = svc
            .upsert_instrument(Request::new(UpsertInstrumentRequest { instrument: None }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_import_valid_toml() {
        let svc = test_service().await;
        let toml_content = r#"
            [[devices]]
            id = "laser"
            name = "Test Laser"
            enabled = true

            [devices.driver]
            type = "mock"
            power = 100
        "#;

        let resp = svc
            .import_config(Request::new(ImportConfigRequest {
                toml_content: toml_content.into(),
            }))
            .await
            .unwrap();
        let inner = resp.into_inner();
        assert!(inner.success);
        assert_eq!(inner.instruments_imported, 1);
        assert_eq!(inner.drivers_imported, 1);

        // Verify it's in the DB
        let resp = svc
            .list_instruments(Request::new(ListInstrumentsRequest {}))
            .await
            .unwrap();
        assert_eq!(resp.into_inner().instruments.len(), 1);
    }

    #[tokio::test]
    async fn test_import_invalid_toml() {
        let svc = test_service().await;
        let result = svc
            .import_config(Request::new(ImportConfigRequest {
                toml_content: "not { valid toml [[".into(),
            }))
            .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_export_reimportable() {
        let svc = test_service().await;

        // Import some data
        let toml_content = r#"
            [[devices]]
            id = "sensor_1"
            name = "Sensor"
            enabled = true

            [devices.driver]
            type = "mock"
            rate = 100
        "#;
        svc.import_config(Request::new(ImportConfigRequest {
            toml_content: toml_content.into(),
        }))
        .await
        .unwrap();

        // Export
        let resp = svc
            .export_config(Request::new(ExportConfigRequest {}))
            .await
            .unwrap();
        let exported = resp.into_inner().toml_content;
        assert!(!exported.is_empty());

        // Verify exported TOML is valid and parseable
        let _: hardware::registry::HardwareConfig = toml::from_str(&exported).unwrap();
    }

    #[tokio::test]
    async fn test_list_drivers() {
        let svc = test_service().await;

        // Import data which creates drivers
        let toml_content = r#"
            [[devices]]
            id = "dev1"
            name = "Dev 1"
            [devices.driver]
            type = "mock"
        "#;
        svc.import_config(Request::new(ImportConfigRequest {
            toml_content: toml_content.into(),
        }))
        .await
        .unwrap();

        let resp = svc
            .list_drivers(Request::new(ListDriversRequest {}))
            .await
            .unwrap();
        let drivers = resp.into_inner().drivers;
        assert!(!drivers.is_empty());
        assert_eq!(drivers[0].driver_type, "mock");
    }

    #[tokio::test]
    async fn test_import_preserves_existing_driver_metadata() {
        let svc = test_service().await;

        svc.db
            .upsert_drivers(&[DbDriver {
                driver_type: "mock".into(),
                name: "Mock Driver".into(),
                capabilities: vec!["readable".into(), "commandable".into()],
                commands: vec!["self_test".into(), "reset".into()],
            }])
            .await
            .unwrap();

        let toml_content = r#"
            [[devices]]
            id = "dev1"
            name = "Dev 1"
            [devices.driver]
            type = "mock"
        "#;

        svc.import_config(Request::new(ImportConfigRequest {
            toml_content: toml_content.into(),
        }))
        .await
        .unwrap();

        let drivers = svc.db.get_all_drivers().await.unwrap();
        let mock = drivers
            .iter()
            .find(|driver| driver.driver_type == "mock")
            .expect("mock driver row missing after import");

        assert_eq!(mock.name, "Mock Driver");
        assert_eq!(mock.capabilities, vec!["readable", "commandable"]);
        assert_eq!(mock.commands, vec!["self_test", "reset"]);
    }

    #[tokio::test]
    async fn test_get_db_info() {
        let svc = test_service().await;
        let resp = svc
            .get_db_info(Request::new(GetDbInfoRequest {}))
            .await
            .unwrap();
        let info = resp.into_inner();
        assert_eq!(info.engine, "mem");
        assert_eq!(info.namespace, "daq");
        assert!(info.healthy);
    }
}
