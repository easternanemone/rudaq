//! Thorlabs Elliptec ELL14 rotation mount driver
//!
//! This module provides an `Instrument` implementation for Elliptec ELL14
//! rotation mounts using RS-485 multidrop serial communication.
//!
//! ## Configuration
//!
//! ```toml
//! [instruments.elliptec_rotators]
//! type = "elliptec"
//! port = "/dev/ttyUSB0"
//! baud_rate = 9600
//! device_addresses = [0, 1]  # Multiple devices on same bus
//! polling_rate_hz = 2.0
//! ```
//!
//! ## Protocol Reference
//!
//! The Elliptec protocol uses ASCII commands with device address prefix:
//! Format: `<Address><Command>[Data]`
//! Response: `<Address><Response>[Data]<CR><LF>`
//!
//! ### Get Commands
//! - `in`: Get device info (returns 33-byte response with model, serial, firmware, etc.)
//! - `gs`: Get status (error code)
//! - `gp`: Get position (returns hex position in counts)
//! - `gj`: Get step size (jog step size)
//! - `go`: Get home offset
//! - `i1`, `i2`: Get motor info (for dual-motor devices)
//!
//! ### Movement Commands
//! - `ho0`: Home clockwise
//! - `ho1`: Home counter-clockwise
//! - `ma<hex>`: Move absolute (8-char hex position)
//! - `mr<hex>`: Move relative (8-char signed hex offset)
//! - `fw`: Forward jog
//! - `bw`: Backward jog
//!
//! ### Set Commands
//! - `sj<hex>`: Set step size
//! - `so<hex>`: Set home offset
//! - `ca<addr>`: Change address
//! - `is<0|1>`: Isolate/de-isolate device
//!
//! ### Position Encoding
//! - ELL14: 143360 counts = 360 degrees (398.222... counts/degree)
//! - Position sent/received as 8-character hex string (e.g., "00023000")

#[cfg(feature = "instrument_serial")]
use crate::adapters::serial::SerialAdapter;
use crate::{
    config::Settings,
    core::{DataPoint, Instrument, InstrumentCommand},
    measurement::InstrumentMeasurement,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use std::sync::Arc;

/// Device information parsed from IN command response
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Device address (0-15)
    pub address: u8,
    /// Motor type code (14 = ELL14)
    pub motor_type: u8,
    /// Serial number (8 ASCII characters)
    pub serial_no: String,
    /// Manufacturing year (4 ASCII characters, e.g. "2023")
    pub year: String,
    /// Firmware version (2 hex digits, e.g. "17" = v1.7)
    pub firmware: u8,
    /// Thread type (0 = imperial, 1 = metric)
    pub thread_metric: bool,
    /// Hardware revision (ASCII character)
    pub hardware: char,
    /// Travel range in degrees (ELL14: 360)
    pub range_degrees: u16,
    /// Pulses per revolution (ELL14: 143360)
    pub pulse_per_rev: u32,
}

impl DeviceInfo {
    /// Parse IN command response
    /// Format: "2IN0E1140051720231701016800023000"
    /// - Position 0: Address
    /// - Position 1-2: "IN" command echo
    /// - Position 3-4: Motor type (hex)
    /// - Position 5-12: Serial number (8 chars)
    /// - Position 13-16: Year (4 chars)
    /// - Position 17-18: Firmware (hex)
    /// - Position 19: Thread (0/1)
    /// - Position 20: Hardware revision
    /// - Position 21-24: Range (hex, in degrees)
    /// - Position 25-32: Pulse/rev (hex)
    pub fn parse(response: &str) -> Result<Self> {
        if response.len() < 33 {
            return Err(anyhow!("IN response too short: {} bytes", response.len()));
        }

        let address = response[0..1]
            .parse::<u8>()
            .with_context(|| format!("Invalid address in response: {}", &response[0..1]))?;

        if &response[1..3] != "IN" {
            return Err(anyhow!("Expected 'IN' response, got: {}", &response[1..3]));
        }

        let motor_type = u8::from_str_radix(&response[3..5], 16)
            .with_context(|| format!("Invalid motor type hex: {}", &response[3..5]))?;

        let serial_no = response[5..13].to_string();
        let year = response[13..17].to_string();

        let firmware = u8::from_str_radix(&response[17..19], 16)
            .with_context(|| format!("Invalid firmware hex: {}", &response[17..19]))?;

        let thread_metric = &response[19..20] != "0";
        let hardware = response.chars().nth(20).unwrap_or('?');

        let range_degrees = u16::from_str_radix(&response[21..25], 16)
            .with_context(|| format!("Invalid range hex: {}", &response[21..25]))?;

        let pulse_per_rev = u32::from_str_radix(&response[25..33], 16)
            .with_context(|| format!("Invalid pulse/rev hex: {}", &response[25..33]))?;

        // Validate critical fields to prevent division by zero
        if range_degrees == 0 {
            return Err(anyhow!("Invalid device info: range_degrees is zero"));
        }
        if pulse_per_rev == 0 {
            return Err(anyhow!("Invalid device info: pulse_per_rev is zero"));
        }

        Ok(Self {
            address,
            motor_type,
            serial_no,
            year,
            firmware,
            thread_metric,
            hardware,
            range_degrees,
            pulse_per_rev,
        })
    }

