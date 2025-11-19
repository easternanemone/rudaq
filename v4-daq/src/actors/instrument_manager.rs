use crate::actors::data_publisher::{DataConsumer, DataPublisher, PublishBatch, Subscribe};
use crate::actors::hdf5_storage::{HDF5Storage, WriteBatch};
#[cfg(feature = "instrument_serial")]
use crate::actors::newport_1830c::Newport1830C;
use crate::config::{InstrumentDefinition, V4Config};
use crate::traits::power_meter::{PowerMeasurement, PowerMeter, PowerUnit, Wavelength};
use anyhow::{anyhow, Context as AnyhowContext, Result};
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;
use kameo::actor::{Actor, ActorID, ActorRef, WeakActorRef};
use kameo::error::{ActorStopReason, BoxSendError};
use kameo::message::{Context, Message};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::{hash_map::Entry, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Message: spawn a new instrument from a definition.
#[derive(Debug, Clone)]
pub struct SpawnInstrument {
    pub definition: InstrumentDefinition,
}

/// Message: kill a managed instrument.
#[derive(Debug, Clone)]
pub struct KillInstrument {
    pub id: String,
}

/// Message: route a command to a specific instrument.
#[derive(Debug, Clone)]
pub struct SendCommand {
    pub instrument_id: String,
    pub command: InstrumentCommand,
}

/// Message: retrieve the list of instruments and their status.
#[derive(Debug)]
pub struct GetInstrumentList;

/// Message: register a data consumer via the manager (delegated to DataPublisher).
pub struct SubscribeToData {
    pub subscriber: Arc<dyn DataConsumer>,
}

impl std::fmt::Debug for SubscribeToData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscribeToData")
            .field("subscriber", &"Arc<dyn DataConsumer>")
            .finish()
    }
}

/// Message: ingest Arrow record batches from instruments.
#[derive(Debug, Clone)]
pub struct InstrumentMeasurement {
    pub instrument_id: String,
    pub batch: RecordBatch,
}

#[derive(Debug, Clone)]
pub enum InstrumentCommand {
    PowerMeter(PowerMeterCommand),
    Vendor {
        command: String,
        payload: Option<JsonValue>,
    },
}

#[derive(Debug, Clone)]
pub enum PowerMeterCommand {
    ReadInstantaneous,
    SetWavelength(Wavelength),
    GetWavelength,
    SetUnit(PowerUnit),
    GetUnit,
}

