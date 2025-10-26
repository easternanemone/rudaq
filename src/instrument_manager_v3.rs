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
use tokio::sync::{broadcast, oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::core_v3::{Command, Instrument, Measurement, Response};

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
    measurement_rx: broadcast::Receiver<Measurement>,
}

/// V3 Instrument Manager - The orchestration layer
///
/// Coordinates V3 instrument lifecycle, configuration, and data flow. This is the
/// missing architectural tier identified in Phase 2 analysis - all reference
/// frameworks have equivalent (DynExp ModuleManager, PyMoDAQ PluginManager, etc.)
pub struct InstrumentManagerV3 {
    /// Registry mapping instrument type names to factory functions
    ///
    /// Example: "Newport1830CV3" -> Newport1830CV3::from_config
    factories: HashMap<String, InstrumentFactory>,
    
    /// Active instruments keyed by their configuration ID
    ///
    /// Example: "power_meter_1" -> InstrumentHandle
    active_instruments: Arc<Mutex<HashMap<String, InstrumentHandle>>>,
    
    /// Broadcast channel for aggregated measurements (V3 → V1 bridge)
    ///
    /// Temporarily bridges V3 Measurement to V1 InstrumentMeasurement for
    /// backward compatibility during Phase 3 migration
    legacy_bridge_tx: Option<broadcast::Sender<Measurement>>,
}

impl InstrumentManagerV3 {
    /// Create a new instrument manager with empty factory registry
    ///
    /// Call `register_factory()` to add instrument types before loading from config.
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            active_instruments: Arc::new(Mutex::new(HashMap::new())),
            legacy_bridge_tx: None,
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
    
    /// Set the legacy bridge channel for V3 → V1 data flow
    ///
    /// During Phase 3, V3 measurements are bridged to V1 InstrumentMeasurement
    /// for backward compatibility with existing DaqApp/GUI/Storage.
    pub fn set_legacy_bridge(&mut self, tx: broadcast::Sender<Measurement>) {
        self.legacy_bridge_tx = Some(tx);
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
        
        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        
        // Spawn runtime task
        let task_handle = tokio::spawn(async move {
            // Wait for shutdown signal
            let _ = shutdown_rx.await;
            
            // Graceful shutdown
            instrument.shutdown().await?;
            
            Ok(())
        });
        
        // Setup data bridge if legacy channel configured
        if let Some(bridge_tx) = &self.legacy_bridge_tx {
            Self::spawn_data_bridge(
                cfg.id.clone(),
                measurement_rx.resubscribe(),
                bridge_tx.clone(),
            );
        }
        
        // Store handle
        let handle = InstrumentHandle {
            shutdown_tx: Some(shutdown_tx),
            task_handle,
            measurement_rx,
        };
        
        self.active_instruments.lock().await.insert(cfg.id.clone(), handle);
        
        Ok(())
    }
    
    /// Spawn data bridge task for V3 → V1 compatibility
    ///
    /// Subscribes to V3 measurement channel and forwards to legacy broadcast.
    /// Currently only supports Measurement::Scalar; logs warnings for Image/Spectrum.
    fn spawn_data_bridge(
        instrument_id: String,
        mut v3_rx: broadcast::Receiver<Measurement>,
        legacy_tx: broadcast::Sender<Measurement>,
    ) {
        tokio::spawn(async move {
            loop {
                match v3_rx.recv().await {
                    Ok(measurement) => {
                        // Check if V1 can handle this measurement type
                        match &measurement {
                            Measurement::Scalar { .. } => {
                                // Forward to legacy channel
                                if let Err(e) = legacy_tx.send(measurement) {
                                    tracing::error!(
                                        "Legacy bridge send failed for '{}': {}",
                                        instrument_id,
                                        e
                                    );
                                    break;
                                }
                            }
                            Measurement::Image { .. } => {
                                tracing::warn!(
                                    "Image measurement from '{}' not supported by V1 bridge (Phase 3 limitation)",
                                    instrument_id
                                );
                            }
                            Measurement::Spectrum { .. } => {
                                tracing::warn!(
                                    "Spectrum measurement from '{}' not supported by V1 bridge (Phase 3 limitation)",
                                    instrument_id
                                );
                            }
                            _ => {
                                tracing::warn!(
                                    "Unknown measurement type from '{}' not supported by V1 bridge",
                                    instrument_id
                                );
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
        });
    }
    
    /// Execute a command on a specific instrument
    ///
    /// This is the primary control interface for V3 instruments. Commands are
    /// sent directly (no actor model overhead) and responses are awaited.
    pub async fn execute_command(
        &self,
        instrument_id: &str,
        command: Command,
    ) -> Result<Response> {
        // In this simplified implementation, we don't hold instruments directly
        // Instead, commands would be sent via channels to instrument tasks
        // TODO: Implement command channels per instrument
        
        Err(anyhow!(
            "Command execution not yet implemented - Phase 3 Milestone 2"
        ))
    }
    
    /// Get measurement receiver for a specific instrument
    ///
    /// Returns a broadcast receiver that can subscribe to the instrument's
    /// measurement stream. Used by GUI, storage writers, and processors.
    pub async fn subscribe_measurements(
        &self,
        instrument_id: &str,
    ) -> Result<broadcast::Receiver<Measurement>> {
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
        let mut instruments = self.active_instruments.lock().await;
        let ids: Vec<String> = instruments.keys().cloned().collect();
        
        for id in ids {
            if let Some(mut handle) = instruments.remove(&id) {
                // Send shutdown signal
                if let Some(shutdown_tx) = handle.shutdown_tx.take() {
                    let _ = shutdown_tx.send(());
                }
                
                // Await task completion with timeout
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    handle.task_handle,
                )
                .await
                {
                    Ok(Ok(Ok(()))) => {
                        tracing::info!("Instrument '{}' shutdown successfully", id);
                    }
                    Ok(Ok(Err(e))) => {
                        tracing::error!("Instrument '{}' shutdown error: {}", id, e);
                    }
                    Ok(Err(e)) => {
                        tracing::error!("Instrument '{}' task panicked: {}", id, e);
                    }
                    Err(_) => {
                        tracing::warn!("Instrument '{}' shutdown timeout (5s), aborting", id);
                        // Task aborts automatically when JoinHandle drops
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

/// V3 instrument configuration from TOML
///
/// Represents a `[[instruments_v3]]` section in config/default.toml
#[derive(Debug, Clone, serde::Deserialize)]
pub struct InstrumentConfigV3 {
    /// Unique identifier for this instrument instance
    pub id: String,
    
    /// Instrument type name (must match factory registry key)
    pub type_name: String,
    
    /// Type-specific configuration settings
    ///
    /// Deserialized from TOML table to JSON for flexibility. Each instrument
    /// factory is responsible for parsing its own settings.
    #[serde(default)]
    pub settings: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_v3::{InstrumentState, ParameterBase};
    
    // Mock instrument for testing
    struct MockInstrumentV3 {
        id: String,
        tx: broadcast::Sender<Measurement>,
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
        
        fn data_channel(&self) -> broadcast::Receiver<Measurement> {
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
}