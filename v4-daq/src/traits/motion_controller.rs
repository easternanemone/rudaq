//! MotionController meta-instrument trait
//!
//! Hardware-agnostic interface for multi-axis motion control.
//! Implementations handle protocol-specific details (ESP300 serial, CAN, etc.).

use anyhow::Result;
use arrow::array::{Float64Array, StringArray, TimestampNanosecondArray, UInt8Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::sync::Arc;

/// Axis state enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, kameo::Reply)]
pub enum AxisState {
    /// Idle, not moving
    Idle,
    /// Currently moving to target
    Moving,
    /// In homing sequence
    Homing,
    /// Limit switch engaged (NOT an error - normal state at boundary)
    LimitSwitch,
    /// Faulted state (hardware error, needs reset)
    Faulted,
}

impl AxisState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AxisState::Idle => "Idle",
            AxisState::Moving => "Moving",
            AxisState::Homing => "Homing",
            AxisState::LimitSwitch => "LimitSwitch",
            AxisState::Faulted => "Faulted",
        }
    }
}

/// Axis position and state snapshot
#[derive(Debug, Clone, Copy, kameo::Reply)]
pub struct AxisPosition {
    /// Current position in motor units (e.g., mm, degrees)
    pub position: f64,
    /// Current velocity in units/second (may be 0 if idle)
    pub velocity: f64,
    /// Axis state
    pub state: AxisState,
    /// Timestamp of this measurement (ns)
    pub timestamp_ns: i64,
}

/// Motion configuration for an axis
#[derive(Debug, Clone, kameo::Reply)]
pub struct MotionConfig {
    /// Velocity in units/second
    pub velocity: f64,
    /// Acceleration in units/second²
    pub acceleration: f64,
    /// Deceleration in units/second²
    pub deceleration: f64,
    /// Minimum position (soft limit)
    pub min_position: f64,
    /// Maximum position (soft limit)
    pub max_position: f64,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            velocity: 1.0,           // 1 unit/s
            acceleration: 10.0,      // 10 units/s²
            deceleration: 10.0,      // 10 units/s²
            min_position: -100.0,    // -100 units
            max_position: 100.0,     // +100 units
        }
    }
}

/// Motion event for streaming position data
#[derive(Debug, Clone, Copy, kameo::Reply)]
pub struct MotionEvent {
    pub axis: u8,
    pub position: AxisPosition,
}

/// Motion controller meta-instrument trait
///
/// Hardware-agnostic interface for multi-axis motion control.
/// Implementations handle protocol-specific details (ESP300 serial, CAN, etc.).
///
/// ## Position Units
/// - Trait uses abstract "motor units" (mm, degrees, encoder counts, etc.)
/// - Adapter specifies units in metadata
/// - Conversion to physical units happens at analysis layer
///
/// ## Multi-Axis Coordination
/// - Individual axis control only (no synchronized moves)
/// - Higher-level sequencer handles coordinated motion
/// - Position streaming includes all axes for correlation
///
/// ## Homing
/// - Uses hardware default homing sequence
/// - Timeout: 30 seconds
/// - Sets axis position to 0 after homing complete
///
/// ## Limit Handling
/// - Soft limits: Return error (configuration problem)
/// - Hard limits: Set `AxisState::LimitSwitch` (normal boundary)
#[async_trait]
pub trait MotionController: Send + Sync {
    /// Move axis to absolute position
    ///
    /// # Arguments
    /// * `axis` - Axis number (0-indexed; 0=X, 1=Y, 2=Z for 3-axis)
    /// * `position` - Target position in motor units
    ///
    /// # Behavior
    /// - Commands motion with configured velocity/acceleration
    /// - Returns immediately (motion continues asynchronously)
    /// - Use `read_position()` or position streaming to monitor progress
    ///
    /// # Errors
    /// - Axis out of range
    /// - Position exceeds soft limits
    /// - Hardware communication error
    /// - Axis in error state
    async fn move_absolute(&self, axis: u8, position: f64) -> Result<()>;

    /// Move axis by relative delta
    ///
    /// # Arguments
    /// * `axis` - Axis number
    /// * `delta` - Position change (positive = forward)
    ///
    /// # Behavior
    /// - Same as move_absolute but with position += delta
    ///
    /// # Errors
    /// - Computed target exceeds limits
    /// - Axis out of range
    async fn move_relative(&self, axis: u8, delta: f64) -> Result<()>;