#[derive(Debug, Clone)]
pub enum InstrumentCommandResponse {
    Ack,
    PowerMeasurement(PowerMeasurement),
    Wavelength(Wavelength),
    Unit(PowerUnit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentStatus {
    Starting,
    Running,
    Restarting,
    Failed,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct InstrumentInfo {
    pub id: String,
    pub instrument_type: String,
    pub status: InstrumentStatus,
    pub last_error: Option<String>,
    pub is_active: bool,
}

/// Arguments used to bootstrap the InstrumentManager actor.
pub struct InstrumentManagerArgs {
    pub config: Arc<V4Config>,
    pub data_publisher: ActorRef<DataPublisher>,
    pub storage: Option<ActorRef<HDF5Storage>>,
    pub catalog: InstrumentCatalog,
}

impl InstrumentManagerArgs {
    pub fn new(
        config: Arc<V4Config>,
        data_publisher: ActorRef<DataPublisher>,
        storage: Option<ActorRef<HDF5Storage>>,
    ) -> Self {
        Self {
            config,
            data_publisher,
            storage,
            catalog: InstrumentCatalog::with_builtin(),
        }
    }

    pub fn with_catalog(mut self, catalog: InstrumentCatalog) -> Self {
        self.catalog = catalog;
        self
    }
}

/// InstrumentManager supervises instrument actors and coordinates data flow.
pub struct InstrumentManager {
    config: Arc<V4Config>,
    data_publisher: ActorRef<DataPublisher>,
    storage: Option<ActorRef<HDF5Storage>>,
    catalog: InstrumentCatalog,
    registry: HashMap<String, InstrumentRecord>,
    actor_lookup: HashMap<ActorID, String>,
    actor_ref: ActorRef<InstrumentManager>,
}

struct InstrumentRecord {
    definition: InstrumentDefinition,
    driver: Option<DynInstrumentDriver>,
    status: InstrumentStatus,
    restart_policy: RestartPolicy,
    last_error: Option<String>,
}

impl InstrumentRecord {
    fn new(definition: InstrumentDefinition) -> Self {
        Self {
            definition,
            driver: None,
            status: InstrumentStatus::Starting,
            restart_policy: RestartPolicy::default(),
            last_error: None,
        }
    }

    fn info(&self) -> InstrumentInfo {
        InstrumentInfo {
            id: self.definition.id.clone(),
            instrument_type: self.definition.r#type.clone(),
            status: self.status,
            last_error: self.last_error.clone(),
            is_active: self.driver.is_some(),
        }
    }
}

#[derive(Clone)]
struct RestartPolicy {
    attempts: u32,
    max_restarts: u32,
    base_delay: Duration,
    current_delay: Duration,
    max_delay: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        let base_delay = Duration::from_millis(200);
        Self {
            attempts: 0,
            max_restarts: 5,
            base_delay,
            current_delay: base_delay,
            max_delay: Duration::from_secs(5),
        }
    }
}

impl RestartPolicy {
    fn next_backoff(&mut self) -> Option<Duration> {
        if self.attempts >= self.max_restarts {
            return None;
        }
        let delay = self.current_delay;
        self.attempts += 1;
        self.current_delay = (self.current_delay * 2).min(self.max_delay);
        Some(delay)
    }

    fn reset(&mut self) {
        self.attempts = 0;
        self.current_delay = self.base_delay;
    }
}

#[derive(Clone)]
pub struct InstrumentCatalog {
    builders: HashMap<String, InstrumentBuilder>,
}

type DynInstrumentDriver = Arc<dyn InstrumentDriver>;
type InstrumentBuilder =
    Arc<dyn Fn(&InstrumentDefinition) -> Result<DynInstrumentDriver> + Send + Sync>;

impl InstrumentCatalog {
    pub fn new() -> Self {
        Self {
            builders: HashMap::new(),
        }
    }

    pub fn with_builtin() -> Self {
        let mut catalog = Self::new();
        #[cfg(feature = "instrument_serial")]
        catalog.register_factory(
            "Newport1830C",
            Arc::new(|definition| build_newport_driver(definition)),
        );
        catalog
    }

    pub fn register_factory(
        &mut self,
        instrument_type: impl Into<String>,
        builder: InstrumentBuilder,
    ) {
        self.builders.insert(instrument_type.into(), builder);
    }

    pub fn build(&self, definition: &InstrumentDefinition) -> Result<DynInstrumentDriver> {
        let builder = self
            .builders
            .get(&definition.r#type)
            .ok_or_else(|| anyhow!("Unsupported instrument type: {}", definition.r#type))?;
        builder(definition)
    }
}

#[async_trait::async_trait]
pub trait InstrumentDriver: Send + Sync {
    fn id(&self) -> &str;
    fn instrument_type(&self) -> &str;
    fn actor_id(&self) -> ActorID;
    async fn link_to_manager(&self, manager_ref: &ActorRef<InstrumentManager>) -> Result<()>;
    async fn handle_command(&self, command: InstrumentCommand)
        -> Result<InstrumentCommandResponse>;
    async fn shutdown(&self) -> Result<()>;
}

#[async_trait::async_trait]
#[cfg(feature = "instrument_serial")]
impl InstrumentDriver for NewportDriver {
    fn id(&self) -> &str {
        &self.id
    }

    fn instrument_type(&self) -> &str {
        &self.instrument_type
    }

    fn actor_id(&self) -> ActorID {
        self.actor.id()
    }

    async fn link_to_manager(&self, manager_ref: &ActorRef<InstrumentManager>) -> Result<()> {
        self.actor.link(manager_ref).await;
        Ok(())
    }

    async fn handle_command(
        &self,
        command: InstrumentCommand,
    ) -> Result<InstrumentCommandResponse> {
        match command {
            InstrumentCommand::PowerMeter(cmd) => match cmd {
                PowerMeterCommand::ReadInstantaneous => {
                    let measurement = self.actor.clone().read_power().await?;
                    Ok(InstrumentCommandResponse::PowerMeasurement(measurement))
                }
                PowerMeterCommand::SetWavelength(wavelength) => {
                    self.actor.clone().set_wavelength(wavelength).await?;
                    Ok(InstrumentCommandResponse::Ack)
                }
                PowerMeterCommand::GetWavelength => {
                    let value = self.actor.clone().get_wavelength().await?;
                    Ok(InstrumentCommandResponse::Wavelength(value))
                }
                PowerMeterCommand::SetUnit(unit) => {
                    self.actor.clone().set_unit(unit).await?;
                    Ok(InstrumentCommandResponse::Ack)
                }
                PowerMeterCommand::GetUnit => {
                    let unit = self.actor.clone().get_unit().await?;
                    Ok(InstrumentCommandResponse::Unit(unit))
                }
            },
            InstrumentCommand::Vendor { command, .. } => Err(anyhow!(
                "Vendor command '{}' unsupported for Newport1830C",
                command
            )),
        }
    }

    async fn shutdown(&self) -> Result<()> {
        if self.actor.is_alive() {
            let _ = self.actor.stop_gracefully().await;
            self.actor.wait_for_shutdown().await;
        }
        Ok(())
    }
}

#[cfg(feature = "instrument_serial")]
struct NewportDriver {
    id: String,
    instrument_type: String,
    actor: ActorRef<Newport1830C>,
}

#[cfg(feature = "instrument_serial")]
impl NewportDriver {
    fn new(id: String, instrument_type: String, actor: ActorRef<Newport1830C>) -> Self {
        Self {
            id,
            instrument_type,
            actor,
        }
    }
}

#[cfg(feature = "instrument_serial")]
#[derive(Debug, Deserialize, Default)]
struct Newport1830CConfig {
    port: Option<String>,
    baud_rate: Option<u32>,
}

#[cfg(feature = "instrument_serial")]
fn build_newport_driver(definition: &InstrumentDefinition) -> Result<DynInstrumentDriver> {
    let config: Newport1830CConfig = definition.config.clone().try_into().unwrap_or_default();

    let actor_state = if let Some(port) = config.port {
        Newport1830C::with_serial(port, config.baud_rate.unwrap_or(9600))
    } else {
        Newport1830C::new()
    };

    let actor = Newport1830C::spawn(actor_state);
    Ok(Arc::new(NewportDriver::new(
        definition.id.clone(),
        definition.r#type.clone(),
        actor,
    )))
}

impl Actor for InstrumentManager {
    type Args = InstrumentManagerArgs;
    type Error = BoxSendError;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let mut manager = InstrumentManager {
            config: args.config,
            data_publisher: args.data_publisher,
            storage: args.storage,
            catalog: args.catalog,
            registry: HashMap::new(),
            actor_lookup: HashMap::new(),
            actor_ref: actor_ref.clone(),
        };

        manager.bootstrap().await;
        info!("InstrumentManager started");
        Ok(manager)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: ActorStopReason,
    ) -> Result<(), Self::Error> {
        info!("InstrumentManager stopped");
        Ok(())
    }

    async fn on_link_died(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        id: ActorID,
        reason: ActorStopReason,
    ) -> Result<std::ops::ControlFlow<ActorStopReason>, Self::Error> {
        if let Some(instrument_id) = self.actor_lookup.remove(&id) {
            let reason_text = format!("{:?}", reason);
            if let Some(record) = self.registry.get_mut(&instrument_id) {
                record.driver = None;
                record.last_error = Some(reason_text.clone());
                match reason {
                    ActorStopReason::Normal | ActorStopReason::Killed => {
                        record.status = InstrumentStatus::Stopped;
                        record.restart_policy.reset();
                    }
                    #[cfg(feature = "remote")]
                    ActorStopReason::PeerDisconnected => {
                        record.status = InstrumentStatus::Failed;
                    }
                    ActorStopReason::Panicked(_) | ActorStopReason::LinkDied { .. } => {
                        if let Some(delay) = record.restart_policy.next_backoff() {
                            record.status = InstrumentStatus::Restarting;
                            warn!(
                                instrument_id = %instrument_id,
                                ?reason,
                                "Instrument crashed; scheduling restart"
                            );
                            self.spawn_restart_task(instrument_id.clone(), delay);
                        } else {
                            record.status = InstrumentStatus::Failed;
                            error!(
                                instrument_id = %instrument_id,
                                "Instrument exceeded restart attempts"
                            );
                        }
                    }
                }
            }
        }

        Ok(std::ops::ControlFlow::Continue(()))
    }
}

impl InstrumentManager {
    async fn bootstrap(&mut self) {
        // Collect definitions first to avoid borrow checker issues
        let definitions: Vec<_> = self.config.enabled_instruments().into_iter().cloned().collect();

        for definition in definitions {
            if !self.registry.contains_key(&definition.id) {
                self.registry
                    .insert(definition.id.clone(), InstrumentRecord::new(definition.clone()));
            }
            if let Err(err) = self.start_instrument(&definition.id).await {
                if let Some(record) = self.registry.get_mut(&definition.id) {
                    record.status = InstrumentStatus::Failed;
                    record.last_error = Some(err.to_string());
                }
                error!(instrument_id = %definition.id, ?err, "Failed to start instrument");
            }
        }
    }

    fn register_definition(&mut self, definition: InstrumentDefinition) {
        match self.registry.entry(definition.id.clone()) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().definition = definition;
            }
            Entry::Vacant(vacant) => {
                vacant.insert(InstrumentRecord::new(definition));
            }
        }
    }

    async fn start_instrument(&mut self, instrument_id: &str) -> Result<()> {
        let definition = self
            .registry
            .get(instrument_id)
            .map(|record| record.definition.clone())
            .ok_or_else(|| anyhow!("Instrument '{}' not registered", instrument_id))?;

        if let Some(record) = self.registry.get(instrument_id) {
            if record.driver.is_some() {
                return Err(anyhow!("Instrument '{}' already running", instrument_id));
            }
        }

        let driver = self.catalog.build(&definition)?;
        driver
            .link_to_manager(&self.actor_ref)
            .await
            .context("failed to link instrument to manager")?;

        self.actor_lookup
            .insert(driver.actor_id(), definition.id.clone());

        if let Some(record) = self.registry.get_mut(instrument_id) {
            record.driver = Some(driver);
            record.status = InstrumentStatus::Running;
            record.last_error = None;
            record.restart_policy.reset();
        }

        info!("Instrument '{}' started", instrument_id);
        Ok(())
    }

    async fn stop_instrument(&mut self, id: &str) -> Result<()> {
        let record = self
            .registry
            .get_mut(id)
            .ok_or_else(|| anyhow!("Instrument '{}' not found", id))?;

        if let Some(driver) = record.driver.take() {
            self.actor_lookup.remove(&driver.actor_id());
            driver.shutdown().await?;
        }

        record.status = InstrumentStatus::Stopped;
        record.restart_policy.reset();
        record.last_error = None;
        Ok(())
    }

    fn spawn_restart_task(&self, instrument_id: String, delay: Duration) {
        let actor_ref = self.actor_ref.clone();
        tokio::spawn(async move {
            sleep(delay).await;
            if let Err(err) = actor_ref
                .tell(PerformRestart {
                    instrument_id: instrument_id.clone(),
                })
                .await
            {
                error!(?err, instrument_id = %instrument_id, "Failed to queue restart");
            }
        });
    }

    async fn forward_measurement(&self, instrument_id: &str, batch: RecordBatch) -> Result<()> {
        self.data_publisher
            .clone()
            .ask(PublishBatch {
                instrument_id: instrument_id.to_string(),
                batch: batch.clone(),
            })
            .await
            .context("failed to publish batch")?;

        if let Some(storage) = &self.storage {
            let serialized = serialize_record_batch(&batch)?;
            storage
                .clone()
                .ask(WriteBatch {
                    instrument_id: instrument_id.to_string(),
                    batch: Some(serialized),
                })
                .await
                .context("failed to persist batch")?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct PerformRestart {
    instrument_id: String,
}

impl Message<PerformRestart> for InstrumentManager {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: PerformRestart,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let Err(err) = self.start_instrument(&msg.instrument_id).await {
            if let Some(record) = self.registry.get_mut(&msg.instrument_id) {
                record.status = InstrumentStatus::Failed;
                record.last_error = Some(err.to_string());
            }
            return Err(err);
        }

        Ok(())
    }
}

impl Message<SpawnInstrument> for InstrumentManager {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: SpawnInstrument,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let id = msg.definition.id.clone();
        self.register_definition(msg.definition);
        self.start_instrument(&id).await
    }
}

impl Message<KillInstrument> for InstrumentManager {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: KillInstrument,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.stop_instrument(&msg.id).await
    }
}

impl Message<SendCommand> for InstrumentManager {
    type Reply = Result<InstrumentCommandResponse>;

    async fn handle(
        &mut self,
        msg: SendCommand,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let record = self
            .registry
            .get(&msg.instrument_id)
            .ok_or_else(|| anyhow!("Instrument '{}' not found", msg.instrument_id))?;

        let driver = record
            .driver
            .clone()
            .ok_or_else(|| anyhow!("Instrument '{}' is not running", msg.instrument_id))?;

        driver.handle_command(msg.command).await
    }
}

impl Message<GetInstrumentList> for InstrumentManager {
    type Reply = Result<Vec<InstrumentInfo>>;

    async fn handle(
        &mut self,
        _msg: GetInstrumentList,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        Ok(self.registry.values().map(|record| record.info()).collect())
    }
}

impl Message<SubscribeToData> for InstrumentManager {
    type Reply = Result<String>;

    async fn handle(
        &mut self,
        msg: SubscribeToData,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.data_publisher
            .clone()
            .ask(Subscribe {
                subscriber: msg.subscriber,
            })
            .await
            .map_err(|e| anyhow::anyhow!("failed to subscribe to data: {}", e))
    }
}

impl Message<InstrumentMeasurement> for InstrumentManager {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: InstrumentMeasurement,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if !self.registry.contains_key(&msg.instrument_id) {
            return Err(anyhow!("Instrument '{}' not registered", msg.instrument_id));
        }

        self.forward_measurement(&msg.instrument_id, msg.batch)
            .await
    }
}

fn serialize_record_batch(batch: &RecordBatch) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    {
        let schema = batch.schema();
        let mut writer = StreamWriter::try_new(&mut buffer, &schema)?;
        writer.write(batch)?;
        writer.finish()?;
    }
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, StringArray, TimestampNanosecondArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

