//! Generic SCPI Instrument Actor (V4)
//!
//! Kameo actor implementing ScpiEndpoint trait for any SCPI-compliant instrument.
//! Supports GPIB, USB, Ethernet, and other VISA-compatible protocols.
//!
//! ## Supported Instruments
//!
//! Any instrument implementing SCPI (Standard Commands for Programmable Instruments):
//! - Oscilloscopes (Keysight, Tektronix, Rohde & Schwarz, etc.)
//! - Power supplies (Keysight, Rigol, etc.)
//! - Multimeters (Keysight 34401A, etc.)
//! - Function generators (Keysight 33500B, etc.)
//! - Spectrum analyzers
//! - Network analyzers
//!
//! ## Example Usage
//!
//! ```no_run
//! use kameo::prelude::*;
//! use v4_daq::actors::ScpiActor;
//! use v4_daq::hardware::VisaAdapterV4Builder;
//! use std::time::Duration;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create VISA adapter for instrument
//! let adapter = VisaAdapterV4Builder::new("TCPIP0::192.168.1.100::INSTR".to_string())
//!     .with_timeout(Duration::from_secs(2))
//!     .build()
//!     .await?;
//!
//! // Spawn SCPI actor
//! let actor = ScpiActor {
//!     id: "keysight_34401a".to_string(),
//!     adapter: Some(adapter),
//!     timeout: Duration::from_secs(2),
//!     identity: None,
//! };
//!
//! let actor_ref = kameo::spawn(actor);
//!
//! // Query instrument identity
//! let idn = actor_ref.ask(Identify).await??;
//! println!("Instrument: {}", idn);
//!
//! // Measure DC voltage
//! let voltage_str = actor_ref.ask(Query { cmd: "MEAS:VOLT:DC?" }).await??;
//! let voltage: f64 = voltage_str.parse()?;
//! println!("Voltage: {:.6} V", voltage);
//! # Ok(())
//! # }
//! ```

use crate::hardware::{VisaAdapterV4, VisaAdapterV4Builder};
use crate::traits::{ScpiEndpoint, ScpiEvent};
use anyhow::{Context as AnyhowContext, Result};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use kameo::error::BoxSendError;
use kameo::message::{Context, Message};
use kameo::prelude::*;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Generic SCPI instrument actor
///
/// Implements ScpiEndpoint trait for any SCPI-compliant instrument.
/// Uses VisaAdapterV4 for hardware communication.
pub struct ScpiActor {
    /// Unique instrument identifier
    pub id: String,

    /// VISA adapter (None = mock mode)
    pub adapter: Option<VisaAdapterV4>,

    /// Default timeout for SCPI operations
    pub timeout: Duration,

    /// Cached instrument identity (*IDN? response)
    pub identity: Option<String>,
}

impl ScpiActor {
    /// Create a new SCPI actor with VISA adapter
    ///
    /// # Arguments
    /// * `id` - Unique instrument identifier
    /// * `resource` - VISA resource string (e.g., "TCPIP0::192.168.1.100::INSTR")
    /// * `timeout` - Default timeout for SCPI operations
    pub async fn new(id: String, resource: String, timeout: Duration) -> Result<Self> {
        let adapter = VisaAdapterV4Builder::new(resource)
            .with_timeout(timeout)
            .build()
            .await?;

        Ok(Self {
            id,
            adapter: Some(adapter),
            timeout,
            identity: None,
        })
    }

    /// Create a mock SCPI actor (no hardware)
    pub fn mock(id: String) -> Self {
        Self {
            id,
            adapter: None,
            timeout: Duration::from_secs(1),
            identity: Some("Mock SCPI Instrument".to_string()),
        }
    }
}

impl Actor for ScpiActor {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(
        args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        info!("SCPI actor {} starting", args.id);

        // Query instrument identity on startup
        let mut actor = args;

        if actor.adapter.is_some() {
            match actor.identify().await {
                Ok(idn) => {
                    info!("SCPI actor {} connected: {}", actor.id, idn);
                    actor.identity = Some(idn);
                }
                Err(e) => {
                    warn!("SCPI actor {} failed to read identity: {}", actor.id, e);
                }
            }
        }

        Ok(actor)
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        info!("SCPI actor {} stopping", self.id);
        Ok(())
    }
}