    /// Get human-readable motor type name
    pub fn motor_type_name(&self) -> String {
        match self.motor_type {
            14 => "ELL14 Rotation Mount".to_string(),
            _ => format!("ELL{:02X}", self.motor_type),
        }
    }

    /// Get conversion factor (counts per degree)
    pub fn counts_per_degree(&self) -> f64 {
        self.pulse_per_rev as f64 / self.range_degrees as f64
    }
}

/// Elliptec ELL14 instrument implementation
#[derive(Clone)]
pub struct Elliptec {
    id: String,
    #[cfg(feature = "instrument_serial")]
    adapter: Option<SerialAdapter>,
    device_addresses: Vec<u8>,
    device_info: std::collections::HashMap<u8, DeviceInfo>,
    measurement: Option<InstrumentMeasurement>,
}

impl Elliptec {
    /// Creates a new Elliptec instrument
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            #[cfg(feature = "instrument_serial")]
            adapter: None,
            device_addresses: vec![0], // Default to address 0
            device_info: std::collections::HashMap::new(),
            measurement: None,
        }
    }

    #[cfg(feature = "instrument_serial")]
    async fn send_command_async(&self, address: u8, command: &str) -> Result<String> {
        use super::serial_helper;
        use std::time::Duration;

        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected to Elliptec '{}'", self.id))?
            .clone();

        // Elliptec protocol: address + command
        let cmd = format!("{}{}", address, command);

        serial_helper::send_command_async(
            adapter,
            &self.id,
            &cmd,
            "",
            Duration::from_millis(500),
            b'\r',
        )
        .await
    }

    #[cfg(feature = "instrument_serial")]
    async fn get_position(&self, address: u8) -> Result<f64> {
        // 'gp' command - get position
        let response = self.send_command_async(address, "gp").await?;

        // Response format: "0PO12345678" where address=0, PO is response code, 12345678 is hex position
        if response.len() < 10 {
            return Err(anyhow!("Invalid position response: {}", response));
        }

        let hex_pos = &response[3..]; // Skip address and "PO"
        let raw_pos = u32::from_str_radix(hex_pos, 16)
            .with_context(|| format!("Failed to parse hex position: {}", hex_pos))?;

        // Convert to degrees using device-specific conversion factor
        let degrees = if let Some(info) = self.device_info.get(&address) {
            // Use device-specific conversion
            (raw_pos as f64 / info.pulse_per_rev as f64) * info.range_degrees as f64
        } else {
            // Fallback to ELL14 defaults if device info not available
            warn!(
                "Using default ELL14 conversion for device {} (device info not available)",
                address
            );
            (raw_pos as f64 / 143360.0) * 360.0
        };
        Ok(degrees)
    }

    #[cfg(feature = "instrument_serial")]
    async fn set_position(&self, address: u8, degrees: f64) -> Result<()> {
        // Convert degrees to counts using device-specific conversion factor
        let counts = if let Some(info) = self.device_info.get(&address) {
            // Use device-specific conversion
            ((degrees / info.range_degrees as f64) * info.pulse_per_rev as f64) as u32
        } else {
            // Fallback to ELL14 defaults if device info not available
            warn!(
                "Using default ELL14 conversion for device {} (device info not available)",
                address
            );
            ((degrees / 360.0) * 143360.0) as u32
        };
        let hex_pos = format!("{:08X}", counts);

        // 'ma' command - move absolute
        self.send_command_async(address, &format!("ma{}", hex_pos))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Instrument for Elliptec {
    type Measure = InstrumentMeasurement;

    fn name(&self) -> String {
        self.id.clone()
    }

    #[cfg(feature = "instrument_serial")]
    async fn connect(&mut self, id: &str, settings: &Arc<Settings>) -> Result<()> {
        info!("Connecting to Elliptec rotators: {}", id);
        self.id = id.to_string();

        let instrument_config = settings
            .instruments
            .get(id)
            .ok_or_else(|| anyhow!("Configuration for '{}' not found", id))?;

        let port_name = instrument_config
            .get("port")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("'port' not found in config for '{}'", self.id))?;

        let baud_rate = instrument_config
            .get("baud_rate")
            .and_then(|v| v.as_integer())
            .unwrap_or(9600) as u32;

        // Get device addresses
        self.device_addresses = instrument_config
            .get("device_addresses")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_integer())
                    .map(|i| i as u8)
                    .collect()
            })
            .unwrap_or_else(|| vec![0]);

        info!("Elliptec device addresses: {:?}", self.device_addresses);

        // Open serial port (RS-485 multidrop does NOT use hardware flow control)
        let port = serialport::new(port_name, baud_rate)
            .timeout(std::time::Duration::from_millis(100))
            .flow_control(serialport::FlowControl::None) // RS-485 does not use RTS/CTS
            .open()
            .with_context(|| format!("Failed to open serial port '{}' for Elliptec", port_name))?;

        self.adapter = Some(SerialAdapter::new(port));

        // Query device info for each address
        for &addr in &self.device_addresses {
            let response = self.send_command_async(addr, "in").await?;
            info!("Elliptec device {} info: {}", addr, response);

            // Parse and store device info
            match DeviceInfo::parse(&response) {
                Ok(info) => {
                    info!(
                        "Elliptec device {}: {} S/N:{} FW:v{}.{} Year:{} Range:{}° PPR:{}",
                        addr,
                        info.motor_type_name(),
                        info.serial_no,
                        info.firmware / 16,
                        info.firmware % 16,
                        info.year,
                        info.range_degrees,
                        info.pulse_per_rev
                    );
                    self.device_info.insert(addr, info);
                }
                Err(e) => {
                    warn!("Failed to parse device info for device {}: {}", addr, e);
                    // Continue anyway - device might still work with default conversion
                }
            }
        }

        // Create measurement distributor with configured capacity
        let capacity = settings.application.broadcast_channel_capacity;
        let measurement = InstrumentMeasurement::new(capacity, self.id.clone());
        self.measurement = Some(measurement.clone());

        // Spawn polling task
        let instrument = self.clone();
        let polling_rate = instrument_config
            .get("polling_rate_hz")
            .and_then(|v| v.as_float())
            .unwrap_or(2.0);

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs_f64(1.0 / polling_rate));

            loop {
                interval.tick().await;

                let timestamp = chrono::Utc::now();

                // Poll each device
                for &addr in &instrument.device_addresses {
                    match instrument.get_position(addr).await {
                        Ok(position) => {
                            let dp = DataPoint {
                                timestamp,
                                instrument_id: instrument.id.clone(),
                                channel: format!("device{}_position", addr),
                                value: position,
                                unit: "deg".to_string(),
                                metadata: Some(serde_json::json!({"device_address": addr})),
                            };

                            if measurement.broadcast(dp).await.is_err() {
                                warn!("No active receivers for Elliptec data");
                                return;
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to read position from Elliptec device {}: {}",
                                addr, e
                            );
                        }
                    }
                }
            }
        });

        info!("Elliptec rotators '{}' connected successfully", self.id);
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn connect(&mut self, id: &str, _settings: &Arc<Settings>) -> Result<()> {
        self.id = id.to_string();
        Err(anyhow!(
            "Serial support not enabled. Rebuild with --features instrument_serial"
        ))
    }

    async fn disconnect(&mut self) -> Result<()> {
        info!("Disconnecting from Elliptec rotators: {}", self.id);
        #[cfg(feature = "instrument_serial")]
        {
            self.adapter = None;
        }
        self.measurement = None;
        Ok(())
    }

    fn measure(&self) -> &Self::Measure {
        self.measurement
            .as_ref()
            .expect("Elliptec measurement not initialised")
    }

    #[cfg(feature = "instrument_serial")]
    async fn handle_command(&mut self, command: InstrumentCommand) -> Result<()> {
        match command {
            InstrumentCommand::SetParameter(key, value) => {
                // Parse device_address:parameter format
                let parts: Vec<&str> = key.split(':').collect();
                if parts.len() == 2 {
                    let addr: u8 = parts[0]
                        .parse()
                        .with_context(|| format!("Invalid device address: {}", parts[0]))?;

                    if parts[1] == "position" {
                        let degrees: f64 = value
                            .as_f64()
                            .with_context(|| format!("Invalid position value: {}", value))?;
                        self.set_position(addr, degrees).await?;
                        info!("Set Elliptec device {} to {} degrees", addr, degrees);
                    }
                } else {
                    warn!("Unknown parameter '{}' for Elliptec", key);
                }
            }
            InstrumentCommand::Execute(cmd, args) => {
                if cmd == "home" {
                    if args.is_empty() {
                        // Home all devices
                        let mut batch = self.adapter.as_mut().unwrap().start_batch();
                        for &addr in &self.device_addresses {
                            batch.queue(format!("{}ho", addr));
                        }
                        batch.flush().await?;
                        info!("Homed all Elliptec devices");
                    } else {
                        // Home specific device
                        let addr: u8 = args[0]
                            .parse()
                            .with_context(|| format!("Invalid device address: {}", args[0]))?;
                        self.send_command_async(addr, "ho").await?;
                        info!("Homed Elliptec device {}", addr);
                    }
                }
            }
            _ => {
                warn!("Unsupported command type for Elliptec");
            }
        }
        Ok(())
    }

    #[cfg(not(feature = "instrument_serial"))]
    async fn handle_command(&mut self, _command: InstrumentCommand) -> Result<()> {
        Err(anyhow!("Serial support not enabled"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_info_parse_device_2() {
        // Actual response from device 2 in integration test
        let response = "2IN0E1140051720231701016800023000";

        let info = DeviceInfo::parse(response).expect("Failed to parse device 2 info");

        assert_eq!(info.address, 2);
        assert_eq!(info.motor_type, 0x0E); // 14 decimal = ELL14
        assert_eq!(info.serial_no, "11400517");
        assert_eq!(info.year, "2023");
        assert_eq!(info.firmware, 0x17); // 23 decimal = v2.3
        assert_eq!(info.thread_metric, false);
        assert_eq!(info.hardware, '1'); // Hardware revision 1
        assert_eq!(info.range_degrees, 0x0168); // 360 decimal
        assert_eq!(info.pulse_per_rev, 0x00023000); // 143360 decimal
        assert_eq!(info.motor_type_name(), "ELL14 Rotation Mount");

        // Verify conversion factor
        let expected_cpd = 143360.0 / 360.0; // 398.222...
        assert!((info.counts_per_degree() - expected_cpd).abs() < 0.001);
    }

    #[test]
    fn test_device_info_parse_device_3() {
        // Actual response from device 3 in integration test
        let response = "3IN0E1140028420211501016800023000";

        let info = DeviceInfo::parse(response).expect("Failed to parse device 3 info");

        assert_eq!(info.address, 3);
        assert_eq!(info.motor_type, 0x0E); // 14 decimal = ELL14
        assert_eq!(info.serial_no, "11400284");
        assert_eq!(info.year, "2021");
        assert_eq!(info.firmware, 0x15); // 21 decimal = v2.1
        assert_eq!(info.thread_metric, false);
        assert_eq!(info.hardware, '1'); // Hardware revision 1
        assert_eq!(info.range_degrees, 0x0168); // 360 decimal
        assert_eq!(info.pulse_per_rev, 0x00023000); // 143360 decimal
    }

    #[test]
    fn test_device_info_parse_invalid_length() {
        let response = "2IN0E1140051"; // Too short

        let result = DeviceInfo::parse(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn test_device_info_parse_invalid_command() {
        let response = "2XX0E1140051720231701016800023000";

        let result = DeviceInfo::parse(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Expected 'IN'"));
    }

    #[test]
    fn test_device_info_parse_zero_range() {
        // Response with range_degrees = 0 (positions 21-24)
        let response = "2IN0E1140051720231701000000023000";

        let result = DeviceInfo::parse(response);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("range_degrees is zero"));
    }

    #[test]
    fn test_device_info_parse_zero_pulse_per_rev() {
        // Response with pulse_per_rev = 0 (positions 25-32)
        let response = "2IN0E1140051720231701016800000000";

        let result = DeviceInfo::parse(response);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("pulse_per_rev is zero"));
    }

    #[test]
    fn test_position_conversion_ell14() {
        // Test the position conversion formula
        let info = DeviceInfo {
            address: 2,
            motor_type: 14,
            serial_no: "11400517".to_string(),
            year: "2023".to_string(),
            firmware: 0x17,
            thread_metric: false,
            hardware: '0',
            range_degrees: 360,
            pulse_per_rev: 143360,
        };

        // Test full rotation
        let counts_full = info.pulse_per_rev;
        let degrees_full =
            (counts_full as f64 / info.pulse_per_rev as f64) * info.range_degrees as f64;
        assert!((degrees_full - 360.0).abs() < 0.001);

        // Test half rotation
        let counts_half = info.pulse_per_rev / 2;
        let degrees_half =
            (counts_half as f64 / info.pulse_per_rev as f64) * info.range_degrees as f64;
        assert!((degrees_half - 180.0).abs() < 0.001);

        // Test quarter rotation
        let counts_quarter = info.pulse_per_rev / 4;
        let degrees_quarter =
            (counts_quarter as f64 / info.pulse_per_rev as f64) * info.range_degrees as f64;
        assert!((degrees_quarter - 90.0).abs() < 0.001);
    }
}
