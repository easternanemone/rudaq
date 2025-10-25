//! Generic SCPI Instrument V3 (Unified Architecture)
//!
//! V3 implementation demonstrating generic instrument pattern:
//! - Implements ONLY `core_v3::Instrument` trait (no meta-trait)
//! - Validates V3 extensibility for arbitrary instruments
//! - Generic SCPI command execution via `Command::Custom`
//! - VISA abstraction layer (Mock/Real via feature flag)
//!
//! ## Purpose
//!
//! This is the FINAL Phase 2 migration and validates that V3 architecture
//! works for instruments that DON'T fit specific meta-traits (PowerMeter,
//! Stage, Laser, Camera). Generic SCPI instruments might be:
//! - Multimeters
//! - Oscilloscopes
//! - Function generators
//! - Power supplies
//! - Any SCPI-compliant device
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.multimeter]
//! type = "scpi_v3"
//! resource = "TCPIP::192.168.1.100::INSTR"
//! timeout_ms = 5000
//! sdk_mode = "mock"  # or "real" for actual VISA hardware
//! ```
//!
//! ## SCPI Protocol
//!
//! SCPI (Standard Commands for Programmable Instruments):
//! - Industry-standard ASCII command protocol
//! - Format: `COMMAND:SUBCOMMAND? [args]`
//! - Query suffix: `?` returns value
//! - Examples: `*IDN?` (identification), `MEAS:VOLT?` (measure voltage)
//!
//! ## Migration Notes
//!
//! This replaces the V1 `ScpiInstrument` and V2 `ScpiInstrumentV2`:
//! - V1: Actor model with message passing
//! - V2: SerialAdapter/VisaAdapter abstraction
//! - V3: Direct async methods, VISA abstraction, no adapter layer

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::core_v3::{
    Command, Instrument, InstrumentState, Measurement, ParameterBase, Response,
};
use crate::parameter::{Parameter, ParameterBuilder};

// =============================================================================
// VISA Abstraction (for testing)
// =============================================================================

/// VISA resource abstraction trait for testing
#[async_trait]
trait VisaResource: Send + Sync {
    /// Write command to instrument
    async fn write(&mut self, cmd: &str) -> Result<()>;
    
    /// Query command (write + read)
    async fn query(&mut self, cmd: &str) -> Result<String>;
    
    /// Close resource
    async fn close(&mut self) -> Result<()>;
}

/// Mock VISA resource for testing
struct MockVisaResource {
    resource_string: String,
    identity: String,
    measurement_value: f64,
}

impl MockVisaResource {
    fn new(resource: &str) -> Self {
        Self {
            resource_string: resource.to_string(),
            identity: format!("Mock SCPI Instrument,Model 1234,SN001,v1.0 [{}]", resource),
            measurement_value: 1.234, // Default measurement
        }
    }
}

#[async_trait]
impl VisaResource for MockVisaResource {
    async fn write(&mut self, cmd: &str) -> Result<()> {
        // Simulate command processing
        if cmd.starts_with("*RST") {
            // Reset to default measurement value (not zero, to test functionality)
            self.measurement_value = 1.234;
        }
        Ok(())
    }
    
    async fn query(&mut self, cmd: &str) -> Result<String> {
        // Simulate SCPI responses
        match cmd.trim() {
            "*IDN?" => Ok(self.identity.clone()),
            "*OPC?" => Ok("1".to_string()),
            "MEAS:VOLT?" | "MEAS:VOLT:DC?" => {
                Ok(format!("{:.6e}", self.measurement_value))
            }
            "MEAS:CURR?" | "MEAS:CURR:DC?" => {
                Ok(format!("{:.6e}", self.measurement_value * 0.001))
            }
            "SYST:ERR?" => Ok("0,\"No error\"".to_string()),
            _ => Ok("0".to_string()), // Generic success response
        }
    }
    
    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Real VISA resource implementation (feature-gated)
#[cfg(feature = "instrument_visa")]
struct RealVisaResource {
    // TODO: Integrate actual VISA library (e.g., pyvisa-rs or native VISA)
    _resource_string: String,
}

#[cfg(feature = "instrument_visa")]
#[async_trait]
impl VisaResource for RealVisaResource {
    async fn write(&mut self, _cmd: &str) -> Result<()> {
        // TODO: Implement actual VISA write
        Err(anyhow!("Real VISA not yet implemented"))
    }
    
    async fn query(&mut self, _cmd: &str) -> Result<String> {
        // TODO: Implement actual VISA query
        Err(anyhow!("Real VISA not yet implemented"))
    }
    
