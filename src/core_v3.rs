//! Core V3 - Unified Architecture (Phase 1)
//!
//! This module implements the redesigned core abstractions based on analysis of
//! DynExp, PyMODAQ, and ScopeFoundry reference frameworks. It coexists with the
//! current architecture during migration.
//!
//! Key improvements:
//! - Unified Instrument trait (replaces V1/V2 split)
//! - Meta instrument traits for polymorphism (DynExp pattern)
//! - Direct async communication (no actor model)
//! - Simplified data flow (single broadcast)
//!
//! See: docs/ARCHITECTURAL_REDESIGN_2025.md

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::core::ParameterValue;

// Re-export existing types that are already correct
pub use crate::core::{ImageData, PixelBuffer, SpectrumData};

// Define types that don't exist yet in old core
/// Region of Interest for camera acquisition
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Roi {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PartialOrd for Roi {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Roi {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare by area first, then by position
        let self_area = self.width * self.height;
        let other_area = other.width * other.height;

        match self_area.cmp(&other_area) {
            std::cmp::Ordering::Equal => {
                // If equal area, compare by top-left position
                match self.x.cmp(&other.x) {
                    std::cmp::Ordering::Equal => self.y.cmp(&other.y),
                    other => other,
                }
            }
            other => other,
        }
    }
}

impl Default for Roi {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 1024,
            height: 1024,
        }
    }
}

/// Image metadata (exposure, gain, etc.)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binning: Option<(u32, u32)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_timestamp_us: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readout_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roi_origin: Option<(u32, u32)>,
}

// =============================================================================
// Measurement Types (Unified)
// =============================================================================

/// Unified measurement representation (replaces V1 DataPoint + V2 Measurement split)
///
/// All instruments emit this enum directly. No conversion layers needed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Measurement {
    /// Single scalar value with metadata
    Scalar {
        name: String,
        value: f64,
        unit: String,
        timestamp: DateTime<Utc>,
    },

    /// Vector of values (e.g., spectrum, time series)
    Vector {
        name: String,
        values: Vec<f64>,
        unit: String,
        timestamp: DateTime<Utc>,
    },

    /// 2D image data with zero-copy optimization
    Image {
        name: String,
        width: u32,
        height: u32,
        buffer: PixelBuffer,
        unit: String,
        metadata: ImageMetadata,
        timestamp: DateTime<Utc>,
    },

    /// Spectrum with frequency/amplitude pairs
    Spectrum {
        name: String,
        frequencies: Vec<f64>,
        amplitudes: Vec<f64>,
        frequency_unit: Option<String>,
        amplitude_unit: Option<String>,
        metadata: Option<Value>,
        timestamp: DateTime<Utc>,
    },
}

impl Measurement {
    /// Extract timestamp regardless of variant
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Measurement::Scalar { timestamp, .. } => *timestamp,
            Measurement::Vector { timestamp, .. } => *timestamp,
            Measurement::Image { timestamp, .. } => *timestamp,
            Measurement::Spectrum { timestamp, .. } => *timestamp,
        }
    }

    /// Extract name regardless of variant
    pub fn name(&self) -> &str {
        match self {
            Measurement::Scalar { name, .. } => name,
            Measurement::Vector { name, .. } => name,
            Measurement::Image { name, .. } => name,
            Measurement::Spectrum { name, .. } => name,
        }
    }
}

// =============================================================================
// Instrument State and Commands
// =============================================================================

/// Instrument lifecycle state
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstrumentState {
    /// Instrument is not connected to hardware.
    Disconnected,
    /// Instrument is in the process of connecting.
    Connecting,
    /// Instrument is connected and ready to operate.
    Connected,
    /// Currently acquiring/operating
    Running,
    /// Paused (can resume)
    Paused,
    /// Error state (see error message)
    Error,
    /// Shutting down
    ShuttingDown,
}

