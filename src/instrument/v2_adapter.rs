//! V2InstrumentAdapter - Bridge between V1 and V2 instrument APIs
//!
//! This adapter wraps V2 instruments (which use `daq_core::Instrument` trait)
//! to work with the V1 InstrumentRegistry (which expects `core::Instrument` trait).
//!
//! ## Architecture
//!
//! ```text
//! V2 Instrument (daq_core::Instrument)
//!       ↓
//! V2InstrumentAdapter (implements core::Instrument)
//!       ↓
//! V1 InstrumentRegistry
//!       ↓
//! DaqManagerActor
//! ```
//!
//! ## Key Responsibilities
//!
//! 1. **Trait Translation**: Implements V1 `core::Instrument` by delegating to V2 `daq_core::Instrument`
//! 2. **Measurement Conversion**: Converts V2 `Arc<Measurement>` to V1 `InstrumentMeasurement` broadcasts
//! 3. **Command Forwarding**: Translates V1 `InstrumentCommand` to V2 `InstrumentCommand`
//! 4. **State Bridging**: Maps V2 `InstrumentState` to V1 connection lifecycle
//! 5. **Lifecycle Management**: Spawns background task to forward measurement stream
//!
//! ## Usage
//!
//! ```rust
//! use rust_daq::instrument::v2_adapter::V2InstrumentAdapter;
//! use rust_daq::instruments_v2::pvcam::PVCAMInstrumentV2;
//!
//! // Wrap V2 instrument for V1 registry
//! let v2_instrument = PVCAMInstrumentV2::new("camera1".to_string());
//! let adapted = V2InstrumentAdapter::new(v2_instrument);
//!
//! // Now works with V1 InstrumentRegistry
//! instrument_registry.register("pvcam_v2", |id| {
//!     Box::new(V2InstrumentAdapter::new(
//!         PVCAMInstrumentV2::new(id.to_string())
//!     ))
//! });
//! ```

use crate::core::{Instrument as V1Instrument, InstrumentCommand as V1Command};
use crate::measurement::InstrumentMeasurement;
use anyhow::{Context, Result};
use async_trait::async_trait;
use daq_core::{Instrument as V2Instrument, InstrumentCommand as V2Command, Measurement};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use crate::measurement::DataDistributor; // Add DataDistributor import

/// Adapter that wraps a V2 instrument to work with V1 InstrumentRegistry.
///
/// The adapter spawns a background task that:
/// 1. Subscribes to V2 instrument's measurement stream
/// 2. Converts `Arc<Measurement>` to `DataPoint` format
/// 3. Broadcasts via `InstrumentMeasurement`
/// 4. (Optional) Broadcasts original `Arc<Measurement>` to V2 data_distributor
///
/// ## Thread Safety
///
/// The wrapped V2 instrument is protected by `Arc<Mutex<>>` to enable:
/// - Shared ownership between adapter and background task
/// - Safe concurrent access from V1 command handlers and V2 stream task
///
/// ## Shutdown Protocol
///
/// 1. `disconnect()` sends V2 `Shutdown` command to wrapped instrument
/// 2. Background task detects stream closure and exits
/// 3. Task handle is awaited with timeout
/// 4. Resources are released
pub struct V2InstrumentAdapter<I: V2Instrument> {
    /// Wrapped V2 instrument (shared with background task)
    inner: Arc<Mutex<I>>,
    
    /// V1 measurement interface for broadcasting
    measurement: InstrumentMeasurement,
    
    /// Background task handle (for cleanup on disconnect)
    task_handle: Option<JoinHandle<()>>,
    
    /// Shutdown signal sender (for graceful task termination)
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    
    /// Optional V2 data distributor for broadcasting original measurements
    /// This enables V2 GUI components (like ImageTab) to receive full Image/Spectrum data
    v2_distributor: Option<Arc<DataDistributor<Arc<Measurement>>>>,
}