    async fn close(&mut self) -> Result<()> {
        // TODO: Implement actual VISA close
        Ok(())
    }
}

// =============================================================================
// SDK Mode Selection
// =============================================================================

/// SDK mode for SCPI instrument
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScpiSdkKind {
    /// Mock VISA for testing
    Mock,
    /// Real VISA hardware
    Real,
}

// =============================================================================
// Generic SCPI Instrument V3
// =============================================================================

/// Generic SCPI Instrument V3 implementation
///
/// Demonstrates V3 architecture for instruments that DON'T fit specific
/// meta-traits. Implements ONLY the core `Instrument` trait, validating
/// that V3 is extensible for arbitrary instruments.
///
/// This pattern is suitable for:
/// - Multimeters (generic measurements)
/// - Oscilloscopes (waveform capture)
/// - Function generators (signal generation)
/// - Power supplies (voltage/current control)
/// - Any SCPI-compliant instrument without specialized traits
pub struct ScpiInstrumentV3 {
    /// Instrument identifier
    id: String,
    
    /// Current state
    state: InstrumentState,
    
    /// Data broadcast channel
    data_tx: broadcast::Sender<Measurement>,
    
    /// Parameters (for dynamic access via ParameterBase)
    parameters: HashMap<String, Box<dyn ParameterBase>>,
    
    // VISA abstraction
    visa_resource: Arc<RwLock<Option<Box<dyn VisaResource>>>>,
    resource_string: String,
    sdk_kind: ScpiSdkKind,
    
    // Instrument identity
    identity: Arc<RwLock<Option<String>>>,
    
    // Typed parameters
    timeout_ms: Arc<RwLock<Parameter<u64>>>,
    auto_clear: Arc<RwLock<Parameter<bool>>>,
}

impl ScpiInstrumentV3 {
    /// Create new generic SCPI instrument V3
    ///
    /// # Arguments
    /// * `id` - Unique instrument identifier
    /// * `resource_string` - VISA resource string (e.g., "TCPIP::192.168.1.1::INSTR")
    /// * `sdk_kind` - Mock or Real VISA mode
    pub fn new(
        id: impl Into<String>,
        resource_string: impl Into<String>,
        sdk_kind: ScpiSdkKind,
    ) -> Self {
        let id = id.into();
        let (data_tx, _) = broadcast::channel(1024);
        
        // Create parameters
        let timeout_ms = Arc::new(RwLock::new(
            ParameterBuilder::new("timeout_ms", 5000u64)
                .description("VISA timeout in milliseconds")
                .unit("ms")
                .range(100u64, 30000u64)
                .build(),
        ));
        
        let auto_clear = Arc::new(RwLock::new(
            ParameterBuilder::new("auto_clear", true)
                .description("Automatically clear errors after queries")
                .build(),
        ));
        
        Self {
            id,
            state: InstrumentState::Uninitialized,
            data_tx,
            parameters: HashMap::new(),
            visa_resource: Arc::new(RwLock::new(None)),
            resource_string: resource_string.into(),
            sdk_kind,
            identity: Arc::new(RwLock::new(None)),
            timeout_ms,
            auto_clear,
        }
    }
    
    /// Send SCPI write command (no response expected)
    ///
    /// # Example
    /// ```ignore
    /// scpi.write("*RST").await?;
    /// scpi.write("OUTP:STAT ON").await?;
    /// ```
    pub async fn write(&self, cmd: &str) -> Result<()> {
        let mut resource = self.visa_resource.write().await;
        if let Some(resource) = &mut *resource {
            resource.write(cmd).await
        } else {
            Err(anyhow!("VISA resource not initialized"))
        }
    }
    
    /// Send SCPI query command (write + read response)
    ///
    /// # Example
    /// ```ignore
    /// let voltage = scpi.query("MEAS:VOLT?").await?;
    /// ```
    pub async fn query(&self, cmd: &str) -> Result<String> {
        let mut resource = self.visa_resource.write().await;
        if let Some(resource) = &mut *resource {
            resource.query(cmd).await
        } else {
            Err(anyhow!("VISA resource not initialized"))
        }
    }
    