// ============================================================================
// ScpiEndpoint Trait Implementation
// ============================================================================

#[async_trait]
impl ScpiEndpoint for ScpiActor {
    async fn query(&self, cmd: &str) -> Result<String> {
        debug!("SCPI query: {}", cmd);

        if let Some(ref adapter) = self.adapter {
            adapter
                .query(cmd)
                .await
                .with_context(|| format!("SCPI query failed: {}", cmd))
        } else {
            // Mock mode
            Ok(format!("MOCK_RESPONSE:{}", cmd))
        }
    }

    async fn query_with_timeout(&self, cmd: &str, timeout: Duration) -> Result<String> {
        debug!("SCPI query with timeout {:?}: {}", timeout, cmd);

        if let Some(ref adapter) = self.adapter {
            adapter
                .query_with_timeout(cmd, timeout)
                .await
                .with_context(|| format!("SCPI query with timeout failed: {}", cmd))
        } else {
            // Mock mode
            Ok(format!("MOCK_RESPONSE:{}", cmd))
        }
    }

    async fn write(&self, cmd: &str) -> Result<()> {
        debug!("SCPI write: {}", cmd);

        if let Some(ref adapter) = self.adapter {
            adapter
                .write(cmd)
                .await
                .with_context(|| format!("SCPI write failed: {}", cmd))
        } else {
            // Mock mode
            Ok(())
        }
    }

    async fn transact(&self, cmd: &str, timeout: Duration) -> Result<()> {
        debug!("SCPI transact (timeout {:?}): {}", timeout, cmd);

        // Send command
        self.write(cmd).await?;

        // Poll *STB? (Service Request / Status Byte) until ready
        let start = tokio::time::Instant::now();

        loop {
            let status = self.query("*STB?").await?;
            let status_byte: u8 = status
                .trim()
                .parse()
                .with_context(|| format!("Failed to parse status byte: {}", status))?;

            // Bit 5 = Event Status Bit (ESB) - operation complete
            if status_byte & 0x20 != 0 {
                debug!("SCPI transact complete");
                return Ok(());
            }

            // Bit 4 = Message Available (MAV) - error occurred
            if status_byte & 0x10 != 0 {
                anyhow::bail!("SCPI error during transaction (MAV bit set)");
            }

            if start.elapsed() > timeout {
                anyhow::bail!("SCPI transaction timeout after {:?}", timeout);
            }

            // Poll every 50ms
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn read_error(&self) -> Result<u8> {
        let response = self.query("*ESR?").await?;
        response
            .trim()
            .parse()
            .with_context(|| format!("Failed to parse error register: {}", response))
    }

    async fn clear_errors(&self) -> Result<()> {
        self.write("*CLS").await
    }

    async fn reset(&self) -> Result<()> {
        self.write("*RST").await?;

        // Wait for reset to complete (instruments may take time)
        tokio::time::sleep(Duration::from_secs(1)).await;

        Ok(())
    }

    async fn identify(&self) -> Result<String> {
        self.query("*IDN?").await
    }

    fn get_timeout(&self) -> Duration {
        self.timeout
    }

    fn set_timeout(&self, timeout: Duration) {
        // Note: This mutates self, but trait signature doesn't allow &mut self
        // In practice, timeout is typically set via builder or config
        // For now, log a warning
        warn!("set_timeout called but trait doesn't allow mutation - use builder pattern instead");
        let _ = timeout;
    }
}

// ============================================================================
// Kameo Message Types
// ============================================================================

/// Query SCPI command and read response
#[derive(Debug, Clone)]
pub struct Query {
    pub cmd: String,
}

impl Message<Query> for ScpiActor {
    type Reply = Result<String>;

    async fn handle(
        &mut self,
        msg: Query,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.query(&msg.cmd).await
    }
}

/// Query SCPI command with explicit timeout
#[derive(Debug, Clone)]
pub struct QueryWithTimeout {
    pub cmd: String,
    pub timeout: Duration,
}

impl Message<QueryWithTimeout> for ScpiActor {
    type Reply = Result<String>;

    async fn handle(
        &mut self,
        msg: QueryWithTimeout,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.query_with_timeout(&msg.cmd, msg.timeout).await
    }
}

/// Write SCPI command (no response)
#[derive(Debug, Clone)]
pub struct Write {
    pub cmd: String,
}

impl Message<Write> for ScpiActor {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: Write,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.write(&msg.cmd).await
    }
}

/// Execute SCPI command with status polling
#[derive(Debug, Clone)]
pub struct Transact {
    pub cmd: String,
    pub timeout: Duration,
}

impl Message<Transact> for ScpiActor {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: Transact,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.transact(&msg.cmd, msg.timeout).await
    }
}