/// Generic command envelope for instrument control
///
/// Replaces the complex InstrumentCommand enum. Instruments handle
/// commands via their trait methods instead.
#[derive(Clone, Debug)]
pub enum Command {
    /// Start acquisition/operation
    Start,
    /// Stop acquisition/operation
    Stop,
    /// Pause acquisition/operation
    Pause,
    /// Resume from pause
    Resume,
    /// Request current state
    GetState,
    /// Request parameter value
    GetParameter(String),
    /// Set parameter value (parameter name, JSON value)
    SetParameter(String, serde_json::Value),
    /// Configure multiple parameters at once
    Configure {
        params: HashMap<String, ParameterValue>,
    },
    /// Instrument-specific command (for specialized operations)
    Custom(String, serde_json::Value),
}

/// Response to command execution
#[derive(Clone, Debug)]
pub enum Response {
    /// Command completed successfully
    Ok,
    /// Command completed with state update
    State(InstrumentState),
    /// Command completed with parameter value
    Parameter(serde_json::Value),
    /// Command completed with custom data
    Custom(serde_json::Value),
    /// Command failed with error message
    Error(String),
}

// =============================================================================
// Parameter Base Trait (for dynamic access)
// =============================================================================

/// Base trait for all parameters (enables heterogeneous collections)
///
/// Concrete parameters use `Parameter<T>` (see parameter.rs).
pub trait ParameterBase: Send + Sync {
    /// Parameter name
    fn name(&self) -> &str;

    /// Get current value as JSON
    fn value_json(&self) -> serde_json::Value;

    /// Set value from JSON
    fn set_json(&mut self, value: serde_json::Value) -> Result<()>;

    /// Get parameter constraints as JSON
    fn constraints_json(&self) -> serde_json::Value;
}

// =============================================================================
// Core Instrument Trait (Unified)
// =============================================================================

/// Base trait for all instruments (replaces both V1 and V2)
///
/// All instruments implement this trait directly. No wrapper types needed.
/// Instruments run in their own Tokio tasks and communicate via channels.
///
/// # Data Flow
///
/// ```text
/// Instrument Task → data_channel() → broadcast::Receiver<Measurement>
///                                    ↓
///                                   GUI/Storage/Processors subscribe directly
/// ```
///
/// # Command Flow
///
/// ```text
/// Manager → execute(cmd) → Instrument implementation
/// ```
///
/// # Example Implementation
///
/// ```rust,ignore
/// struct MockCamera {
///     id: String,
///     state: InstrumentState,
///     data_tx: broadcast::Sender<Measurement>,
///     exposure: Parameter<f64>,
/// }
///
/// #[async_trait]
/// impl Instrument for MockCamera {
///     fn id(&self) -> &str { &self.id }
///     fn state(&self) -> InstrumentState { self.state }
///
///     async fn initialize(&mut self) -> Result<()> {
///         self.state = InstrumentState::Connecting;
///         // ... hardware connection logic ...
///         self.state = InstrumentState::Connected;
///         Ok(())
///     }
///
///     async fn shutdown(&mut self) -> Result<()> {
///         self.state = InstrumentState::ShuttingDown;
///         Ok(())
///     }
///
///     fn data_channel(&self) -> broadcast::Receiver<Measurement> {
///         self.data_tx.subscribe()
///     }
///
///     async fn execute(&mut self, cmd: Command) -> Result<Response> {
///         match cmd {
///             Command::Start => {
///                 self.state = InstrumentState::Running;
///                 Ok(Response::Ok)
///             }
///             Command::Stop => {
///                 self.state = InstrumentState::Connected;
///                 Ok(Response::Ok)
///             }
///             _ => Ok(Response::Error("Unsupported command".to_string()))
///         }
///     }
///
///     fn parameters(&self) -> &HashMap<String, Box<dyn ParameterBase>> {
///         &self.params
///     }
/// }
/// ```
#[async_trait]
pub trait Instrument: Send + Sync {
    /// Unique instrument identifier
    fn id(&self) -> &str;

    /// Current lifecycle state
    fn state(&self) -> InstrumentState;

    /// Initialize hardware connection
    ///
    /// Called once before instrument can be used. Should establish
    /// hardware connection, verify communication, and prepare for operation.
    async fn initialize(&mut self) -> Result<()>;

    /// Shutdown hardware connection gracefully
    ///
    /// Called during application shutdown or instrument removal.
    /// Should release hardware resources and clean up.
    async fn shutdown(&mut self) -> Result<()>;

