//! Newport 1830-C Power Meter V3 (Unified Architecture)
//!
//! V3 implementation using the unified core_v3 traits:
//! - Implements `core_v3::Instrument` trait (replaces V1/V2 split)
//! - Implements `core_v3::PowerMeter` trait for polymorphism
//! - Uses `Parameter<T>` for declarative parameter management
//! - Direct async methods (no InstrumentCommand message passing)
//! - Single broadcast channel (no double-broadcast overhead)
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.power_meter]
//! type = "newport_1830c_v3"
//! port = "/dev/ttyUSB0"
//! wavelength_nm = 1550.0
//! range = "auto"  # or "1mW", "10mW", "100mW", etc.
//! sdk_mode = "mock"  # or "real" for actual hardware
//! ```
//!
//! ## Protocol
//!
//! Newport 1830-C uses RS-232 serial communication:
//! - Baud: 9600, 8N1
//! - Commands: "PM:P?" (read power), "PM:Lambda <nm>" (set wavelength)
//! - Responses: ASCII text terminated by "\r\n"
//!
//! ## Migration from V2
//!
//! V3 eliminates the SerialAdapter and actor model:
//! - V2: SerialAdapter + handle_command() → Complex message passing
//! - V3: Direct trait methods (read_power(), set_wavelength(), etc.)

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::core_v3::{
    Command, Instrument, InstrumentState, Measurement, ParameterBase, PowerMeter, Response,
};
use crate::parameter::{Parameter, ParameterBuilder};

// =============================================================================
// Serial Port Abstraction (for testing)
// =============================================================================

/// Serial port abstraction trait for testing
#[async_trait]
trait SerialPort: Send + Sync {
    async fn write(&mut self, data: &str) -> Result<()>;
    async fn read_line(&mut self) -> Result<String>;
}

/// Mock serial port for testing
struct MockSerialPort {
    wavelength: f64,
    power: f64,
}

impl MockSerialPort {
    fn new() -> Self {
        Self {
            wavelength: 1550.0,
            power: 0.001, // 1mW default
        }
    }
}

#[async_trait]
impl SerialPort for MockSerialPort {
    async fn write(&mut self, data: &str) -> Result<()> {
        let cmd = data.trim();
        
        // Parse and handle commands
        if cmd.starts_with("PM:Lambda") {
            if let Some(nm_str) = cmd.split_whitespace().nth(1) {
                self.wavelength = nm_str.parse()?;
            }
        }
        
        Ok(())
    }
    
    async fn read_line(&mut self) -> Result<String> {
        // Return mock responses based on last command
        Ok(format!("{:.6e}\r\n", self.power))
    }
}

/// Real serial port wrapper
#[cfg(feature = "instrument_serial")]
struct RealSerialPort {
    port: std::sync::Mutex<Box<dyn serialport::SerialPort>>,
}

#[cfg(feature = "instrument_serial")]
#[async_trait]
impl SerialPort for RealSerialPort {
    async fn write(&mut self, data: &str) -> Result<()> {
        use std::io::Write;
        let mut port = self.port.lock().unwrap();
        port.write_all(data.as_bytes())?;
        Ok(())
    }
    
    async fn read_line(&mut self) -> Result<String> {
        use std::io::Read;
        let mut port = self.port.lock().unwrap();
        let mut buffer = vec![0u8; 128];
        let n = port.read(&mut buffer)?;
        let line = String::from_utf8_lossy(&buffer[..n]).to_string();
        Ok(line)
    }
}

// =============================================================================
// SDK Mode Selection
// =============================================================================

/// SDK mode for Newport 1830C
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newport1830cSdkKind {
    /// Mock serial for testing
    Mock,
    /// Real serial hardware
    Real,
}

// =============================================================================
// Newport 1830C V3
// =============================================================================

