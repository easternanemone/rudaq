//! V3 Instrument Manager - Orchestration layer for V3 instruments
//!
//! This module provides the missing orchestration layer identified in the Phase 2
//! completion analysis. All reference frameworks (DynExp, PyMoDAQ, ScopeFoundry, Qudi)
//! have a manager/orchestrator that coordinates instrument lifecycle, configuration,
//! and data flow. This is that layer for rust-daq V3.
//!
//! ## Responsibilities
//!
//! 1. **Lifecycle Management**: Owns V3 instrument trait objects, spawns their tasks,
//!    monitors health, and orchestrates graceful shutdown
//! 2. **Configuration**: Reads `[[instruments_v3]]` from TOML, uses factory pattern
//!    to instantiate instruments
//! 3. **Data Flow**: Subscribes to measurement channels, bridges to application
//! 4. **Parameter Discovery**: Exposes unified interface for parameter control
//!
//! ## Reference Pattern
//!
//! Based on DynExp's Module/ModuleInstance/Manager architecture:
//! - `Instrument` trait = DynExp Module (configuration template)
//! - `InstrumentHandle` = DynExp ModuleInstance (runtime state)
//! - `InstrumentManagerV3` = DynExp Manager (orchestrator)

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::config::InstrumentConfigV3;
use crate::core::ImageData as CoreImageData;
use crate::core_v3::{Command, ImageMetadata, Instrument, Measurement as V3Measurement, Response};
use crate::measurement::DataDistributor;
use daq_core::Measurement as V1Measurement;
use daq_core::SpectrumData as V1SpectrumData;
use serde_json::Value as JsonValue;

/// Factory function signature for creating V3 instruments from configuration
///
/// Takes instrument ID and configuration (as JSON for flexibility), returns
/// a boxed trait object. This enables runtime polymorphism and configuration-based
/// instrument instantiation.
pub type InstrumentFactory = fn(&str, &serde_json::Value) -> Result<Box<dyn Instrument>>;

/// Runtime handle for an active V3 instrument
///
/// Owns the shutdown channel and task handle, enabling lifecycle management
/// without holding the instrument itself (which runs in its own task).
struct InstrumentHandle {
    /// Oneshot channel to signal shutdown
    shutdown_tx: Option<oneshot::Sender<()>>,

    /// Join handle for the instrument's runtime task
    task_handle: JoinHandle<Result<()>>,

    /// Broadcast receiver for measurement data
    measurement_rx: broadcast::Receiver<V3Measurement>,

    /// Command channel sender
    command_tx: mpsc::Sender<CommandMessage>,
}

/// Message sent over the per-instrument command channel
struct CommandMessage {
    command: Command,
    response_tx: oneshot::Sender<Result<Response>>,
}

/// V3 Instrument Manager - The orchestration layer
///
/// Coordinates V3 instrument lifecycle, configuration, and data flow. This is the
/// missing architectural tier identified in Phase 2 analysis - all reference
/// frameworks have equivalent (DynExp ModuleManager, PyMODAQ PluginManager, etc.)
pub struct InstrumentManagerV3 {
    /// Registry mapping instrument type names to factory functions
    ///
    /// Example: "Newport1830CV3" -> Newport1830CV3::from_config
    factories: HashMap<String, InstrumentFactory>,

    /// Active instruments keyed by their configuration ID
    ///
    /// Example: "power_meter_1" -> InstrumentHandle
    active_instruments: Arc<Mutex<HashMap<String, InstrumentHandle>>>,

    /// Data distributor for aggregated measurements (V3 → V1 bridge)
    ///
    /// Uses non-blocking DataDistributor to forward V3 Measurement to V1 GUI/Storage
    /// during Phase 3 migration, leveraging daq-87/daq-88 backpressure fixes
    data_distributor: Option<Arc<DataDistributor<Arc<V1Measurement>>>>,