impl<I: V2Instrument + 'static> V2InstrumentAdapter<I> {
    /// Creates a new adapter wrapping a V2 instrument.
    ///
    /// The V2 instrument should already be constructed but not yet initialized.
    /// Call `connect()` to initialize the wrapped instrument and start streaming.
    ///
    /// # Arguments
    ///
    /// * `instrument` - V2 instrument to wrap
    ///
    /// # Returns
    ///
    /// Adapter ready for registration in V1 InstrumentRegistry
    pub fn new(instrument: I) -> Self {
        let id = instrument.id().to_string();
        Self {
            inner: Arc::new(Mutex::new(instrument)),
            measurement: InstrumentMeasurement::new(1024, id),
            task_handle: None,
            shutdown_tx: None,
            v2_distributor: None,
        }
    }

    /// Get a receiver for the original V2 measurement stream.
    ///
    /// This allows consumers (like app_actor) to access the full V2 measurements
    /// (Scalar/Spectrum/Image) in addition to the V1-compatible DataPoint stream.
    ///
    /// ## Use Case
    ///
    /// V2 GUI components (like ImageTab) need the original `Arc<Measurement::Image>`
    /// data, not just the statistics DataPoints. This method exposes the V2 stream
    /// so app_actor can broadcast both:
    /// - Statistics DataPoints → data_distributor (V1 compatibility)
    /// - Original Arc<Measurement> → data_distributor (V2 GUI components)
    ///
    /// ## Returns
    ///
    /// A new receiver subscribed to the wrapped V2 instrument's measurement stream.
    pub fn v2_measurement_stream(&self) -> daq_core::MeasurementReceiver {
        let inner = self.inner.blocking_lock();
        inner.measurement_stream()
    }

    /// Set the V2 data distributor for broadcasting original measurements.
    ///
    /// This should be called before `connect()` to enable dual-channel broadcasting:
    /// - V1 DataPoint statistics via `InstrumentMeasurement`
    /// - V2 Arc<Measurement> via `data_distributor`
    ///
    /// When set, the adapter will broadcast ALL measurement types (Scalar/Spectrum/Image)
    /// to the data_distributor, enabling V2 GUI components to receive full image data.
    ///
    /// # Arguments
    ///
    /// * `distributor` - The application-wide data distributor
    pub fn set_v2_distributor(&mut self, distributor: Arc<DataDistributor<Arc<Measurement>>>) {
        self.v2_distributor = Some(distributor);
    }

    /// Spawn background task to forward V2 measurement stream to V1 broadcasts.
    ///
    /// This task runs until:
    /// - The V2 measurement stream closes (instrument shutdown)
    /// - A shutdown signal is received via oneshot channel
    /// - An error occurs (logged and task exits)
    ///
    /// ## Data Conversion
    ///
    /// V2 measurements are converted as follows:
    /// - `Measurement::Scalar(dp)` → Direct broadcast as DataPoint
    /// - `Measurement::Spectrum(sd)` → Broadcast as DataPoint with metadata
    /// - `Measurement::Image(id)` → Broadcast statistics + store image in metadata
    ///
    /// This ensures V1 components can still receive data from V2 instruments,
    /// even for non-scalar types.
    ///
    /// ## Dual-Channel Broadcasting
    ///
    /// If `v2_distributor` is set, the task also broadcasts the original
    /// `Arc<Measurement>` to enable V2 GUI components to receive full data.
    fn spawn_stream_task(&mut self) {
        let inner = Arc::clone(&self.inner);
        let measurement = self.measurement.clone();
        let v2_distributor = self.v2_distributor.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

        self.shutdown_tx = Some(shutdown_tx);

        self.task_handle = Some(tokio::spawn(async move {
            // Get V2 measurement receiver
            let mut rx = {
                let instrument = inner.lock().await;
                instrument.measurement_stream()
            };

            log::debug!("V2InstrumentAdapter: Starting measurement stream forwarding task");

            loop {
                tokio::select! {
                    // Forward measurements from V2 to V1
                    result = rx.recv() => {
                        match result {
                            Ok(arc_measurement) => {
                                // Broadcast original V2 measurement to V2 distributor (if set)
                                if let Some(ref distributor) = v2_distributor {
                                    if let Err(e) = distributor.broadcast(arc_measurement.clone()).await {
                                        log::warn!("V2InstrumentAdapter: Failed to broadcast V2 measurement: {}", e);
                                    }
                                }
                                
                                // Convert Arc<Measurement> to DataPoint(s) for V1 compatibility
                                let datapoints = Self::convert_measurement(&arc_measurement);
                                
                                // Broadcast to V1 subscribers
                                for dp in datapoints {
                                    if let Err(e) = measurement.broadcast(dp).await {
                                        log::warn!("V2InstrumentAdapter: Failed to broadcast: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                log::info!("V2InstrumentAdapter: Measurement stream closed: {}", e);
                                break;
                            }
                        }
                    }
                    
                    // Shutdown signal
                    _ = &mut shutdown_rx => {
                        log::debug!("V2InstrumentAdapter: Received shutdown signal");
                        break;
                    }
                }
            }

            log::debug!("V2InstrumentAdapter: Stream forwarding task exiting");
        }));
    }

    /// Convert V2 Measurement to V1 DataPoint(s).
    ///
    /// This is the bridge between the two measurement systems:
    /// - V2 uses `Measurement` enum (Scalar/Spectrum/Image)
    /// - V1 uses `DataPoint` struct (always scalar)
    ///
    /// ## Conversion Strategy
    ///
    /// - **Scalar**: Direct 1:1 conversion
    /// - **Spectrum**: Create multiple DataPoints (one per wavelength/frequency)
    /// - **Image**: Create DataPoints for statistics (mean, min, max) + metadata reference
    ///
    /// # Arguments
    ///
    /// * `measurement` - Arc-wrapped V2 measurement
    ///
    /// # Returns
    ///
    /// Vector of V1 DataPoints (may be empty, one, or many depending on type)
    fn convert_measurement(measurement: &Arc<Measurement>) -> Vec<crate::core::DataPoint> {
        match measurement.as_ref() {
            Measurement::Scalar(dp) => {
                // Direct conversion: V2 DataPoint → V1 DataPoint
                vec![crate::core::DataPoint {
                    timestamp: dp.timestamp,
                    instrument_id: String::new(), // V2 doesn't track instrument_id in DataPoint
                    channel: dp.channel.clone(),
                    value: dp.value,
                    unit: dp.unit.clone(),
                    metadata: None,
                }]
            }

            Measurement::Spectrum(sd) => {
                // Convert spectrum to multiple scalar points
                // Each (wavelength, intensity) pair becomes a DataPoint
                let mut points = Vec::with_capacity(sd.wavelengths.len());
                
                for (i, (&wavelength, &intensity)) in sd.wavelengths.iter().zip(&sd.intensities).enumerate() {
                    points.push(crate::core::DataPoint {
                        timestamp: sd.timestamp,
                        instrument_id: String::new(),
                        channel: format!("{}_{}", sd.channel, i),
                        value: intensity,
                        unit: sd.unit_y.clone(),
                        metadata: Some(serde_json::json!({
                            "type": "spectrum_point",
                            "wavelength": wavelength,
                            "wavelength_unit": sd.unit_x,
                        })),
                    });
                }
                
                points
            }

            Measurement::Image(img) => {
                // Convert image to statistics DataPoints
                // This allows V1 components to at least see summary statistics
                let pixels_f64 = img.pixels_as_f64();
                
                let mean = if !pixels_f64.is_empty() {
                    pixels_f64.iter().sum::<f64>() / pixels_f64.len() as f64
                } else {
                    0.0
                };
                
                let min = pixels_f64.iter().copied().fold(f64::INFINITY, f64::min);
                let max = pixels_f64.iter().copied().fold(f64::NEG_INFINITY, f64::max);

                vec![
                    // Mean intensity
                    crate::core::DataPoint {
                        timestamp: img.timestamp,
                        instrument_id: String::new(),
                        channel: format!("{}_mean", img.channel),
                        value: mean,
                        unit: img.unit.clone(),
                        metadata: Some(serde_json::json!({
                            "type": "image_statistic",
                            "statistic": "mean",
                            "width": img.width,
                            "height": img.height,
                        })),
                    },
                    // Min intensity
                    crate::core::DataPoint {
                        timestamp: img.timestamp,
                        instrument_id: String::new(),
                        channel: format!("{}_min", img.channel),
                        value: min,
                        unit: img.unit.clone(),
                        metadata: Some(serde_json::json!({
                            "type": "image_statistic",
                            "statistic": "min",
                        })),
                    },
                    // Max intensity
                    crate::core::DataPoint {
                        timestamp: img.timestamp,
                        instrument_id: String::new(),
                        channel: format!("{}_max", img.channel),
                        value: max,
                        unit: img.unit.clone(),
                        metadata: Some(serde_json::json!({
                            "type": "image_statistic",
                            "statistic": "max",
                        })),
                    },
                ]
            }
        }
    }

    /// Convert V1 InstrumentCommand to V2 InstrumentCommand.
    ///
    /// Not all V1 commands map cleanly to V2 commands, so this is best-effort.
    /// Unsupported commands return Ok(()) (no-op).
    ///
    /// ## Command Mapping
    ///
    /// | V1 Command | V2 Command |
    /// |------------|------------|
    /// | Shutdown | Shutdown |
    /// | SetParameter | SetParameter |
    /// | QueryParameter | GetParameter |
    /// | Execute | StartAcquisition (if command = "start") |
    /// | Capability | Not supported (V2 doesn't have capabilities) |
    fn convert_command(cmd: V1Command) -> Option<V2Command> {
        match cmd {
            V1Command::Shutdown => Some(V2Command::Shutdown),
            
            V1Command::SetParameter(name, value) => {
                // Convert ParameterValue to serde_json::Value
                let json_value = match value {
                    crate::core::ParameterValue::Bool(b) => serde_json::Value::Bool(b),
                    crate::core::ParameterValue::Int(i) => serde_json::Value::Number(i.into()),
                    crate::core::ParameterValue::Float(f) => {
                        serde_json::Number::from_f64(f)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::Null)
                    }
                    crate::core::ParameterValue::String(s) => serde_json::Value::String(s),
                    crate::core::ParameterValue::FloatArray(arr) => {
                        serde_json::Value::Array(
                            arr.into_iter()
                                .filter_map(|f| serde_json::Number::from_f64(f))
                                .map(serde_json::Value::Number)
                                .collect()
                        )
                    }
                    crate::core::ParameterValue::IntArray(arr) => {
                        serde_json::Value::Array(
                            arr.into_iter()
                                .map(|i| serde_json::Value::Number(i.into()))
                                .collect()
                        )
                    }
                    crate::core::ParameterValue::Array(_) => {
                        // Complex nested arrays not supported, use null
                        serde_json::Value::Null
                    }
                    crate::core::ParameterValue::Object(_) => {
                        // Complex objects not supported, use null
                        serde_json::Value::Null
                    }
                    crate::core::ParameterValue::Null => serde_json::Value::Null,
                };

                Some(V2Command::SetParameter {
                    name,
                    value: json_value,
                })
            }

            V1Command::QueryParameter(name) => {
                Some(V2Command::GetParameter { name })
            }

            V1Command::Execute(command, _args) => {
                // Map common commands
                match command.as_str() {
                    "start" | "start_acquisition" => Some(V2Command::StartAcquisition),
                    "stop" | "stop_acquisition" => Some(V2Command::StopAcquisition),
                    _ => {
                        log::warn!("V2InstrumentAdapter: Unsupported Execute command: {}", command);
                        None
                    }
                }
            }

            V1Command::Capability { .. } => {
                // V2 doesn't have capability system, ignore
                log::debug!("V2InstrumentAdapter: Ignoring Capability command (not supported in V2)");
                None
            }
        }
    }
}

#[async_trait]
impl<I: V2Instrument + 'static> V1Instrument for V2InstrumentAdapter<I> {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        // Must block to access inner instrument
        // This is called from non-async contexts, so we use blocking_lock
        let inner = self.inner.blocking_lock();
        format!("{} (V2)", inner.id())
    }

    async fn connect(
        &mut self,
        id: &str,
        _settings: &Arc<crate::config::Settings>,
    ) -> Result<()> {
        log::info!("V2InstrumentAdapter: Connecting instrument '{}'", id);

        // Initialize V2 instrument
        {
            let mut inner = self.inner.lock().await;
            inner.initialize().await
                .context("Failed to initialize V2 instrument")?;
        }

        // Spawn measurement forwarding task
        self.spawn_stream_task();

        log::info!("V2InstrumentAdapter: Successfully connected '{}'", id);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        log::info!("V2InstrumentAdapter: Disconnecting instrument");

        // Send shutdown signal to background task
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()); // Ignore error if receiver already dropped
        }

        // Wait for task to finish (with timeout)
        if let Some(handle) = self.task_handle.take() {
            match tokio::time::timeout(std::time::Duration::from_secs(5), handle).await {
                Ok(result) => {
                    if let Err(e) = result {
                        log::warn!("V2InstrumentAdapter: Stream task panicked: {}", e);
                    }
                }
                Err(_) => {
                    log::warn!("V2InstrumentAdapter: Stream task timeout, aborting");
                    // Task will be aborted when handle is dropped
                }
            }
        }

        // Shutdown V2 instrument
        {
            let mut inner = self.inner.lock().await;
            inner.shutdown().await
                .context("Failed to shutdown V2 instrument")?;
        }

        log::info!("V2InstrumentAdapter: Disconnect complete");
        Ok(())
    }

    fn measure(&self) -> &Self::Measure {
        &self.measurement
    }

    async fn handle_command(&mut self, command: V1Command) -> Result<()> {
        log::debug!("V2InstrumentAdapter: Handling command: {:?}", command);

        // Convert V1 command to V2 command
        let v2_command = match Self::convert_command(command) {
            Some(cmd) => cmd,
            None => {
                log::debug!("V2InstrumentAdapter: Command not converted, ignoring");
                return Ok(());
            }
        };

        // Forward to V2 instrument
        let mut inner = self.inner.lock().await;
        inner.handle_command(v2_command).await
            .context("V2 instrument command failed")?;

        Ok(())
    }
    
    fn set_v2_data_distributor(&mut self, distributor: Arc<DataDistributor<Arc<Measurement>>>) {
        log::info!("V2InstrumentAdapter: Setting V2 data distributor");
        self.v2_distributor = Some(distributor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measurement::Measure;
    use daq_core::{arc_measurement, DataPoint, InstrumentState, MeasurementSender};

    /// Mock V2 instrument for testing
    struct MockV2Instrument {
        id: String,
        state: InstrumentState,
        measurement_tx: MeasurementSender,
        _measurement_rx: daq_core::MeasurementReceiver,
    }

    impl MockV2Instrument {
        fn new(id: String) -> Self {
            let (tx, rx) = daq_core::measurement_channel(10);
            Self {
                id,
                state: InstrumentState::Disconnected,
                measurement_tx: tx,
                _measurement_rx: rx,
            }
        }

        fn emit_scalar(&self, value: f64) {
            let dp = DataPoint {
                timestamp: chrono::Utc::now(),
                channel: format!("{}_test", self.id),
                value,
                unit: "V".to_string(),
            };
            let _ = self.measurement_tx.send(arc_measurement(Measurement::Scalar(dp)));
        }
    }

    #[async_trait]
    impl V2Instrument for MockV2Instrument {
        fn id(&self) -> &str {
            &self.id
        }

        fn instrument_type(&self) -> &str {
            "mock_v2"
        }

        fn state(&self) -> InstrumentState {
            self.state.clone()
        }

        async fn initialize(&mut self) -> daq_core::Result<()> {
            self.state = InstrumentState::Ready;
            Ok(())
        }

        async fn shutdown(&mut self) -> daq_core::Result<()> {
            self.state = InstrumentState::Disconnected;
            Ok(())
        }

        fn measurement_stream(&self) -> daq_core::MeasurementReceiver {
            self.measurement_tx.subscribe()
        }

        async fn handle_command(&mut self, _cmd: V2Command) -> daq_core::Result<()> {
            Ok(())
        }

        async fn recover(&mut self) -> daq_core::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_adapter_lifecycle() {
        let mock = MockV2Instrument::new("test".to_string());
        let mut adapter = V2InstrumentAdapter::new(mock);

        // Connect should initialize V2 instrument and spawn task
        let settings = Arc::new(crate::config::Settings::new(None).unwrap());
        adapter.connect("test", &settings).await.unwrap();

        // Disconnect should cleanup
        adapter.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn test_adapter_forwards_measurements() {
        let mock = MockV2Instrument::new("test".to_string());
        let mock_tx = mock.measurement_tx.clone();
        let mut adapter = V2InstrumentAdapter::new(mock);

        let settings = Arc::new(crate::config::Settings::new(None).unwrap());
        adapter.connect("test", &settings).await.unwrap();

        // Give the background task a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Subscribe to V1 measurement stream
        let mut rx = adapter.measure().data_stream().await.unwrap();

        // Emit V2 measurement
        let dp = DataPoint {
            timestamp: chrono::Utc::now(),
            channel: "test_channel".to_string(),
            value: 42.0,
            unit: "V".to_string(),
        };
        mock_tx.send(arc_measurement(Measurement::Scalar(dp.clone()))).unwrap();

        // Should receive converted measurement on V1 stream
        let received = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            rx.recv()
        ).await.unwrap().unwrap();

        assert_eq!(received.channel, "test_channel");
        assert_eq!(received.value, 42.0);
        assert_eq!(received.unit, "V");

        adapter.disconnect().await.unwrap();
    }

    #[tokio::test]
    async fn test_command_conversion() {
        // Test SetParameter conversion
        let v1_cmd = V1Command::SetParameter(
            "exposure".to_string(),
            crate::core::ParameterValue::Float(100.0),
        );
        let v2_cmd = V2InstrumentAdapter::<MockV2Instrument>::convert_command(v1_cmd);
        assert!(matches!(v2_cmd, Some(V2Command::SetParameter { .. })));

        // Test Shutdown conversion
        let v1_cmd = V1Command::Shutdown;
        let v2_cmd = V2InstrumentAdapter::<MockV2Instrument>::convert_command(v1_cmd);
        assert!(matches!(v2_cmd, Some(V2Command::Shutdown)));
    }
}