    /// Subscribe to data stream
    ///
    /// Returns a broadcast receiver for measurements. Multiple subscribers
    /// can receive the same data stream independently.
    fn data_channel(&self) -> broadcast::Receiver<Measurement>;

    /// Execute command (direct async call, no message passing)
    ///
    /// Replaces the old InstrumentCommand enum with direct method dispatch.
    /// Instruments can implement custom command handling as needed.
    async fn execute(&mut self, cmd: Command) -> Result<Response>;

    /// Access instrument parameters
    ///
    /// Returns reference to parameter collection for introspection and
    /// dynamic access (e.g., GUI parameter editors).
    fn parameters(&self) -> &HashMap<String, Box<dyn ParameterBase>>;

    /// Get mutable access to parameters (for setting)
    fn parameters_mut(&mut self) -> &mut HashMap<String, Box<dyn ParameterBase>>;
}

// =============================================================================
// Meta Instrument Traits (DynExp Pattern)
// =============================================================================

/// Camera capability trait
///
/// Modules that require camera functionality should work with this trait
/// instead of concrete camera types. This enables hardware-agnostic
/// experiment logic (e.g., scan modules work with any Camera implementation).
#[async_trait]
pub trait Camera: Instrument {
    /// Set exposure time in milliseconds
    async fn set_exposure(&mut self, ms: f64) -> Result<()>;

    /// Set region of interest
    async fn set_roi(&mut self, roi: Roi) -> Result<()>;

    /// Get current ROI
    async fn roi(&self) -> Roi;

    /// Set binning (horizontal, vertical)
    async fn set_binning(&mut self, h: u32, v: u32) -> Result<()>;

    /// Start continuous acquisition
    async fn start_acquisition(&mut self) -> Result<()>;

    /// Stop acquisition
    async fn stop_acquisition(&mut self) -> Result<()>;

    /// Arm camera for triggered acquisition
    async fn arm_trigger(&mut self) -> Result<()>;

    /// Software trigger (if supported)
    async fn trigger(&mut self) -> Result<()>;
}

/// Stage/positioner capability trait
///
/// Modules that control motion should work with this trait for
/// hardware-agnostic positioning logic.
///
/// V3 Design: Control methods for stage positioning and motion.
/// Position updates are broadcast via Instrument::data_channel().
#[async_trait]
pub trait Stage: Instrument {
    /// Move to absolute position in mm
    async fn move_absolute(&mut self, position_mm: f64) -> Result<()>;

    /// Move relative to current position in mm
    async fn move_relative(&mut self, distance_mm: f64) -> Result<()>;

    /// Get current position in mm
    async fn position(&self) -> Result<f64>;

    /// Stop motion immediately
    async fn stop_motion(&mut self) -> Result<()>;

    /// Check if stage is currently moving
    async fn is_moving(&self) -> Result<bool>;

    /// Home stage (find reference position)
    async fn home(&mut self) -> Result<()>;

    /// Set velocity in mm/s
    async fn set_velocity(&mut self, mm_per_sec: f64) -> Result<()>;

