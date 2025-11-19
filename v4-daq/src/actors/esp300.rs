//! ESP300 Motion Controller Actor (V4)
//!
//! Kameo actor implementing MotionController trait for Newport ESP300.
//! Uses SerialAdapterV4 for RS-232 communication.
//!
//! ## Hardware Requirements
//!
//! - Baud rate: 19200
//! - Flow control: Hardware (RTS/CTS)
//! - Line terminator: \r\n
//!
//! ## Example Usage
//!
//! ```no_run
//! use kameo::prelude::*;
//! use v4_daq::actors::ESP300;
//! use v4_daq::hardware::SerialAdapterV4Builder;
//! use std::time::Duration;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create serial adapter for ESP300
//! let adapter = SerialAdapterV4Builder::new("/dev/ttyUSB0".to_string(), 19200)
//!     .with_timeout(Duration::from_secs(2))
//!     .with_line_terminator("\r\n".to_string())
//!     .with_response_delimiter('\n')
//!     .build();
//!
//! // Spawn ESP300 actor (3 axes)
//! let actor = ESP300 {
//!     id: "esp300_stage".to_string(),
//!     adapter: Some(adapter),
//!     num_axes: 3,
//!     motion_configs: vec![Default::default(); 3],
//!     position_stream_tx: None,
//! };
//!
//! let actor_ref = kameo::spawn(actor);
//!
//! // Move axis 0 to position 10.0 mm
//! actor_ref.ask(MoveAbsolute { axis: 0, position: 10.0 }).await??;
//!
//! // Read current position
//! let pos = actor_ref.ask(ReadPosition { axis: 0 }).await??;
//! println!("Current position: {} mm", pos);
//! # Ok(())
//! # }
//! ```

use crate::hardware::{SerialAdapterV4, SerialAdapterV4Builder};
use crate::traits::motion_controller::{
    AxisPosition, AxisState, MotionConfig, MotionController, MotionEvent,
};
use anyhow::{anyhow, Context as AnyhowContext, Result};
use async_trait::async_trait;
use kameo::error::BoxSendError;
use kameo::message::{Context, Message};
use kameo::prelude::*;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// ESP300 Motion Controller Actor
///
/// Implements MotionController trait for Newport ESP300 multi-axis controller.
/// Uses SerialAdapterV4 for RS-232 communication at 19200 baud with hardware flow control.
pub struct ESP300 {
    /// Unique instrument identifier
    pub id: String,

    /// Serial adapter (None = mock mode)
    pub adapter: Option<SerialAdapterV4>,

    /// Number of axes (typically 1-3)
    pub num_axes: u8,

    /// Motion configuration per axis
    pub motion_configs: Vec<MotionConfig>,

    /// Position stream transmitter (if active)
    pub position_stream_tx: Option<mpsc::Sender<MotionEvent>>,
}

impl ESP300 {
    /// Create a new ESP300 actor with serial adapter
    ///
    /// # Arguments
    /// * `id` - Unique instrument identifier
    /// * `port` - Serial port path (e.g., "/dev/ttyUSB0")
    /// * `num_axes` - Number of axes (1-3)
    pub fn with_serial(id: String, port: String, num_axes: u8) -> Self {
        let adapter = SerialAdapterV4Builder::new(port, 19200)
            .with_timeout(Duration::from_secs(2))
            .with_line_terminator("\r\n".to_string())
            .with_response_delimiter('\n')
            .build();

        Self {
            id,
            adapter: Some(adapter),
            num_axes,
            motion_configs: vec![Default::default(); num_axes as usize],
            position_stream_tx: None,
        }
    }

    /// Create a mock ESP300 actor (no hardware)
    pub fn mock(id: String, num_axes: u8) -> Self {
        Self {
            id,
            adapter: None,
            num_axes,
            motion_configs: vec![Default::default(); num_axes as usize],
            position_stream_tx: None,
        }
    }

    /// Validate axis number
    fn validate_axis(&self, axis: u8) -> Result<()> {
        if axis >= self.num_axes {
            anyhow::bail!("Axis {} out of range (0-{})", axis, self.num_axes - 1);
        }
        Ok(())
    }

    /// Send command to ESP300 and read response
    async fn send_command(&self, cmd: &str) -> Result<String> {
        if let Some(ref adapter) = self.adapter {
            adapter.send_command(cmd).await
        } else {
            // Mock mode - return sensible mock responses
            if cmd.starts_with("*IDN?") {
                Ok("Newport ESP300 Motion Controller Mock".to_string())
            } else if cmd.ends_with("TP?") {
                // Position query - return 0.0
                Ok("0.0".to_string())
            } else if cmd.ends_with("MD?") {
                // Motion done query - return 1 (done)
                Ok("1".to_string())
            } else {
                Ok(format!("0")) // Default numeric response
            }
        }
    }

    /// Send command without expecting response
    async fn send_command_no_response(&self, cmd: &str) -> Result<()> {
        if let Some(ref adapter) = self.adapter {
            adapter.send_command_no_response(cmd).await
        } else {
            // Mock mode
            Ok(())
        }
    }