    /// Execute arbitrary SCPI command and broadcast result as measurement
    ///
    /// This is a convenience method that queries the instrument and
    /// broadcasts the result as a scalar measurement. Useful for
    /// continuous monitoring of instrument values.
    ///
    /// # Example
    /// ```ignore
    /// // Query voltage and broadcast
    /// scpi.query_and_broadcast("voltage", "MEAS:VOLT?", "V").await?;
    /// ```
    pub async fn query_and_broadcast(
        &self,
        name: &str,
        cmd: &str,
        unit: &str,
    ) -> Result<f64> {
        let response = self.query(cmd).await?;
        
        // Parse numeric response
        let value: f64 = response
            .trim()
            .parse()
            .map_err(|e| anyhow!("Failed to parse '{}': {}", response, e))?;
        
        // Broadcast measurement
        let measurement = Measurement::Scalar {
            name: format!("{}_{}", self.id, name),
            value,
            unit: unit.to_string(),
            timestamp: Utc::now(),
        };
        let _ = self.data_tx.send(measurement);
        
        Ok(value)
    }
}

// =============================================================================
// Instrument Trait Implementation
// =============================================================================

#[async_trait]
impl Instrument for ScpiInstrumentV3 {
    fn id(&self) -> &str {
        &self.id
    }
    
    fn state(&self) -> InstrumentState {
        self.state
    }
    
    async fn initialize(&mut self) -> Result<()> {
        if self.state != InstrumentState::Uninitialized {
            return Err(anyhow!("Already initialized"));
        }
        
        // Initialize VISA resource based on SDK kind
        match self.sdk_kind {
            ScpiSdkKind::Mock => {
                let mut resource = self.visa_resource.write().await;
                *resource = Some(Box::new(MockVisaResource::new(&self.resource_string)));
            }
            #[cfg(feature = "instrument_visa")]
            ScpiSdkKind::Real => {
                // TODO: Initialize real VISA resource
                return Err(anyhow!("Real VISA not yet implemented - use Mock for now"));
            }
            #[cfg(not(feature = "instrument_visa"))]
            ScpiSdkKind::Real => {
                return Err(anyhow!(
                    "Real VISA not available - enable 'instrument_visa' feature"
                ));
            }
        }
        
        // Query instrument identity
        let id_response = self.query("*IDN?").await?;
        {
            let mut identity = self.identity.write().await;
            *identity = Some(id_response.clone());
        }
        
        log::info!(
            "SCPI instrument '{}' connected: {}",
            self.id,
            id_response
        );
        
        // Send reset command
        self.write("*RST").await?;
        
        // Wait for operations to complete
        self.query("*OPC?").await?;
        
        self.state = InstrumentState::Idle;
        Ok(())
    }
    
    async fn shutdown(&mut self) -> Result<()> {
        self.state = InstrumentState::ShuttingDown;
        
        // Close VISA resource
        let mut resource = self.visa_resource.write().await;
        if let Some(resource) = &mut *resource {
            resource.close().await?;
        }
        *resource = None;
        
        Ok(())
    }
    
    fn data_channel(&self) -> broadcast::Receiver<Measurement> {
        self.data_tx.subscribe()
    }
    
    async fn execute(&mut self, cmd: Command) -> Result<Response> {
        match cmd {
            Command::Start => {
                self.state = InstrumentState::Running;
                Ok(Response::Ok)
            }
            Command::Stop => {
                self.state = InstrumentState::Idle;
                Ok(Response::Ok)
            }
            Command::Custom(scpi_cmd, args) => {
                // Execute arbitrary SCPI command via Command::Custom
                // This demonstrates V3's extensibility for generic instruments
                
                if scpi_cmd.ends_with('?') {
                    // Query command
                    let response = self.query(&scpi_cmd).await?;
                    Ok(Response::Custom(serde_json::json!(response)))
                } else {
                    // Write command (may have args)
                    let full_cmd = if let Some(arg_value) = args.as_str() {
                        format!("{} {}", scpi_cmd, arg_value)
                    } else if let Some(arg_num) = args.as_f64() {
                        format!("{} {}", scpi_cmd, arg_num)
                    } else {
                        scpi_cmd
                    };
                    
                    self.write(&full_cmd).await?;
                    Ok(Response::Ok)
                }
            }
            _ => Ok(Response::Ok),
        }
    }
    
    fn parameters(&self) -> &HashMap<String, Box<dyn ParameterBase>> {
        &self.parameters
    }
    