/// Read error register (*ESR?)
#[derive(Debug, Clone)]
pub struct ReadError;

impl Message<ReadError> for ScpiActor {
    type Reply = Result<u8>;

    async fn handle(
        &mut self,
        _msg: ReadError,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.read_error().await
    }
}

/// Clear error queue (*CLS)
#[derive(Debug, Clone)]
pub struct ClearErrors;

impl Message<ClearErrors> for ScpiActor {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        _msg: ClearErrors,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.clear_errors().await
    }
}

/// Reset instrument to factory defaults (*RST)
#[derive(Debug, Clone)]
pub struct Reset;

impl Message<Reset> for ScpiActor {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        _msg: Reset,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.reset().await
    }
}

/// Query instrument identity (*IDN?)
#[derive(Debug, Clone)]
pub struct Identify;

impl Message<Identify> for ScpiActor {
    type Reply = Result<String>;

    async fn handle(
        &mut self,
        _msg: Identify,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.identify().await
    }
}

/// Convert SCPI events to Arrow RecordBatch
#[derive(Debug, Clone)]
pub struct ToArrowEvents {
    pub events: Vec<ScpiEvent>,
}

impl Message<ToArrowEvents> for ScpiActor {
    type Reply = Result<RecordBatch>;

    async fn handle(
        &mut self,
        msg: ToArrowEvents,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.to_arrow_events(&msg.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kameo::actor::spawn;

    #[tokio::test]
    async fn test_scpi_actor_mock_mode() {
        let actor = ScpiActor::mock("test_scpi".to_string());

        // Mock mode should return mock responses
        let response = actor.query("*IDN?").await.unwrap();
        assert!(response.contains("MOCK"));
    }

    #[tokio::test]
    async fn test_scpi_actor_spawn() {
        let actor = ScpiActor::mock("test_scpi".to_string());
        let actor_ref = spawn(actor);

        // Test identify message
        let idn = actor_ref.ask(Identify).await.unwrap().unwrap();
        assert_eq!(idn, "Mock SCPI Instrument");
    }

    #[tokio::test]
    async fn test_scpi_query_message() {
        let actor = ScpiActor::mock("test_scpi".to_string());
        let actor_ref = spawn(actor);

        let response = actor_ref
            .ask(Query {
                cmd: "TEST?".to_string(),
            })
            .await
            .unwrap()
            .unwrap();

        assert!(response.contains("TEST?"));
    }

    #[tokio::test]
    async fn test_scpi_write_message() {
        let actor = ScpiActor::mock("test_scpi".to_string());
        let actor_ref = spawn(actor);

        actor_ref
            .ask(Write {
                cmd: "OUTPUT ON".to_string(),
            })
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn test_scpi_clear_errors() {
        let actor = ScpiActor::mock("test_scpi".to_string());
        let actor_ref = spawn(actor);

        actor_ref.ask(ClearErrors).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_scpi_reset() {
        let actor = ScpiActor::mock("test_scpi".to_string());
        let actor_ref = spawn(actor);

        actor_ref.ask(Reset).await.unwrap().unwrap();
    }
}