    /// Forwarder task handles for graceful shutdown
    ///
    /// Tracks spawned data bridge tasks so they can be cancelled during shutdown
    forwarder_handles: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl InstrumentManagerV3 {
    /// Create a new instrument manager with empty factory registry
    ///
    /// Call `register_factory()` to add instrument types before loading from config.
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            active_instruments: Arc::new(Mutex::new(HashMap::new())),
            data_distributor: None,
            forwarder_handles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a factory function for an instrument type
    ///
    /// # Example
    ///
    /// ```ignore
    /// manager.register_factory("MockPowerMeterV3", MockPowerMeterV3::from_config);
    /// manager.register_factory("Newport1830CV3", Newport1830CV3::from_config);
    /// ```
    pub fn register_factory(&mut self, type_name: impl Into<String>, factory: InstrumentFactory) {
        self.factories.insert(type_name.into(), factory);
    }

    /// Set the data distributor for V3 → V1 data flow
    ///
    /// During Phase 3, V3 measurements are bridged to V1 DataDistributor
    /// for backward compatibility with existing DaqApp/GUI/Storage.
    /// Uses non-blocking broadcast() to prevent slow subscribers from blocking data flow.
    pub fn set_data_distributor(&mut self, distributor: Arc<DataDistributor<Arc<V1Measurement>>>) {
        self.data_distributor = Some(distributor);
    }

    /// Load instruments from V3 configuration
    ///
    /// Reads `[[instruments_v3]]` sections, instantiates using factory pattern,
    /// initializes each instrument, and spawns runtime tasks.
    ///
    /// # Configuration Format
    ///
    /// ```toml
    /// [[instruments_v3]]
    /// id = "power_meter_1"
    /// type = "Newport1830CV3"
    /// port = "/dev/ttyUSB0"
    /// [instruments_v3.params]
    /// wavelength_nm = 532.0
    /// range = "auto"
    /// ```
    pub async fn load_from_config(
        &mut self,
        instruments_config: &[InstrumentConfigV3],
    ) -> Result<()> {
        for cfg in instruments_config {
            self.spawn_instrument(cfg)
                .await
                .with_context(|| format!("Failed to load instrument '{}'", cfg.id))?;
        }

        Ok(())
    }

    /// Spawn a single instrument from configuration
    ///
    /// 1. Lookup factory by type name
    /// 2. Instantiate instrument
    /// 3. Initialize (connect, configure)
    /// 4. Spawn runtime task
    /// 5. Setup data bridge
    async fn spawn_instrument(&mut self, cfg: &InstrumentConfigV3) -> Result<()> {
        // Lookup factory
        let factory = self
            .factories
            .get(&cfg.type_name)
            .ok_or_else(|| anyhow!("Unknown V3 instrument type: '{}'", cfg.type_name))?;

        // Instantiate
        let mut instrument = factory(&cfg.id, &cfg.settings)
            .with_context(|| format!("Factory failed for type '{}'", cfg.type_name))?;

        // Initialize
        instrument
            .initialize()
            .await
            .with_context(|| format!("Initialization failed for '{}'", cfg.id))?;

        // Get measurement channel before moving instrument
        let measurement_rx = instrument.data_channel();

        // Create shutdown and command channels
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let (command_tx, mut command_rx) = mpsc::channel::<CommandMessage>(32); // Command buffer

        // Spawn runtime task
        let task_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Wait for shutdown signal
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    // Process incoming commands
                    Some(CommandMessage { command, response_tx }) = command_rx.recv() => {
                        let exec_result = instrument.execute(command).await;

                        if let Err(ref err) = exec_result {
                            tracing::error!(
                                instrument_id = instrument.id(),
                                error = ?err,
                                "Instrument command execution failed"
                            );
                        }

                        if let Err(e) = response_tx.send(exec_result) {
                            tracing::warn!(
                                instrument_id = instrument.id(),
                                error = ?e,
                                "Failed to deliver command response"
                            );
                        }
                    }
                }
            }

            // Graceful shutdown
            instrument.shutdown().await?;