    /// Query axis position
    async fn query_position(&self, axis: u8) -> Result<f64> {
        let cmd = format!("{}TP?", axis + 1); // ESP300 uses 1-indexed axes
        let response = self.send_command(&cmd).await?;

        response
            .trim()
            .parse()
            .with_context(|| format!("Failed to parse position from: {}", response))
    }

    /// Query axis motion status (0 = moving, 1 = done)
    async fn query_motion_done(&self, axis: u8) -> Result<bool> {
        let cmd = format!("{}MD?", axis + 1);
        let response = self.send_command(&cmd).await?;

        let status: i32 = response
            .trim()
            .parse()
            .with_context(|| format!("Failed to parse motion status from: {}", response))?;

        Ok(status == 1)
    }
}

impl Actor for ESP300 {
    type Args = Self;
    type Error = BoxSendError;

    async fn on_start(
        args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        info!("ESP300 actor {} starting ({} axes)", args.id, args.num_axes);

        // Query identity on startup
        let mut actor = args;

        if actor.adapter.is_some() {
            match actor.send_command("*IDN?").await {
                Ok(idn) => {
                    info!("ESP300 actor {} connected: {}", actor.id, idn);
                }
                Err(e) => {
                    warn!("ESP300 actor {} failed to read identity: {}", actor.id, e);
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
        info!("ESP300 actor {} stopping", self.id);

        // Stop position streaming if active
        if let Some(tx) = self.position_stream_tx.take() {
            drop(tx); // Close channel
        }

        // Stop all axes
        for axis in 0..self.num_axes {
            let _ = self.send_command(&format!("{}ST", axis + 1)).await;
        }

        Ok(())
    }
}

// ============================================================================
// MotionController Trait Implementation
// ============================================================================

#[async_trait]
impl MotionController for ESP300 {
    async fn move_absolute(&self, axis: u8, position: f64) -> Result<()> {
        self.validate_axis(axis)?;

        let config = &self.motion_configs[axis as usize];

        // Check soft limits
        if position < config.min_position || position > config.max_position {
            anyhow::bail!(
                "Position {} exceeds limits [{}, {}]",
                position,
                config.min_position,
                config.max_position
            );
        }

        let cmd = format!("{}PA{}", axis + 1, position);
        self.send_command_no_response(&cmd).await?;

        debug!("ESP300 axis {} moving to absolute position {}", axis, position);
        Ok(())
    }

    async fn move_relative(&self, axis: u8, delta: f64) -> Result<()> {
        self.validate_axis(axis)?;

        // Query current position to validate final position
        let current = self.query_position(axis).await?;
        let target = current + delta;

        let config = &self.motion_configs[axis as usize];
        if target < config.min_position || target > config.max_position {
            anyhow::bail!(
                "Target position {} exceeds limits [{}, {}]",
                target,
                config.min_position,
                config.max_position
            );
        }

        let cmd = format!("{}PR{}", axis + 1, delta);
        self.send_command_no_response(&cmd).await?;

        debug!("ESP300 axis {} moving relative by {}", axis, delta);
        Ok(())
    }

    async fn stop(&self, axis: Option<u8>) -> Result<()> {
        match axis {
            Some(ax) => {
                self.validate_axis(ax)?;
                let cmd = format!("{}ST", ax + 1);
                self.send_command_no_response(&cmd).await?;
                debug!("ESP300 axis {} stopped", ax);
            }
            None => {
                // Stop all axes
                for ax in 0..self.num_axes {
                    let cmd = format!("{}ST", ax + 1);
                    let _ = self.send_command_no_response(&cmd).await;
                }
                debug!("ESP300 all axes stopped");
            }
        }

        Ok(())
    }

    async fn home(&self, axis: u8) -> Result<()> {
        self.validate_axis(axis)?;

        let cmd = format!("{}OR", axis + 1); // OR = Origin/Home
        self.send_command_no_response(&cmd).await?;

        debug!("ESP300 axis {} homing started", axis);

        // Wait for homing to complete (max 30s timeout)
        let start = tokio::time::Instant::now();
        let timeout = Duration::from_secs(30);

        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("Homing timeout after {:?}", timeout);
            }

            // Check if motion complete
            if self.query_motion_done(axis).await? {
                info!("ESP300 axis {} homing complete", axis);
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn read_position(&self, axis: u8) -> Result<f64> {
        self.validate_axis(axis)?;
        self.query_position(axis).await
    }

    async fn read_axis_state(&self, axis: u8) -> Result<AxisPosition> {
        self.validate_axis(axis)?;

        let position = self.query_position(axis).await?;
        let is_done = self.query_motion_done(axis).await?;

        let state = if is_done {
            AxisState::Idle
        } else {
            AxisState::Moving
        };

        // ESP300 doesn't provide real-time velocity, use 0 when idle
        let velocity = if is_done { 0.0 } else { self.motion_configs[axis as usize].velocity };

        let timestamp_ns = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos() as i64;

        Ok(AxisPosition {
            position,
            velocity,
            state,
            timestamp_ns,
        })
    }

    async fn configure_motion(&self, axis: u8, config: MotionConfig) -> Result<()> {
        self.validate_axis(axis)?;

        // Validate parameters
        if config.velocity <= 0.0 {
            anyhow::bail!("Velocity must be positive");
        }
        if config.acceleration <= 0.0 {
            anyhow::bail!("Acceleration must be positive");
        }
        if config.deceleration <= 0.0 {
            anyhow::bail!("Deceleration must be positive");
        }

        // Set velocity
        let cmd = format!("{}VA{}", axis + 1, config.velocity);
        self.send_command_no_response(&cmd).await?;

        // Set acceleration
        let cmd = format!("{}AC{}", axis + 1, config.acceleration);
        self.send_command_no_response(&cmd).await?;

        // Set deceleration
        let cmd = format!("{}AG{}", axis + 1, config.deceleration);
        self.send_command_no_response(&cmd).await?;

        // Update stored config
        // Note: Can't mutate self, but config is passed by value so we can't store it
        // This is a limitation of the trait signature - would need &mut self

        debug!("ESP300 axis {} motion configured", axis);
        Ok(())
    }

    async fn start_position_stream(&self) -> Result<tokio::sync::mpsc::Receiver<MotionEvent>> {
        let (tx, rx) = mpsc::channel(10);

        // TODO: Implement position streaming background task
        // For now, return empty channel
        warn!("ESP300 position streaming not yet implemented");

        Ok(rx)
    }

    async fn stop_position_stream(&self) -> Result<()> {
        // TODO: Stop streaming background task
        warn!("ESP300 position streaming not yet implemented");
        Ok(())
    }

    fn num_axes(&self) -> u8 {
        self.num_axes
    }

    async fn get_motion_config(&self, axis: u8) -> Result<MotionConfig> {
        self.validate_axis(axis)?;
        Ok(self.motion_configs[axis as usize].clone())
    }
}

// ============================================================================
// Kameo Message Types
// ============================================================================

/// Move axis to absolute position
#[derive(Debug, Clone)]
pub struct MoveAbsolute {
    pub axis: u8,
    pub position: f64,
}

impl Message<MoveAbsolute> for ESP300 {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: MoveAbsolute,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.move_absolute(msg.axis, msg.position).await
    }
}

/// Move axis by relative delta
#[derive(Debug, Clone)]
pub struct MoveRelative {
    pub axis: u8,
    pub delta: f64,
}

impl Message<MoveRelative> for ESP300 {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: MoveRelative,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.move_relative(msg.axis, msg.delta).await
    }
}

/// Stop axis motion
#[derive(Debug, Clone)]
pub struct Stop {
    pub axis: Option<u8>,
}

impl Message<Stop> for ESP300 {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: Stop,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.stop(msg.axis).await
    }
}

/// Home axis (find reference position)
#[derive(Debug, Clone)]
pub struct Home {
    pub axis: u8,
}

impl Message<Home> for ESP300 {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: Home,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.home(msg.axis).await
    }
}