/// Newport 1830-C Power Meter V3 implementation
///
/// Unified architecture implementation demonstrating:
/// - Direct `Instrument` + `PowerMeter` trait implementation
/// - `Parameter<T>` for declarative settings
/// - Single broadcast channel for data streaming
/// - Direct async methods (no message passing)
/// - Serial abstraction layer (Mock/Real)
pub struct Newport1830CV3 {
    /// Instrument identifier
    id: String,
    
    /// Current state
    state: InstrumentState,
    
    /// Data broadcast channel
    data_tx: broadcast::Sender<Measurement>,
    
    /// Parameters (for dynamic access via ParameterBase)
    parameters: HashMap<String, Box<dyn ParameterBase>>,
    
    // Serial abstraction
    serial_port: Option<Box<dyn SerialPort>>,
    port_path: String,
    sdk_kind: Newport1830cSdkKind,
    
    // Typed parameters (for direct access via PowerMeter trait)
    wavelength_nm: Arc<RwLock<Parameter<f64>>>,
    range: Arc<RwLock<Parameter<String>>>,
    units: Arc<RwLock<Parameter<String>>>,
}

impl Newport1830CV3 {
    /// Create new Newport 1830C V3 instance
    ///
    /// # Arguments
    /// * `id` - Unique instrument identifier
    /// * `port_path` - Serial port path (e.g., "/dev/ttyUSB0")
    /// * `sdk_kind` - Mock or Real serial mode
    pub fn new(id: impl Into<String>, port_path: impl Into<String>, sdk_kind: Newport1830cSdkKind) -> Self {
        let id = id.into();
        let (data_tx, _) = broadcast::channel(1024);
        
        // Create parameters
        let wavelength_nm = Arc::new(RwLock::new(
            ParameterBuilder::new("wavelength_nm", 1550.0)
                .description("Laser wavelength for power calibration")
                .unit("nm")
                .range(400.0, 1700.0)
                .build(),
        ));
        
        let range = Arc::new(RwLock::new(
            ParameterBuilder::new("range", "auto".to_string())
                .description("Power measurement range")
                .choices(vec![
                    "auto".to_string(),
                    "1uW".to_string(),
                    "10uW".to_string(),
                    "100uW".to_string(),
                    "1mW".to_string(),
                    "10mW".to_string(),
                    "100mW".to_string(),
                    "1W".to_string(),
                ])
                .build(),
        ));
        
        let units = Arc::new(RwLock::new(
            ParameterBuilder::new("units", "W".to_string())
                .description("Power measurement units")
                .choices(vec![
                    "W".to_string(),
                    "dBm".to_string(),
                    "dB".to_string(),
                    "REL".to_string(),
                ])
                .build(),
        ));
        
        Self {
            id,
            state: InstrumentState::Uninitialized,
            data_tx,
            parameters: HashMap::new(),
            serial_port: None,
            port_path: port_path.into(),
            sdk_kind,
            wavelength_nm,
            range,
            units,
        }
    }
    
    /// Send command to power meter
    async fn send_command(&mut self, cmd: &str) -> Result<()> {
        if let Some(port) = &mut self.serial_port {
            let command = format!("{}\r\n", cmd);
            port.write(&command).await
        } else {
            Err(anyhow!("Serial port not initialized"))
        }
    }
    
    /// Read response from power meter
    async fn read_response(&mut self) -> Result<String> {
        if let Some(port) = &mut self.serial_port {
            let response = port.read_line().await?;
            Ok(response.trim().to_string())
        } else {
            Err(anyhow!("Serial port not initialized"))
        }
    }
}

// =============================================================================
// Instrument Trait Implementation
// =============================================================================

#[async_trait]
impl Instrument for Newport1830CV3 {
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
        
