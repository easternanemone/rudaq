//! SCPI over TCP communication helpers for Red Pitaya
//!
//! This module provides an async SCPI client for communicating with Red Pitaya
//! devices over TCP. It handles connection management, command/query operations,
//! and response parsing.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time::timeout;

/// Default SCPI port for Red Pitaya
pub const DEFAULT_PORT: u16 = 5000;

/// Default command timeout in milliseconds
pub const DEFAULT_TIMEOUT_MS: u64 = 2000;

/// Async SCPI client for TCP communication with Red Pitaya
pub struct ScpiClient {
    stream: Mutex<BufReader<TcpStream>>,
    timeout: Duration,
}

impl ScpiClient {
    /// Create a new SCPI client connected to the specified host and port.
    ///
    /// # Arguments
    /// * `host` - Hostname or IP address
    /// * `port` - TCP port (typically 5000 for Red Pitaya)
    ///
    /// # Returns
    /// * `Ok(ScpiClient)` on successful connection
    /// * `Err` if connection fails
    pub async fn connect(host: &str, port: u16) -> Result<Self> {
        let addr: SocketAddr = format!("{}:{}", host, port)
            .parse()
            .with_context(|| format!("Invalid address: {}:{}", host, port))?;

        let stream = timeout(Duration::from_secs(5), TcpStream::connect(addr))
            .await
            .with_context(|| format!("Connection timeout to {}:{}", host, port))?
            .with_context(|| format!("Failed to connect to {}:{}", host, port))?;

        // Disable Nagle's algorithm for low latency
        stream.set_nodelay(true)?;

        tracing::info!("Connected to Red Pitaya at {}:{}", host, port);

        Ok(Self {
            stream: Mutex::new(BufReader::new(stream)),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        })
    }

    /// Set the command timeout duration.
    pub fn set_timeout(&mut self, duration: Duration) {
        self.timeout = duration;
    }

    /// Send a command without expecting a response.
    ///
    /// # Arguments
    /// * `command` - SCPI command string (e.g., "PID:SETP 1.5")
    pub async fn write(&self, command: &str) -> Result<()> {
        let mut stream = self.stream.lock().await;

        let cmd = format!("{}\r\n", command);
        tracing::debug!("SCPI write: {:?}", cmd.trim());

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .with_context(|| format!("Failed to write command: {}", command))?;

        stream
            .get_mut()
            .flush()
            .await
            .context("Failed to flush stream")?;

        // Small delay for command processing
        tokio::time::sleep(Duration::from_millis(10)).await;

        Ok(())
    }

    /// Send a query and read the response.
    ///
    /// # Arguments
    /// * `query` - SCPI query string (e.g., "PID:INP?")
    ///
    /// # Returns
    /// Trimmed response string
    pub async fn query(&self, query: &str) -> Result<String> {
        let mut stream = self.stream.lock().await;

        // Clear any pending data in the buffer
        Self::flush_input_buffer(&mut stream).await?;

        // Send query
        let cmd = format!("{}\r\n", query);
        tracing::debug!("SCPI query: {:?}", cmd.trim());

        stream
            .get_mut()
            .write_all(cmd.as_bytes())
            .await
            .with_context(|| format!("Failed to write query: {}", query))?;

        stream
            .get_mut()
            .flush()
            .await
            .context("Failed to flush stream")?;

        // Read response with timeout
        let mut response = String::new();
        let result = timeout(self.timeout, stream.read_line(&mut response)).await;

        match result {
            Ok(Ok(0)) => anyhow::bail!("Connection closed by device"),
            Ok(Ok(_)) => {
                let trimmed = response.trim().to_string();
                tracing::debug!("SCPI response: {:?}", trimmed);
                Ok(trimmed)
            }
            Ok(Err(e)) => Err(e).context("Failed to read response"),
            Err(_) => anyhow::bail!("Timeout waiting for response to: {}", query),
        }
    }

    /// Query a floating-point value.
    ///
    /// # Arguments
    /// * `query` - SCPI query string
    ///
    /// # Returns
    /// Parsed f64 value
    pub async fn query_f64(&self, query: &str) -> Result<f64> {
        let response = self.query(query).await?;
        response.parse::<f64>().with_context(|| {
            format!(
                "Failed to parse '{}' as f64 from query: {}",
                response, query
            )
        })
    }