    fn parameters_mut(&mut self) -> &mut HashMap<String, Box<dyn ParameterBase>> {
        &mut self.parameters
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_scpi_v3_initialization() {
        let mut scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        assert_eq!(scpi.state(), InstrumentState::Uninitialized);
        
        scpi.initialize().await.unwrap();
        assert_eq!(scpi.state(), InstrumentState::Idle);
        
        // Verify identity was queried
        let identity = scpi.identity.read().await;
        assert!(identity.is_some());
        assert!(identity.as_ref().unwrap().contains("Mock SCPI Instrument"));
    }
    
    #[tokio::test]
    async fn test_scpi_v3_write_command() {
        let mut scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        scpi.initialize().await.unwrap();
        
        // Send write command
        scpi.write("*RST").await.unwrap();
        scpi.write("OUTP:STAT ON").await.unwrap();
    }
    
    #[tokio::test]
    async fn test_scpi_v3_query_command() {
        let mut scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        scpi.initialize().await.unwrap();
        
        // Query identity
        let idn = scpi.query("*IDN?").await.unwrap();
        assert!(idn.contains("Mock SCPI Instrument"));
        
        // Query voltage (returns scientific notation: 1.234000e0)
        let voltage = scpi.query("MEAS:VOLT?").await.unwrap();
        let voltage_val: f64 = voltage.trim().parse().unwrap();
        assert!((voltage_val - 1.234).abs() < 1e-6);
    }
    
    #[tokio::test]
    async fn test_scpi_v3_custom_command() {
        let mut scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        scpi.initialize().await.unwrap();
        
        // Execute custom query command via Command::Custom
        let cmd = Command::Custom(
            "MEAS:VOLT?".to_string(),
            serde_json::Value::Null,
        );
        let response = scpi.execute(cmd).await.unwrap();
        
        match response {
            Response::Custom(data) => {
                let voltage_str = data.as_str().unwrap();
                // Parse scientific notation response
                let voltage_val: f64 = voltage_str.trim().parse().unwrap();
                assert!((voltage_val - 1.234).abs() < 1e-6);
            }
            _ => panic!("Expected Response::Custom"),
        }
        
        // Execute custom write command via Command::Custom
        let cmd = Command::Custom(
            "OUTP:STAT".to_string(),
            serde_json::json!("ON"),
        );
        let response = scpi.execute(cmd).await.unwrap();
        assert!(matches!(response, Response::Ok));
    }
    
    #[tokio::test]
    async fn test_scpi_v3_query_and_broadcast() {
        let mut scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        scpi.initialize().await.unwrap();
        
        // Subscribe to data channel
        let mut rx = scpi.data_channel();
        
        // Query and broadcast voltage
        let value = scpi
            .query_and_broadcast("voltage", "MEAS:VOLT?", "V")
            .await
            .unwrap();
        assert!((value - 1.234e0).abs() < 1e-6);
        
        // Check that measurement was broadcast
        tokio::select! {
            result = rx.recv() => {
                let measurement = result.unwrap();
                match measurement {
                    Measurement::Scalar { name, value, unit, .. } => {
                        assert_eq!(name, "test_scpi_voltage");
                        assert!((value - 1.234).abs() < 1e-6);
                        assert_eq!(unit, "V");
                    }
                    _ => panic!("Expected Scalar measurement"),
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                panic!("No measurement received");
            }
        }
    }
    
    #[tokio::test]
    async fn test_scpi_v3_multiple_queries() {
        let mut scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        scpi.initialize().await.unwrap();
        
        // Query different measurements
        let voltage = scpi.query("MEAS:VOLT?").await.unwrap();
        let current = scpi.query("MEAS:CURR?").await.unwrap();
        let error = scpi.query("SYST:ERR?").await.unwrap();
        
        // Parse scientific notation responses
        let voltage_val: f64 = voltage.trim().parse().unwrap();
        assert!((voltage_val - 1.234).abs() < 1e-6);
        
        let current_val: f64 = current.trim().parse().unwrap();
        assert!((current_val - 1.234e-3).abs() < 1e-9); // 1mA
        
        assert!(error.contains("No error"));
    }
    
    #[tokio::test]
    async fn test_scpi_v3_state_transitions() {
        let mut scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        scpi.initialize().await.unwrap();
        assert_eq!(scpi.state(), InstrumentState::Idle);
        
        // Start
        scpi.execute(Command::Start).await.unwrap();
        assert_eq!(scpi.state(), InstrumentState::Running);
        
        // Stop
        scpi.execute(Command::Stop).await.unwrap();
        assert_eq!(scpi.state(), InstrumentState::Idle);
    }
    
    #[tokio::test]
    async fn test_scpi_v3_shutdown() {
        let mut scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        scpi.initialize().await.unwrap();
        
        scpi.shutdown().await.unwrap();
        assert_eq!(scpi.state(), InstrumentState::ShuttingDown);
        
        // Verify resource was closed
        let resource = scpi.visa_resource.read().await;
        assert!(resource.is_none());
    }
    
    #[tokio::test]
    async fn test_scpi_v3_error_handling() {
        let scpi = ScpiInstrumentV3::new(
            "test_scpi",
            "TCPIP::192.168.1.100::INSTR",
            ScpiSdkKind::Mock,
        );
        
        // Query without initialization should fail
        let result = scpi.query("*IDN?").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not initialized"));
    }
}