/// Read current position
#[derive(Debug, Clone)]
pub struct ReadPosition {
    pub axis: u8,
}

impl Message<ReadPosition> for ESP300 {
    type Reply = Result<f64>;

    async fn handle(
        &mut self,
        msg: ReadPosition,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.read_position(msg.axis).await
    }
}

/// Read full axis state
#[derive(Debug, Clone)]
pub struct ReadAxisState {
    pub axis: u8,
}

impl Message<ReadAxisState> for ESP300 {
    type Reply = Result<AxisPosition>;

    async fn handle(
        &mut self,
        msg: ReadAxisState,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.read_axis_state(msg.axis).await
    }
}

/// Configure motion parameters
#[derive(Debug, Clone)]
pub struct ConfigureMotion {
    pub axis: u8,
    pub config: MotionConfig,
}

impl Message<ConfigureMotion> for ESP300 {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        msg: ConfigureMotion,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.configure_motion(msg.axis, msg.config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_esp300_mock_mode() {
        let actor = ESP300::mock("test_esp300".to_string(), 3);

        // Mock mode should return mock responses
        let response = actor.send_command("*IDN?").await.unwrap();
        assert!(response.contains("MOCK"));
    }

    #[tokio::test]
    async fn test_esp300_spawn() {
        let actor = ESP300::mock("test_esp300".to_string(), 3);
        let actor_ref = kameo::spawn(actor);

        // Test that actor spawns successfully
        assert!(actor_ref.is_alive());

        actor_ref.kill();
        actor_ref.wait_for_shutdown().await;
    }

    #[tokio::test]
    async fn test_esp300_axis_validation() {
        let actor = ESP300::mock("test_esp300".to_string(), 2);

        // Valid axis
        assert!(actor.validate_axis(0).is_ok());
        assert!(actor.validate_axis(1).is_ok());

        // Invalid axis
        assert!(actor.validate_axis(2).is_err());
        assert!(actor.validate_axis(99).is_err());
    }
}