    /// Stop all motion on axis
    ///
    /// # Arguments
    /// * `axis` - Axis number (or None for all axes)
    ///
    /// # Behavior
    /// - Immediate deceleration to stop
    /// - Holds position
    ///
    /// # Errors
    /// - Hardware communication error
    async fn stop(&self, axis: Option<u8>) -> Result<()>;

    /// Home axis (find reference position)
    ///
    /// # Arguments
    /// * `axis` - Axis number
    ///
    /// # Behavior
    /// - Executes homing sequence (typically: move to limit, back off, set zero)
    /// - Blocks until homing complete (max 30s timeout)
    /// - Sets axis position to 0
    ///
    /// # Errors
    /// - Limit switch not found
    /// - Hardware timeout
    /// - Axis already homing
    async fn home(&self, axis: u8) -> Result<()>;

    /// Read current position of axis
    ///
    /// # Arguments
    /// * `axis` - Axis number
    ///
    /// # Returns
    /// - Current position in motor units
    ///
    /// # Errors
    /// - Axis out of range
    /// - Hardware communication error
    async fn read_position(&self, axis: u8) -> Result<f64>;

    /// Read full state of axis
    ///
    /// # Arguments
    /// * `axis` - Axis number
    ///
    /// # Returns
    /// - AxisPosition with position, velocity, state, timestamp
    ///
    /// # Errors
    /// - Axis out of range
    /// - Hardware communication error
    async fn read_axis_state(&self, axis: u8) -> Result<AxisPosition>;

    /// Configure motion parameters for axis
    ///
    /// # Arguments
    /// * `axis` - Axis number
    /// * `config` - Velocity, acceleration, soft limits
    ///
    /// # Errors
    /// - Axis out of range
    /// - Invalid parameters (velocity=0, acceleration=negative, etc.)
    async fn configure_motion(&self, axis: u8, config: MotionConfig) -> Result<()>;

    /// Start streaming position data for monitoring
    ///
    /// # Returns
    /// - Async receiver for MotionEvent updates
    /// - Events include all axes, emitted at configured polling rate
    ///
    /// # Note
    /// - Only one stream active at a time
    /// - Calling again closes previous stream
    async fn start_position_stream(&self) -> Result<tokio::sync::mpsc::Receiver<MotionEvent>>;

    /// Stop position streaming
    ///
    /// # Behavior
    /// - Closes position stream
    /// - Does NOT stop motion
    async fn stop_position_stream(&self) -> Result<()>;

    /// Get number of axes on this controller
    fn num_axes(&self) -> u8;

    /// Get current configuration for axis
    async fn get_motion_config(&self, axis: u8) -> Result<MotionConfig>;

    /// Serialize motion events to Arrow RecordBatch
    ///
    /// # Arguments
    /// * `events` - Slice of motion events to serialize
    ///
    /// # Output Schema
    /// ```text
    /// timestamp (i64 ns, Timestamp(Nanosecond))
    /// axis (u8)
    /// position (f64)
    /// velocity (f64)
    /// state (utf8: "Idle", "Moving", "Homing", etc.)
    /// ```
    fn to_arrow_positions(&self, events: &[MotionEvent]) -> Result<RecordBatch> {
        static SCHEMA: Lazy<Arc<Schema>> = Lazy::new(|| {
            Arc::new(Schema::new(vec![
                Field::new(
                    "timestamp",
                    DataType::Timestamp(arrow::datatypes::TimeUnit::Nanosecond, None),
                    false,
                ),
                Field::new("axis", DataType::UInt8, false),
                Field::new("position", DataType::Float64, false),
                Field::new("velocity", DataType::Float64, false),
                Field::new("state", DataType::Utf8, false),
            ]))
        });

        let timestamps: Vec<i64> = events.iter().map(|e| e.position.timestamp_ns).collect();
        let axes: Vec<u8> = events.iter().map(|e| e.axis).collect();
        let positions: Vec<f64> = events.iter().map(|e| e.position.position).collect();
        let velocities: Vec<f64> = events.iter().map(|e| e.position.velocity).collect();
        let states: StringArray = events
            .iter()
            .map(|e| Some(e.position.state.as_str()))
            .collect();

        let batch = RecordBatch::try_new(
            SCHEMA.clone(),
            vec![
                Arc::new(TimestampNanosecondArray::from(timestamps)),
                Arc::new(UInt8Array::from(axes)),
                Arc::new(Float64Array::from(positions)),
                Arc::new(Float64Array::from(velocities)),
                Arc::new(states),
            ],
        )?;

        Ok(batch)
    }
}