            Ok(())
        });

        // Setup data bridge if data distributor configured
        if let Some(distributor) = &self.data_distributor {
            let forwarder_handle = Self::spawn_data_bridge(
                cfg.id.clone(),
                measurement_rx.resubscribe(),
                distributor.clone(),
            );

            // Store forwarder handle for shutdown
            self.forwarder_handles
                .lock()
                .await
                .insert(cfg.id.clone(), forwarder_handle);
        }

        // Store handle
        let handle = InstrumentHandle {
            shutdown_tx: Some(shutdown_tx),
            task_handle,
            measurement_rx,
            command_tx,
        };

        self.active_instruments
            .lock()
            .await
            .insert(cfg.id.clone(), handle);

        Ok(())
    }

    /// Spawn data bridge task for V3 → V1 compatibility
    ///
    /// Subscribes to V3 measurement channel and forwards to DataDistributor.
    /// Uses non-blocking broadcast() to prevent slow subscribers from blocking data flow.
    /// Currently only supports Measurement::Scalar; logs warnings for Image/Spectrum.
    fn spawn_data_bridge(
        instrument_id: String,
        mut v3_rx: broadcast::Receiver<V3Measurement>,
        distributor: Arc<DataDistributor<Arc<V1Measurement>>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match v3_rx.recv().await {
                    Ok(measurement) => {
                        // Convert V3 Measurement to V1 Measurement for bridge
                        // Currently only Scalar is supported in Phase 3
                        let v1_measurement = match &measurement {
                            V3Measurement::Scalar {
                                name,
                                value,
                                unit,
                                timestamp,
                            } => {
                                let data_point = daq_core::DataPoint {
                                    channel: name.clone(),
                                    value: *value,
                                    timestamp: *timestamp,
                                    unit: unit.clone(),
                                };
                                Some(V1Measurement::Scalar(data_point))
                            }
                            V3Measurement::Image {
                                name,
                                width,
                                height,
                                buffer,
                                unit,
                                metadata,
                                timestamp,
                            } => {
                                let width_usize = (*width).max(1) as usize;
                                let height_usize = (*height).max(1) as usize;
                                let expected_len = width_usize.saturating_mul(height_usize);

                                if buffer.len() != expected_len {
                                    tracing::warn!(
                                        instrument_id = instrument_id,
                                        channel = name,
                                        expected_len,
                                        actual_len = buffer.len(),
                                        "Image buffer length mismatch during V3→V1 bridge"
                                    );
                                }

                                let metadata_value = Self::image_metadata_to_value(metadata);

                                let image_data = CoreImageData {
                                    timestamp: *timestamp,
                                    channel: name.clone(),
                                    width: width_usize,
                                    height: height_usize,
                                    pixels: buffer.clone(),
                                    unit: unit.clone(),
                                    metadata: metadata_value,
                                };

                                Some(V1Measurement::Image(image_data.into()))
                            }
                            V3Measurement::Spectrum {
                                name,
                                frequencies,
                                amplitudes,
                                frequency_unit,
                                amplitude_unit,
                                metadata,
                                timestamp,
                            } => {
                                if frequencies.len() != amplitudes.len() {
                                    tracing::warn!(
                                        instrument_id = instrument_id,
                                        channel = name,
                                        freq_len = frequencies.len(),
                                        amp_len = amplitudes.len(),
                                        "Spectrum measurement length mismatch during V3→V1 bridge"
                                    );
                                    None
                                } else {
                                    let spectrum = V1SpectrumData {
                                        timestamp: *timestamp,
                                        channel: name.clone(),
                                        wavelengths: frequencies.clone(),
                                        intensities: amplitudes.clone(),
                                        unit_x: frequency_unit
                                            .clone()
                                            .unwrap_or_else(|| "Hz".to_string()),
                                        unit_y: amplitude_unit
                                            .clone()
                                            .unwrap_or_else(|| "arb".to_string()),
                                        metadata: metadata.clone(),
                                    };

                                    Some(V1Measurement::Spectrum(spectrum))
                                }
                            }
                            V3Measurement::Vector { .. } => {
                                tracing::warn!(
                                    instrument_id = instrument_id,
                                    "Vector measurement not yet supported by V3→V1 bridge"
                                );
                                None
                            }
                        };

                        // Forward converted measurement if successful
                        if let Some(v1_msg) = v1_measurement {
                            if let Err(e) = distributor.broadcast(Arc::new(v1_msg)).await {
                                tracing::error!(
                                    "Data bridge broadcast failed for '{}': {}",
                                    instrument_id,
                                    e
                                );
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            "Data bridge for '{}' lagged by {} measurements",
                            instrument_id,
                            n
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("Measurement channel closed for '{}'", instrument_id);
                        break;
                    }
                }
            }
        })
    }

    fn image_metadata_to_value(metadata: &ImageMetadata) -> Option<JsonValue> {
        if metadata.exposure_ms.is_none()
            && metadata.gain.is_none()
            && metadata.binning.is_none()
            && metadata.temperature_c.is_none()
        {
            None
        } else {
            serde_json::to_value(metadata).ok()
        }
    }

    /// Execute a command on a specific instrument
    ///
    /// This is the primary control interface for V3 instruments. Commands are
    /// sent directly (no actor model overhead) and responses are awaited.
    pub async fn execute_command(&self, instrument_id: &str, command: Command) -> Result<Response> {
        let (response_tx, response_rx) = oneshot::channel();

        let sender = {
            let instruments = self.active_instruments.lock().await;
            if let Some(handle) = instruments.get(instrument_id) {
                handle.command_tx.clone()
            } else {
                return Err(anyhow!("Instrument '{}' not found", instrument_id));
            }
        };

        sender
            .send(CommandMessage {
                command,
                response_tx,
            })
            .await
            .with_context(|| {
                format!("Command channel closed for instrument '{}'", instrument_id)
            })?;

        match response_rx.await {
            Ok(result) => result,
            Err(_) => Err(anyhow!(
                "Instrument '{}' dropped command response channel",
                instrument_id
            )),
        }
    }

    /// Get measurement receiver for a specific instrument
    ///
    /// Returns a broadcast receiver that can subscribe to the instrument's
    /// measurement stream. Used by GUI, storage writers, and processors.
    pub async fn subscribe_measurements(
        &self,
        instrument_id: &str,
    ) -> Result<broadcast::Receiver<V3Measurement>> {
        let instruments = self.active_instruments.lock().await;
        let handle = instruments
            .get(instrument_id)
            .ok_or_else(|| anyhow!("Instrument '{}' not found", instrument_id))?;

        Ok(handle.measurement_rx.resubscribe())
    }

    /// List all active V3 instruments
    pub async fn list_instruments(&self) -> Vec<String> {
        self.active_instruments
            .lock()
            .await
            .keys()
            .cloned()
            .collect()
    }

    /// Shutdown all instruments gracefully
    ///
    /// Sends shutdown signal to each instrument and awaits task completion
    /// with 5-second timeout per instrument (matches V1 behavior).
    pub async fn shutdown_all(&mut self) -> Result<()> {
        // Cancel forwarder tasks first
        {
            let mut handles = self.forwarder_handles.lock().await;
            for (id, handle) in handles.drain() {
                handle.abort();
                tracing::debug!("Cancelled forwarder task for '{}'", id);
            }
        }

        // Shutdown instruments
        let mut instruments = self.active_instruments.lock().await;
        let ids: Vec<String> = instruments.keys().cloned().collect();

        for id in ids {
            if let Some(mut handle) = instruments.remove(&id) {
                // Send shutdown signal
                if let Some(shutdown_tx) = handle.shutdown_tx.take() {
                    let _ = shutdown_tx.send(());
                }

                // Await task completion with timeout
                let timeout_duration = std::time::Duration::from_secs(5);
                let task_handle = handle.task_handle;
                tokio::pin!(task_handle);

                match tokio::time::timeout(timeout_duration, &mut task_handle).await {
                    Ok(join_result) => match join_result {
                        Ok(Ok(())) => {
                            tracing::info!("Instrument '{}' shutdown successfully", id);
                        }
                        Ok(Err(e)) => {
                            tracing::error!("Instrument '{}' shutdown error: {}", id, e);
                        }
                        Err(e) => {
                            tracing::error!("Instrument '{}' task panicked: {}", id, e);
                        }
                    },
                    Err(_) => {
                        tracing::warn!("Instrument '{}' shutdown timeout (5s), aborting", id);
                        task_handle.as_ref().get_ref().abort();
                        if let Err(e) = task_handle.await {
                            tracing::debug!(
                                "Instrument '{}' task aborted with join error: {}",
                                id,
                                e
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for InstrumentManagerV3 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_v3::{InstrumentState, ParameterBase, PixelBuffer};
    use crate::instruments_v2::MockPowerMeterV3;
    use chrono::Utc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};
    use tokio::time::{timeout, Duration};

    // Mock instrument for testing
    struct MockInstrumentV3 {
        id: String,
        tx: broadcast::Sender<V3Measurement>,
        params: HashMap<String, Box<dyn ParameterBase>>,
    }

    impl MockInstrumentV3 {
        fn from_config(id: &str, _cfg: &serde_json::Value) -> Result<Box<dyn Instrument>> {
            let (tx, _rx) = broadcast::channel(16);
            Ok(Box::new(Self {
                id: id.to_string(),
                tx,
                params: HashMap::new(),
            }))
        }
    }

    #[async_trait::async_trait]
    impl Instrument for MockInstrumentV3 {
        fn id(&self) -> &str {
            &self.id
        }

        async fn initialize(&mut self) -> Result<()> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }

        fn data_channel(&self) -> broadcast::Receiver<V3Measurement> {
            self.tx.subscribe()
        }

        async fn execute(&mut self, _cmd: Command) -> Result<Response> {
            Ok(Response::Ok)
        }

        fn parameters(&self) -> &HashMap<String, Box<dyn ParameterBase>> {
            &self.params
        }

        fn parameters_mut(&mut self) -> &mut HashMap<String, Box<dyn ParameterBase>> {
            &mut self.params
        }

        fn state(&self) -> InstrumentState {
            InstrumentState::Idle
        }
    }

    #[tokio::test]
    async fn test_instrument_manager_registration() {
        let mut manager = InstrumentManagerV3::new();
        manager.register_factory("MockInstrumentV3", MockInstrumentV3::from_config);

        assert!(manager.factories.contains_key("MockInstrumentV3"));
    }

    #[tokio::test]
    async fn test_instrument_manager_spawn() {
        let mut manager = InstrumentManagerV3::new();
        manager.register_factory("MockInstrumentV3", MockInstrumentV3::from_config);

        let cfg = InstrumentConfigV3 {
            id: "test_instrument".to_string(),
            type_name: "MockInstrumentV3".to_string(),
            settings: serde_json::json!({}),
        };

        manager.spawn_instrument(&cfg).await.unwrap();

        let instruments = manager.list_instruments().await;
        assert!(instruments.contains(&"test_instrument".to_string()));
    }

    #[tokio::test]
    async fn test_instrument_manager_shutdown() {
        let mut manager = InstrumentManagerV3::new();
        manager.register_factory("MockInstrumentV3", MockInstrumentV3::from_config);

        let cfg = InstrumentConfigV3 {
            id: "test_instrument".to_string(),
            type_name: "MockInstrumentV3".to_string(),
            settings: serde_json::json!({}),
        };

        manager.spawn_instrument(&cfg).await.unwrap();
        manager.shutdown_all().await.unwrap();

        let instruments = manager.list_instruments().await;
        assert!(instruments.is_empty());
    }

    #[tokio::test]
    async fn test_mock_power_meter_integration() {
        let mut manager = InstrumentManagerV3::new();
        manager.register_factory("MockPowerMeterV3", MockPowerMeterV3::from_config);

        let cfg = InstrumentConfigV3 {
            id: "power_meter_test".to_string(),
            type_name: "MockPowerMeterV3".to_string(),
            settings: serde_json::json!({
                "sampling_rate": 10.0,
                "wavelength_nm": 532.0
            }),
        };

        manager.spawn_instrument(&cfg).await.unwrap();

        let instruments = manager.list_instruments().await;
        assert!(instruments.contains(&"power_meter_test".to_string()));

        // Verify we can subscribe to measurements
        let mut rx = manager
            .subscribe_measurements("power_meter_test")
            .await
            .unwrap();

        // Receive at least one measurement to verify data flow
        tokio::select! {
            result = rx.recv() => {
                let measurement = result.unwrap();
                assert_eq!(measurement.name(), "power_meter_test_power");
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                panic!("No measurement received within timeout");
            }
        }

        manager.shutdown_all().await.unwrap();

        let instruments = manager.list_instruments().await;
        assert!(instruments.is_empty());
    }

    #[tokio::test]
    async fn test_data_bridge_forward_image_measurement() {
        let distributor = Arc::new(DataDistributor::new(8));
        let mut subscriber = distributor.subscribe("listener").await;
        let (tx, rx) = broadcast::channel(8);

        let forwarder =
            InstrumentManagerV3::spawn_data_bridge("camera1".to_string(), rx, distributor.clone());

        let measurement = V3Measurement::Image {
            name: "camera1_frame".to_string(),
            width: 2,
            height: 2,
            buffer: PixelBuffer::U16(vec![10, 20, 30, 40]),
            unit: "counts".to_string(),
            metadata: ImageMetadata {
                exposure_ms: Some(5.0),
                gain: Some(2.0),
                binning: None,
                temperature_c: None,
            },
            timestamp: Utc::now(),
        };

        tx.send(measurement).unwrap();

        let received = timeout(Duration::from_millis(200), subscriber.recv())
            .await
            .expect("subscriber should receive image measurement")
            .expect("channel should remain open");

        match received.as_ref() {
            V1Measurement::Image(image) => {
                assert_eq!(image.channel, "camera1_frame");
                assert_eq!(image.width, 2);
                assert_eq!(image.height, 2);
                assert_eq!(image.unit, "counts");
            }
            other => panic!("Expected V1 image measurement, got {other:?}"),
        }

        forwarder.abort();
    }

    #[tokio::test]
    async fn test_data_bridge_forward_spectrum_measurement() {
        let distributor = Arc::new(DataDistributor::new(8));
        let mut subscriber = distributor.subscribe("listener").await;
        let (tx, rx) = broadcast::channel(8);

        let forwarder = InstrumentManagerV3::spawn_data_bridge(
            "spectrum1".to_string(),
            rx,
            distributor.clone(),
        );

        let measurement = V3Measurement::Spectrum {
            name: "spectrum1_fft".to_string(),
            frequencies: vec![0.0, 100.0, 200.0],
            amplitudes: vec![-10.0, -3.0, -6.0],
            frequency_unit: Some("Hz".to_string()),
            amplitude_unit: Some("dB".to_string()),
            metadata: Some(serde_json::json!({ "window_size": 256 })),
            timestamp: Utc::now(),
        };

        tx.send(measurement).unwrap();

        let received = timeout(Duration::from_millis(200), subscriber.recv())
            .await
            .expect("subscriber should receive spectrum measurement")
            .expect("channel should remain open");

        match received.as_ref() {
            V1Measurement::Spectrum(spec) => {
                assert_eq!(spec.channel, "spectrum1_fft");
                assert_eq!(spec.wavelengths.len(), 3);
                assert_eq!(spec.unit_x, "Hz");
                assert_eq!(spec.unit_y, "dB");
            }
            other => panic!("Expected V1 spectrum measurement, got {other:?}"),
        }

        forwarder.abort();
    }

    static BLOCKING_DROP_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

    struct BlockingShutdownInstrument {
        id: String,
        tx: broadcast::Sender<V3Measurement>,
        params: HashMap<String, Box<dyn ParameterBase>>,
        state: InstrumentState,
        drop_flag: Arc<AtomicBool>,
    }

    impl BlockingShutdownInstrument {
        fn new(id: &str, drop_flag: Arc<AtomicBool>) -> Self {
            let (tx, _rx) = broadcast::channel(1);
            Self {
                id: id.to_string(),
                tx,
                params: HashMap::new(),
                state: InstrumentState::Idle,
                drop_flag,
            }
        }

        fn factory(id: &str, cfg: &serde_json::Value) -> Result<Box<dyn Instrument>> {
            let flag = BLOCKING_DROP_FLAG
                .get()
                .expect("drop flag not initialized")
                .clone();
            let _ = cfg; // unused
            Ok(Box::new(Self::new(id, flag)))
        }
    }

    impl Drop for BlockingShutdownInstrument {
        fn drop(&mut self) {
            self.drop_flag.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl Instrument for BlockingShutdownInstrument {
        fn id(&self) -> &str {
            &self.id
        }

        fn state(&self) -> InstrumentState {
            self.state
        }

        async fn initialize(&mut self) -> Result<()> {
            self.state = InstrumentState::Idle;
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<()> {
            futures::future::pending::<()>().await;
            #[allow(unreachable_code)]
            Ok(())
        }

        fn data_channel(&self) -> broadcast::Receiver<V3Measurement> {
            self.tx.subscribe()
        }

        async fn execute(&mut self, _cmd: Command) -> Result<Response> {
            Ok(Response::Ok)
        }

        fn parameters(&self) -> &HashMap<String, Box<dyn ParameterBase>> {
            &self.params
        }

        fn parameters_mut(&mut self) -> &mut HashMap<String, Box<dyn ParameterBase>> {
            &mut self.params
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_shutdown_all_aborts_blocking_instrument() {
        let drop_flag = BLOCKING_DROP_FLAG.get_or_init(|| Arc::new(AtomicBool::new(false)));
        drop_flag.store(false, Ordering::SeqCst);
        let drop_flag = drop_flag.clone();

        let mut manager = InstrumentManagerV3::new();
        manager.register_factory("BlockingInstrument", BlockingShutdownInstrument::factory);

        let cfg = InstrumentConfigV3 {
            id: "blocking".to_string(),
            type_name: "BlockingInstrument".to_string(),
            settings: serde_json::json!({}),
        };

        manager.spawn_instrument(&cfg).await.unwrap();

        let mut shutdown_fut = manager.shutdown_all();
        tokio::pin!(shutdown_fut);
        tokio::time::advance(Duration::from_secs(6)).await;
        shutdown_fut.as_mut().await.unwrap();

        assert!(
            drop_flag.load(Ordering::SeqCst),
            "Blocking instrument should be aborted and dropped"
        );
    }
}
