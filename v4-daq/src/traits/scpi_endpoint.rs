//! ScpiEndpoint meta-instrument trait
//!
//! Hardware-agnostic interface for SCPI command execution.
//! Implementations handle protocol-specific details (VISA, serial, Ethernet, etc.).

use anyhow::Result;
use arrow::array::{BooleanArray, StringArray, TimestampNanosecondArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::sync::Arc;
use std::time::Duration;

/// SCPI command event (for streaming and logging)
#[derive(Debug, Clone, kameo::Reply)]
pub struct ScpiEvent {
    /// Timestamp of command execution (ns)
    pub timestamp_ns: i64,
    /// SCPI command sent
    pub command: String,
    /// Response from instrument (if any)
    pub response: Option<String>,
    /// Success or error
    pub success: bool,
    /// Error message (if failed)
    pub error: Option<String>,
}

/// SCPI endpoint meta-instrument trait
///
/// Hardware-agnostic interface for SCPI command execution.
/// Implementations handle protocol-specific details (VISA, serial, Ethernet, etc.).
///
/// ## Instrument Categories
///
/// ### Type A: Query-Response (Standard SCPI)
/// - Supports both SET and GET commands
/// - Example: `VOLT?` returns voltage reading
/// - Most oscilloscopes, power supplies, meters
/// - Queries return actual hardware state
///
/// ### Type B: SET-Only (Configuration-Only)
/// - Supports SET commands but not GET queries
/// - GET methods return cached state from last SET
/// - Example: Some laser controllers, legacy instruments
/// - Warning logged when query fails but SET succeeded
/// - Implementations should detect instrument type and adapt behavior
///
/// ## Command Queuing
/// - Adapter queues commands internally with `tokio::sync::Mutex`
/// - No explicit queue depth limit (relies on backpressure)
/// - Commands executed sequentially in FIFO order
///
/// ## Error Recovery
/// - Failed queries do NOT auto-clear errors
/// - Caller must explicitly call `clear_errors()` for recovery
/// - Persistent errors may require `reset()` to factory defaults
///
/// ## Status Polling
/// - `transact()` polls *STB? every 50ms
/// - Timeout applies to total transaction time
/// - Returns when status byte indicates completion
#[async_trait]
pub trait ScpiEndpoint: Send + Sync {
    /// Send command and read response
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string (e.g., "*IDN?", "MEAS:VOLT:DC?")
    ///
    /// # Behavior
    /// - Sends command and waits for response
    /// - Returns full response string
    /// - Uses adapter's default timeout
    ///
    /// # Errors
    /// - Hardware communication error
    /// - Timeout (implementation-dependent)
    /// - Malformed response
    async fn query(&self, cmd: &str) -> Result<String>;

    /// Send command with explicit timeout
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string
    /// * `timeout` - Maximum time to wait for response
    ///
    /// # Errors
    /// - Hardware communication error
    /// - Timeout exceeded
    /// - Malformed response
    async fn query_with_timeout(&self, cmd: &str, timeout: Duration) -> Result<String>;

    /// Send command without expecting response
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string (e.g., "*RST", "*CLS")
    ///
    /// # Behavior
    /// - Sends command and returns immediately
    /// - No response is read
    ///
    /// # Errors
    /// - Hardware communication error
    async fn write(&self, cmd: &str) -> Result<()>;

    /// Send command and verify execution with *STB? check
    ///
    /// # Arguments
    /// * `cmd` - SCPI command string
    /// * `timeout` - Maximum time to wait
    ///
    /// # Behavior
    /// - Sends command
    /// - Polls *STB? (service request / status byte) until ready or timeout
    /// - Returns when instrument indicates completion
    ///
    /// # Errors
    /// - Hardware communication error
    /// - Timeout
    /// - Error bit set in status
    async fn transact(&self, cmd: &str, timeout: Duration) -> Result<()>;

    /// Query instrument error (ESR? - Event Status Register)
    ///
    /// # Returns
    /// - Error code from instrument
    /// - 0 = no error, non-zero = error condition
    ///
    /// # Errors
    /// - Hardware communication error
    async fn read_error(&self) -> Result<u8>;

    /// Clear error queue and event status register (*CLS)
    ///
    /// # Errors
    /// - Hardware communication error
    async fn clear_errors(&self) -> Result<()>;

    /// Reset instrument to factory defaults (*RST)
    ///
    /// # Errors
    /// - Hardware communication error
    async fn reset(&self) -> Result<()>;

    /// Query instrument identity (*IDN?)
    ///
    /// # Returns
    /// - Identity string (e.g., "Agilent Technologies,34401A,1234567,5.01")
    ///
    /// # Errors
    /// - Hardware communication error
    async fn identify(&self) -> Result<String>;

    /// Get current timeout setting
    fn get_timeout(&self) -> Duration;

    /// Set default timeout for queries
    ///
    /// # Arguments
    /// * `timeout` - New timeout duration
    fn set_timeout(&self, timeout: Duration);

    /// Serialize SCPI events to Arrow RecordBatch
    ///
    /// # Arguments
    /// * `events` - Slice of events to serialize
    ///
    /// # Output Schema
    /// ```text
    /// timestamp (i64 ns, Timestamp(Nanosecond))
    /// command (utf8)
    /// response (utf8, nullable)
    /// success (bool)
    /// error (utf8, nullable)
    /// ```
    fn to_arrow_events(&self, events: &[ScpiEvent]) -> Result<RecordBatch> {
        static SCHEMA: Lazy<Arc<Schema>> = Lazy::new(|| {
            Arc::new(Schema::new(vec![
                Field::new(
                    "timestamp",
                    DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                    false,
                ),
                Field::new("command", DataType::Utf8, false),
                Field::new("response", DataType::Utf8, true),  // Nullable
                Field::new("success", DataType::Boolean, false),
                Field::new("error", DataType::Utf8, true),  // Nullable
            ]))
        });

        let timestamps: Vec<i64> = events.iter().map(|e| e.timestamp_ns).collect();
        let commands: StringArray = events.iter().map(|e| Some(e.command.as_str())).collect();
        let responses: StringArray = events
            .iter()
            .map(|e| e.response.as_ref().map(|s| s.as_str()))
            .collect();
        let successes: Vec<bool> = events.iter().map(|e| e.success).collect();
        let errors: StringArray = events
            .iter()
            .map(|e| e.error.as_ref().map(|s| s.as_str()))
            .collect();

        let batch = RecordBatch::try_new(
            SCHEMA.clone(),
            vec![
                Arc::new(TimestampNanosecondArray::from(timestamps)),
                Arc::new(commands),
                Arc::new(responses),
                Arc::new(BooleanArray::from(successes)),
                Arc::new(errors),
            ],
        )?;

        Ok(batch)
    }
}