    /// Query a boolean value (ON/OFF or 1/0).
    ///
    /// # Arguments
    /// * `query` - SCPI query string
    ///
    /// # Returns
    /// Parsed boolean value
    pub async fn query_bool(&self, query: &str) -> Result<bool> {
        let response = self.query(query).await?;
        match response.to_uppercase().as_str() {
            "ON" | "1" | "TRUE" => Ok(true),
            "OFF" | "0" | "FALSE" => Ok(false),
            _ => anyhow::bail!(
                "Failed to parse '{}' as boolean from query: {}",
                response,
                query
            ),
        }
    }

    /// Clear any pending data from the input buffer.
    async fn flush_input_buffer(stream: &mut BufReader<TcpStream>) -> Result<()> {
        // Consume any data in BufReader's internal buffer
        {
            let buf = stream.buffer();
            if !buf.is_empty() {
                tracing::debug!("Flushing {} bytes from buffer", buf.len());
                let len = buf.len();
                stream.consume(len);
            }
        }

        // Try to read any pending data from the socket
        let mut discard = vec![0u8; 256];
        loop {
            match timeout(
                Duration::from_millis(10),
                stream.get_mut().peek(&mut discard),
            )
            .await
            {
                Ok(Ok(0)) | Err(_) => break, // No data or timeout
                Ok(Ok(n)) => {
                    // Data available, consume it
                    let mut consume_buf = vec![0u8; n];
                    let _ = stream.get_mut().try_read(&mut consume_buf);
                    tracing::debug!("Flushed {} stale bytes from stream", n);
                }
                Ok(Err(_)) => break,
            }
        }

        Ok(())
    }
}

/// Mock SCPI client for testing without hardware
pub struct MockScpiClient {
    // Simulated PID state
    power: Mutex<f64>,
    setpoint: Mutex<f64>,
    kp: Mutex<f64>,
    ki: Mutex<f64>,
    kd: Mutex<f64>,
    output_min: Mutex<f64>,
    output_max: Mutex<f64>,
    enabled: Mutex<bool>,
}

impl Default for MockScpiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockScpiClient {
    /// Create a new mock SCPI client with default values.
    pub fn new() -> Self {
        Self {
            power: Mutex::new(1.0),
            setpoint: Mutex::new(1.0),
            kp: Mutex::new(1.0),
            ki: Mutex::new(0.1),
            kd: Mutex::new(0.0),
            output_min: Mutex::new(0.0),
            output_max: Mutex::new(1.0),
            enabled: Mutex::new(false),
        }
    }

    /// Send a command without expecting a response.
    pub async fn write(&self, command: &str) -> Result<()> {
        tracing::debug!("Mock SCPI write: {}", command);
        self.parse_write_command(command).await
    }

    /// Send a query and read the response.
    pub async fn query(&self, query: &str) -> Result<String> {
        tracing::debug!("Mock SCPI query: {}", query);
        self.handle_query(query).await
    }

    /// Query a floating-point value.
    pub async fn query_f64(&self, query: &str) -> Result<f64> {
        let response = self.query(query).await?;
        response
            .parse::<f64>()
            .with_context(|| format!("Failed to parse '{}' as f64", response))
    }

    /// Query a boolean value.
    pub async fn query_bool(&self, query: &str) -> Result<bool> {
        let response = self.query(query).await?;
        match response.to_uppercase().as_str() {
            "ON" | "1" | "TRUE" => Ok(true),
            "OFF" | "0" | "FALSE" => Ok(false),
            _ => anyhow::bail!("Failed to parse '{}' as boolean", response),
        }
    }