    #[tokio::test]
    #[cfg(feature = "instrument_serial")]
    async fn test_spawns_from_config() -> Result<()> {
        let config = test_config(vec![newport_definition("pm1")]);
        let publisher = DataPublisher::spawn(DataPublisher::new());
        let args = InstrumentManagerArgs::new(config, publisher.clone(), None);
        let manager = InstrumentManager::spawn(args);
        manager.wait_for_startup().await;

        let instruments = manager.clone().ask(GetInstrumentList).await?;
        assert_eq!(instruments.len(), 1);
        assert_eq!(instruments[0].status, InstrumentStatus::Running);

        manager.kill();
        manager.wait_for_shutdown().await;
        publisher.kill();
        publisher.wait_for_shutdown().await;
        Ok(())
    }

    #[tokio::test]
    #[cfg(feature = "instrument_serial")]
    async fn test_measurement_flow() -> Result<()> {
        let config = test_config(vec![newport_definition("pm1")]);
        let publisher = DataPublisher::spawn(DataPublisher::new());
        let args = InstrumentManagerArgs::new(config, publisher.clone(), None);
        let manager = InstrumentManager::spawn(args);
        manager.wait_for_startup().await;

        let consumer = Arc::new(TestConsumer::default());
        manager
            .clone()
            .ask(SubscribeToData {
                subscriber: consumer.clone(),
            })
            .await?;

        manager
            .clone()
            .ask(InstrumentMeasurement {
                instrument_id: "pm1".to_string(),
                batch: sample_batch(),
            })
            .await?;

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(consumer.total_received().await, 1);

        manager.kill();
        manager.wait_for_shutdown().await;
        publisher.kill();
        publisher.wait_for_shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn test_restart_on_failure() -> Result<()> {
        let mut catalog = InstrumentCatalog::with_builtin();
        catalog.register_factory(
            "MockPanicking",
            Arc::new(|definition| build_mock_driver(definition)),
        );

        let config = test_config(vec![mock_definition("mock1")]);
        let publisher = DataPublisher::spawn(DataPublisher::new());
        let args =
            InstrumentManagerArgs::new(config, publisher.clone(), None).with_catalog(catalog);
        let manager = InstrumentManager::spawn(args);
        manager.wait_for_startup().await;

        manager
            .clone()
            .ask(SendCommand {
                instrument_id: "mock1".to_string(),
                command: InstrumentCommand::Vendor {
                    command: "crash".to_string(),
                    payload: None,
                },
            })
            .await
            .ok();

        tokio::time::sleep(Duration::from_millis(400)).await;
        let info = manager.clone().ask(GetInstrumentList).await?;
        assert_eq!(info[0].status, InstrumentStatus::Running);

        manager.kill();
        manager.wait_for_shutdown().await;
        publisher.kill();
        publisher.wait_for_shutdown().await;
        Ok(())
    }

    fn test_config(instruments: Vec<InstrumentDefinition>) -> Arc<V4Config> {
        Arc::new(V4Config {
            application: crate::config::ApplicationConfig {
                name: "test".to_string(),
                log_level: "info".to_string(),
                data_dir: None,
            },
            actors: crate::config::ActorConfig {
                default_mailbox_capacity: 100,
                spawn_timeout_ms: 1000,
                shutdown_timeout_ms: 1000,
            },
            storage: crate::config::StorageConfig {
                default_backend: "arrow".to_string(),
                output_dir: std::path::PathBuf::from("./tmp"),
                compression_level: 6,
                auto_flush_interval_secs: 0,
            },
            instruments,
        })
    }

    #[cfg(feature = "instrument_serial")]
    fn newport_definition(id: &str) -> InstrumentDefinition {
        use crate::config::InstrumentSpecificConfig;
        InstrumentDefinition {
            id: id.to_string(),
            r#type: "Newport1830C".to_string(),
            enabled: true,
            config: InstrumentSpecificConfig::default(),
        }
    }

    fn mock_definition(id: &str) -> InstrumentDefinition {
        use crate::config::InstrumentSpecificConfig;
        InstrumentDefinition {
            id: id.to_string(),
            r#type: "MockPanicking".to_string(),
            enabled: true,
            config: InstrumentSpecificConfig::default(),
        }
    }

    #[derive(Default)]
    struct TestConsumer {
        inner: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl DataConsumer for TestConsumer {
        async fn handle_batch(&self, _batch: RecordBatch, instrument_id: String) -> Result<()> {
            let mut guard = self.inner.lock().await;
            guard.push(instrument_id);
            Ok(())
        }
    }

    impl TestConsumer {
        async fn total_received(&self) -> usize {
            self.inner.lock().await.len()
        }
    }

    fn sample_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new(
                "timestamp",
                DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                false,
            ),
            Field::new("power", DataType::Float64, false),
            Field::new("unit", DataType::Utf8, false),
        ]));

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampNanosecondArray::from(vec![1])),
                Arc::new(Float64Array::from(vec![0.1])),
                Arc::new(StringArray::from(vec![Some("Watts")])),
            ],
        )
        .unwrap()
    }

    fn build_mock_driver(definition: &InstrumentDefinition) -> Result<DynInstrumentDriver> {
        let actor = MockInstrument::spawn(MockInstrument::default());
        Ok(Arc::new(MockDriver {
            id: definition.id.clone(),
            actor,
        }))
    }

    #[derive(Default)]
    struct MockInstrument;

    #[async_trait::async_trait]
    impl Actor for MockInstrument {
        type Args = Self;
        type Error = BoxSendError;

        async fn on_start(
            args: Self::Args,
            _actor_ref: ActorRef<Self>,
        ) -> Result<Self, Self::Error> {
            Ok(args)
        }
    }

    #[derive(Debug)]
    struct Crash;

    impl Message<Crash> for MockInstrument {
        type Reply = Result<()>;

        async fn handle(
            &mut self,
            _msg: Crash,
            _ctx: &mut Context<Self, Self::Reply>,
        ) -> Self::Reply {
            panic!("intentional crash");
        }
    }

    struct MockDriver {
        id: String,
        actor: ActorRef<MockInstrument>,
    }

    #[async_trait::async_trait]
    impl InstrumentDriver for MockDriver {
        fn id(&self) -> &str {
            &self.id
        }

        fn instrument_type(&self) -> &str {
            "MockPanicking"
        }

        fn actor_id(&self) -> ActorID {
            self.actor.id()
        }

        async fn link_to_manager(&self, manager_ref: &ActorRef<InstrumentManager>) -> Result<()> {
            self.actor.link(manager_ref).await;
            Ok(())
        }

        async fn handle_command(
            &self,
            command: InstrumentCommand,
        ) -> Result<InstrumentCommandResponse> {
            if let InstrumentCommand::Vendor { command, .. } = command {
                if command == "crash" {
                    let _ = self.actor.tell(Crash).await;
                }
            }
            Ok(InstrumentCommandResponse::Ack)
        }

        async fn shutdown(&self) -> Result<()> {
            if self.actor.is_alive() {
                self.actor.kill();
            }
            Ok(())
        }
    }
}