    /// Wait for motion to settle (with timeout)
    async fn wait_settled(&self, timeout: std::time::Duration) -> Result<()> {
        let start = std::time::Instant::now();
        loop {
            if !self.is_moving().await? {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!("Timeout waiting for motion to settle"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

/// Spectrometer capability trait
#[async_trait]
pub trait Spectrometer: Instrument {
    /// Set integration time in milliseconds
    async fn set_integration_time(&mut self, ms: f64) -> Result<()>;

    /// Get wavelength range
    fn wavelength_range(&self) -> (f64, f64);

    /// Start spectrum acquisition
    async fn start_acquisition(&mut self) -> Result<()>;

    /// Stop acquisition
    async fn stop_acquisition(&mut self) -> Result<()>;
}

/// Power meter capability trait
#[async_trait]
pub trait PowerMeter: Instrument {
    /// Set wavelength for calibration (nm)
    async fn set_wavelength(&mut self, nm: f64) -> Result<()>;

    /// Set measurement range (watts)
    async fn set_range(&mut self, watts: f64) -> Result<()>;

    /// Zero/calibrate sensor
    async fn zero(&mut self) -> Result<()>;
}

/// Laser capability trait
///
/// V3 Design: Control methods for tunable lasers with wavelength/power control.
/// Power/wavelength readings are broadcast via Instrument::data_channel().
#[async_trait]
pub trait Laser: Instrument {
    /// Set wavelength in nanometers (for tunable lasers)
    async fn set_wavelength(&mut self, nm: f64) -> Result<()>;

    /// Get current wavelength setting in nanometers
    async fn wavelength(&self) -> Result<f64>;

    /// Set output power in watts
    async fn set_power(&mut self, watts: f64) -> Result<()>;

    /// Get current power output in watts
    async fn power(&self) -> Result<f64>;

    /// Enable shutter (allow laser emission)
    async fn enable_shutter(&mut self) -> Result<()>;

    /// Disable shutter (block laser emission)
    async fn disable_shutter(&mut self) -> Result<()>;

    /// Check if shutter is enabled (laser can emit)
    async fn is_enabled(&self) -> Result<bool>;
}

// =============================================================================
// Instrument Handle (Direct Management)
// =============================================================================

/// Handle for managing instrument lifecycle and communication
///
/// Replaces the actor-based management. Each instrument runs in a task,
/// and the handle provides direct access to channels and lifecycle control.
pub struct InstrumentHandle {
    /// Instrument identifier
    pub id: String,

    /// Tokio task handle (for monitoring and cancellation)
    pub task: JoinHandle<Result<()>>,

    /// Shutdown signal sender
    pub shutdown_tx: oneshot::Sender<()>,

    /// Data broadcast receiver (subscribe to get measurements)
    pub data_rx: broadcast::Receiver<Measurement>,

    /// Command channel for instrument control
    pub command_tx: mpsc::Sender<Command>,

    /// Reference to instrument (for capability downcasting)
    pub instrument: Arc<tokio::sync::Mutex<Box<dyn Instrument>>>,
}

impl InstrumentHandle {
    /// Send command and wait for response
    pub async fn send_command(&self, cmd: Command) -> Result<Response> {
        self.command_tx.send(cmd).await?;
        // Response will come via oneshot channel in actual implementation
        // This is simplified for Phase 1
        Ok(Response::Ok)
    }

    /// Subscribe to data stream
    pub fn subscribe(&self) -> broadcast::Receiver<Measurement> {
        self.data_rx.resubscribe()
    }

    /// Request graceful shutdown
    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        self.task.await??;
        Ok(())
    }

    /// Check if instrument implements Camera trait
    pub async fn as_camera(&self) -> Option<Arc<tokio::sync::Mutex<Box<dyn Camera>>>> {
        let _guard = self.instrument.lock().await;
        // Attempt downcast (simplified for Phase 1)
        // Full implementation would use proper trait object casting
        None
    }

    /// Check if instrument implements Stage trait  
    pub async fn as_stage(&self) -> Option<Arc<tokio::sync::Mutex<Box<dyn Stage>>>> {
        None
    }

    /// Check if instrument implements Spectrometer trait
    pub async fn as_spectrometer(&self) -> Option<Arc<tokio::sync::Mutex<Box<dyn Spectrometer>>>> {
        None
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_measurement_accessors() {
        let m = Measurement::Scalar {
            name: "test".to_string(),
            value: 42.0,
            unit: "mW".to_string(),
            timestamp: Utc::now(),
        };

        assert_eq!(m.name(), "test");
        assert!(m.timestamp() <= Utc::now());
    }

    #[test]
    fn test_instrument_state_transitions() {
        assert_ne!(InstrumentState::Connected, InstrumentState::Running);
        assert_eq!(InstrumentState::Connected, InstrumentState::Connected);
    }

    #[test]
    fn test_command_types() {
        let cmd = Command::Start;
        assert!(matches!(cmd, Command::Start));

        let cmd = Command::SetParameter("exposure".to_string(), serde_json::json!(100.0));
        assert!(matches!(cmd, Command::SetParameter(_, _)));
    }
}