        // Initialize serial port based on SDK kind
        match self.sdk_kind {
            Newport1830cSdkKind::Mock => {
                self.serial_port = Some(Box::new(MockSerialPort::new()));
            }
            #[cfg(feature = "instrument_serial")]
            Newport1830cSdkKind::Real => {
                let port = serialport::new(&self.port_path, 9600)
                    .timeout(std::time::Duration::from_millis(100))
                    .open()
                    .map_err(|e| anyhow!("Failed to open {}: {}", self.port_path, e))?;
                self.serial_port = Some(Box::new(RealSerialPort { port: std::sync::Mutex::new(port) }));
            }
            #[cfg(not(feature = "instrument_serial"))]
            Newport1830cSdkKind::Real => {
                return Err(anyhow!("Real serial not available - enable 'instrument_serial' feature"));
            }
        }
        
        // Query identification
        self.send_command("*IDN?").await?;
        let _id_response = self.read_response().await?;
        
        // Set initial wavelength
        let wavelength = self.wavelength_nm.read().await.get();
        self.send_command(&format!("PM:Lambda {}", wavelength)).await?;
        
        self.state = InstrumentState::Idle;
        Ok(())
    }
    
    async fn shutdown(&mut self) -> Result<()> {
        self.state = InstrumentState::ShuttingDown;
        self.serial_port = None;
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
// PowerMeter Trait Implementation
// =============================================================================

#[async_trait]
impl PowerMeter for Newport1830CV3 {
    async fn set_wavelength(&mut self, nm: f64) -> Result<()> {
        // Validate and set parameter (this handles validation)
        self.wavelength_nm.write().await.set(nm).await?;
        
        // Send to hardware if initialized
        if self.state != InstrumentState::Uninitialized {
            self.send_command(&format!("PM:Lambda {}", nm)).await?;
        }
        
        Ok(())
    }
    
    async fn set_range(&mut self, watts: f64) -> Result<()> {
        // Convert watts to range string
        let range_str = if watts >= 1.0 {
            "1W"
        } else if watts >= 0.1 {
            "100mW"
        } else if watts >= 0.01 {
            "10mW"
        } else if watts >= 0.001 {
            "1mW"
        } else if watts >= 0.0001 {
            "100uW"
        } else if watts >= 0.00001 {
            "10uW"
        } else {
            "1uW"
        };
        
        self.range.write().await.set(range_str.to_string()).await?;
        
        // Send to hardware if initialized
        if self.state != InstrumentState::Uninitialized {
            // Map to Newport range index (0=auto, 1-8=ranges)
            let range_idx = if watts >= 1.0 { 1 }
                else if watts >= 0.1 { 2 }
                else if watts >= 0.01 { 3 }
                else if watts >= 0.001 { 4 }
                else if watts >= 0.0001 { 5 }
                else if watts >= 0.00001 { 6 }
                else { 7 };
            
            self.send_command(&format!("PM:Range {}", range_idx)).await?;
        }
        
        Ok(())
    }
    
    async fn zero(&mut self) -> Result<()> {
        if self.state == InstrumentState::Uninitialized {
            return Err(anyhow!("Power meter not initialized"));
        }
        
        // Send zero/dark calibration command
        self.send_command("PM:DS:Clear").await?;
        
        Ok(())
    }
}

// Additional Newport-specific methods (not in PowerMeter trait)
impl Newport1830CV3 {
    /// Read current power and broadcast measurement
    ///
    /// This is not part of the PowerMeter trait in V3 - measurements
    /// are broadcast via data_channel instead of trait methods.
    pub async fn read_power(&mut self) -> Result<f64> {
        if self.state == InstrumentState::Uninitialized {
            return Err(anyhow!("Power meter not initialized"));
        }
        
        // Send power query
        self.send_command("PM:P?").await?;
        
        // Read response
        let response = self.read_response().await?;
        
        // Parse power value (scientific notation: e.g., "1.234567e-03")
        let power: f64 = response
            .trim()
            .parse()
            .map_err(|e| anyhow!("Failed to parse power '{}': {}", response, e))?;
        
        // Broadcast measurement
        let measurement = Measurement::Scalar {
            name: format!("{}_power", self.id),
            value: power,
            unit: self.units.read().await.get(),
            timestamp: Utc::now(),
        };
        let _ = self.data_tx.send(measurement);
        
        Ok(power)
    }
    
    /// Get current wavelength setting
    pub async fn wavelength(&self) -> f64 {
        self.wavelength_nm.read().await.get()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_newport_1830c_v3_initialization() {
        let mut power_meter = Newport1830CV3::new("test_pm", "/dev/tty.mock", Newport1830cSdkKind::Mock);
        assert_eq!(power_meter.state(), InstrumentState::Uninitialized);
        
        power_meter.initialize().await.unwrap();
        assert_eq!(power_meter.state(), InstrumentState::Idle);
    }
    
    #[tokio::test]
    async fn test_newport_1830c_v3_power_reading() {
        let mut power_meter = Newport1830CV3::new("test_pm", "/dev/tty.mock", Newport1830cSdkKind::Mock);
        power_meter.initialize().await.unwrap();
        
        // Subscribe BEFORE reading to ensure we receive the broadcast
        let mut rx = power_meter.data_channel();
        
        // Read power via Newport-specific method
        let power = power_meter.read_power().await.unwrap();
        assert!(power >= 0.0, "Power should be non-negative");
        
        // Check that measurement was broadcast
        tokio::select! {
            result = rx.recv() => {
                let measurement = result.unwrap();
                match measurement {
                    Measurement::Scalar { value, unit, .. } => {
                        assert!(value >= 0.0);
                        assert_eq!(unit, "W");
                    }
                    _ => panic!("Expected Scalar measurement"),
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                panic!("No measurement received");
            }
        }
    }
    
    #[tokio::test]
    async fn test_newport_1830c_v3_wavelength_setting() {
        let mut power_meter = Newport1830CV3::new("test_pm", "/dev/tty.mock", Newport1830cSdkKind::Mock);
        power_meter.initialize().await.unwrap();
        
        // Set valid wavelength using PowerMeter trait method
        power_meter.set_wavelength(633.0).await.unwrap();
        assert_eq!(power_meter.wavelength().await, 633.0);
        
        // Set another valid wavelength
        power_meter.set_wavelength(1064.0).await.unwrap();
        assert_eq!(power_meter.wavelength().await, 1064.0);
    }
    
    #[tokio::test]
    async fn test_newport_1830c_v3_zero_calibration() {
        let mut power_meter = Newport1830CV3::new("test_pm", "/dev/tty.mock", Newport1830cSdkKind::Mock);
        power_meter.initialize().await.unwrap();
        
        // Zero should succeed
        power_meter.zero().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_newport_1830c_v3_parameter_validation() {
        let mut power_meter = Newport1830CV3::new("test_pm", "/dev/tty.mock", Newport1830cSdkKind::Mock);
        power_meter.initialize().await.unwrap();
        
        // Invalid wavelength should fail (below minimum)
        let result = power_meter.set_wavelength(100.0).await;
        assert!(result.is_err(), "Wavelength below 400nm should fail");
        
        // Invalid wavelength should fail (above maximum)
        let result = power_meter.set_wavelength(2000.0).await;
        assert!(result.is_err(), "Wavelength above 1700nm should fail");
        
        // Valid wavelength should work
        power_meter.set_wavelength(800.0).await.unwrap();
        assert_eq!(power_meter.wavelength().await, 800.0);
    }
    
    #[tokio::test]
    async fn test_newport_1830c_v3_shutdown() {
        let mut power_meter = Newport1830CV3::new("test_pm", "/dev/tty.mock", Newport1830cSdkKind::Mock);
        power_meter.initialize().await.unwrap();
        
        power_meter.shutdown().await.unwrap();
        assert_eq!(power_meter.state(), InstrumentState::ShuttingDown);
    }
}