    /// Parse and execute a write command.
    async fn parse_write_command(&self, command: &str) -> Result<()> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }

        match parts[0].to_uppercase().as_str() {
            "PID:SETP" if parts.len() > 1 => {
                let value: f64 = parts[1].parse()?;
                *self.setpoint.lock().await = value;
            }
            "PID:KP" if parts.len() > 1 => {
                let value: f64 = parts[1].parse()?;
                *self.kp.lock().await = value;
            }
            "PID:KI" if parts.len() > 1 => {
                let value: f64 = parts[1].parse()?;
                *self.ki.lock().await = value;
            }
            "PID:KD" if parts.len() > 1 => {
                let value: f64 = parts[1].parse()?;
                *self.kd.lock().await = value;
            }
            "PID:OMIN" if parts.len() > 1 => {
                let value: f64 = parts[1].parse()?;
                *self.output_min.lock().await = value;
            }
            "PID:OMAX" if parts.len() > 1 => {
                let value: f64 = parts[1].parse()?;
                *self.output_max.lock().await = value;
            }
            "PID:EN" if parts.len() > 1 => {
                let enabled = matches!(parts[1].to_uppercase().as_str(), "ON" | "1");
                *self.enabled.lock().await = enabled;
            }
            _ => tracing::warn!("Unknown mock command: {}", command),
        }
        Ok(())
    }

    /// Handle a query and return the mock response.
    async fn handle_query(&self, query: &str) -> Result<String> {
        // Simulate PID behavior: adjust power based on setpoint and enabled state
        if *self.enabled.lock().await {
            let setpoint = *self.setpoint.lock().await;
            let current_power = *self.power.lock().await;
            // Simple mock: power slowly approaches setpoint
            let new_power = current_power + (setpoint - current_power) * 0.1;
            *self.power.lock().await = new_power;
        }

        match query.to_uppercase().as_str() {
            "PID:INP?" => Ok(format!("{:.6}", *self.power.lock().await)),
            "PID:SETP?" => Ok(format!("{:.6}", *self.setpoint.lock().await)),
            "PID:ERR?" => {
                let setpoint = *self.setpoint.lock().await;
                let power = *self.power.lock().await;
                Ok(format!("{:.6}", setpoint - power))
            }
            "PID:OUT?" => {
                // Mock output: proportional to error
                let setpoint = *self.setpoint.lock().await;
                let power = *self.power.lock().await;
                let kp = *self.kp.lock().await;
                let output = (setpoint - power) * kp;
                let output_min = *self.output_min.lock().await;
                let output_max = *self.output_max.lock().await;
                let clamped = output.clamp(output_min, output_max);
                Ok(format!("{:.6}", clamped))
            }
            "PID:KP?" => Ok(format!("{:.6}", *self.kp.lock().await)),
            "PID:KI?" => Ok(format!("{:.6}", *self.ki.lock().await)),
            "PID:KD?" => Ok(format!("{:.6}", *self.kd.lock().await)),
            "PID:OMIN?" => Ok(format!("{:.6}", *self.output_min.lock().await)),
            "PID:OMAX?" => Ok(format!("{:.6}", *self.output_max.lock().await)),
            "PID:EN?" => {
                if *self.enabled.lock().await {
                    Ok("ON".to_string())
                } else {
                    Ok("OFF".to_string())
                }
            }
            "*IDN?" => Ok("Red Pitaya,MOCK,00000,0.0.0".to_string()),
            _ => anyhow::bail!("Unknown mock query: {}", query),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_client_queries() {
        let client = MockScpiClient::new();

        // Test initial values
        let power = client.query_f64("PID:INP?").await.unwrap();
        assert!((power - 1.0).abs() < 0.001);

        let enabled = client.query_bool("PID:EN?").await.unwrap();
        assert!(!enabled);
    }

    #[tokio::test]
    async fn test_mock_client_write_and_query() {
        let client = MockScpiClient::new();

        // Set setpoint
        client.write("PID:SETP 2.5").await.unwrap();
        let setpoint = client.query_f64("PID:SETP?").await.unwrap();
        assert!((setpoint - 2.5).abs() < 0.001);

        // Set PID gains
        client.write("PID:KP 1.5").await.unwrap();
        let kp = client.query_f64("PID:KP?").await.unwrap();
        assert!((kp - 1.5).abs() < 0.001);

        // Enable PID
        client.write("PID:EN ON").await.unwrap();
        let enabled = client.query_bool("PID:EN?").await.unwrap();
        assert!(enabled);
    }

    #[tokio::test]
    async fn test_mock_client_error_signal() {
        let client = MockScpiClient::new();

        // With default values (power=1.0, setpoint=1.0), error should be ~0
        let error = client.query_f64("PID:ERR?").await.unwrap();
        assert!(error.abs() < 0.001);

        // Change setpoint
        client.write("PID:SETP 2.0").await.unwrap();
        let error = client.query_f64("PID:ERR?").await.unwrap();
        assert!((error - 1.0).abs() < 0.001); // setpoint - power = 2.0 - 1.0 = 1.0
    }
}
