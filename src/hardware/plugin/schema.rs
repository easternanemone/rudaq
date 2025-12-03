//! YAML schema types for instrument plugin configuration.
//!
//! This module defines the Rust types that correspond to the YAML plugin format.

use serde::{Deserialize, Serialize};

/// Top-level struct for the instrument plugin configuration file (YAML).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentConfig {
    pub metadata: InstrumentMetadata,

    #[serde(default)]
    pub protocol: ProtocolConfig,

    #[serde(default)]
    pub on_connect: Vec<CommandSequence>,

    #[serde(default)]
    pub on_disconnect: Vec<CommandSequence>,

    #[serde(default)]
    pub error_patterns: Vec<String>,

    #[serde(default)]
    pub capabilities: CapabilitiesConfig,

    #[serde(default)]
    pub ui_layout: Vec<UiElement>,
}

/// Metadata about the instrument driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub driver_type: DriverType,
}

/// Defines the type of driver/protocol for the generic interpreter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DriverType {
    /// Serial port with SCPI-style commands
    #[serde(rename = "serial_scpi")]
    SerialScpi,
    /// TCP/IP with SCPI-style commands
    #[serde(rename = "tcp_scpi")]
    TcpScpi,
    /// Serial port with raw binary protocol
    #[serde(rename = "serial_raw")]
    SerialRaw,
    /// TCP/IP with raw binary protocol  
    #[serde(rename = "tcp_raw")]
    TcpRaw,
}

/// Protocol-specific settings for communication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolConfig {
    /// Baud rate for serial connections (ignored for TCP)
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    /// Command/response termination string
    #[serde(default = "default_termination")]
    pub termination: String,
    /// Delay after sending each command (ms)
    #[serde(default = "default_command_delay_ms")]
    pub command_delay_ms: u64,
    /// Timeout for read operations (ms)
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// TCP host address (required for tcp_scpi/tcp_raw)
    #[serde(default)]
    pub tcp_host: Option<String>,
    /// TCP port number (required for tcp_scpi/tcp_raw)
    #[serde(default)]
    pub tcp_port: Option<u16>,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            baud_rate: default_baud_rate(),
            termination: default_termination(),
            command_delay_ms: default_command_delay_ms(),
            timeout_ms: default_timeout_ms(),
            tcp_host: None,
            tcp_port: None,
        }
    }
}

fn default_baud_rate() -> u32 {
    9600
}
fn default_termination() -> String {
    "\r\n".to_string()
}
fn default_command_delay_ms() -> u64 {
    0
}
fn default_timeout_ms() -> u64 {
    1000
}

/// Defines a sequence of commands for `on_connect` or `on_disconnect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandSequence {
    pub cmd: String,
    #[serde(default)]
    pub wait_ms: u64,
}

/// Defines the various capabilities an instrument can have.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CapabilitiesConfig {
    #[serde(default)]
    pub readable: Vec<ReadableCapability>,
    #[serde(default)]
    pub movable: Option<MovableCapability>,
    #[serde(default)]
    pub settable: Vec<SettableCapability>,
    #[serde(default)]
    pub switchable: Vec<SwitchableCapability>,
    #[serde(default)]
    pub actionable: Vec<ActionableCapability>,
    #[serde(default)]
    pub loggable: Vec<LoggableCapability>,
    #[serde(default)]
    pub scriptable: Vec<ScriptableCapability>,
    #[serde(default)]
    pub frame_producer: Option<FrameProducerCapability>,
    #[serde(default)]
    pub exposure_control: Option<ExposureControlCapability>,
    #[serde(default)]
    pub triggerable: Option<TriggerableCapability>,
}

/// EXPOSURE CONTROL capability: For camera exposure/integration time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExposureControlCapability {
    pub set_cmd: String,
    pub get_cmd: String,
    pub get_pattern: String,
    #[serde(default)]
    pub min_seconds: Option<f64>,
    #[serde(default)]
    pub max_seconds: Option<f64>,
    #[serde(default)]
    pub mock: Option<MockData>,
}

/// READABLE capability: For reading sensor values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadableCapability {
    pub name: String,
    pub command: String,
    pub pattern: String, // Friendly parsing pattern (e.g., "{val} W")
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub mock: Option<MockData>,
}

/// MOVABLE capability: For motion control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MovableCapability {
    pub axes: Vec<AxisConfig>,
    pub set_cmd: String,
    pub get_cmd: String,
    pub get_pattern: String,
}

/// Configuration for a single axis in MovableCapability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisConfig {
    pub name: String,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}

/// TRIGGERABLE capability: For external triggering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerableCapability {
    pub arm_cmd: String,
    pub trigger_cmd: String,
    #[serde(default)]
    pub status_cmd: Option<String>,
    #[serde(default)]
    pub status_pattern: Option<String>,
    #[serde(default)]
    pub armed_value: Option<String>,
}

/// SETTABLE capability: For configuring parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettableCapability {
    pub name: String,
    pub set_cmd: String,
    pub get_cmd: Option<String>, // Some settables might not be readable
    pub pattern: String,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub value_type: ValueType, // e.g., float, int, enum
    #[serde(default)]
    pub options: Vec<String>, // For enum types
    #[serde(default)]
    pub mock: Option<MockData>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum ValueType {
    #[default]
    #[serde(rename = "float")]
    Float,
    #[serde(rename = "int")]
    Int,
    #[serde(rename = "string")]
    String,
    #[serde(rename = "enum")]
    Enum,
    #[serde(rename = "bool")]
    Bool,
}

/// SWITCHABLE capability: For ON/OFF states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwitchableCapability {
    pub name: String,
    pub on_cmd: String,
    pub off_cmd: String,
    pub status_cmd: Option<String>, // Some might not have a status query
    pub pattern: Option<String>,    // Pattern to parse status
    #[serde(default)]
    pub mock: Option<MockData>,
}

/// ACTIONABLE capability: For one-time actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionableCapability {
    pub name: String,
    pub cmd: String,
    #[serde(default)]
    pub wait_ms: u64, // Delay after command for device to process
}

/// SCRIPTABLE capability: For complex Rhai-scripted sequences.
///
/// Allows embedding Rhai scripts in YAML for multi-step operations,
/// state machines, or conditional logic that can't be expressed as
/// simple command/response patterns.
///
/// # Example YAML
///
/// ```yaml
/// capabilities:
///   scriptable:
///     - name: "safe_shutdown"
///       description: "Gracefully shut down the laser with safety checks"
///       script: |
///         // Check if laser is already off
///         let power = driver.read("power");
///         if power < 0.1 {
///           return "Already off";
///         }
///         // Ramp down power gradually
///         for level in [80, 60, 40, 20, 0] {
///           driver.set("power_setpoint", level);
///           sleep(0.5);
///         }
///         driver.switch_off("emission");
///         "Shutdown complete"
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptableCapability {
    /// Unique name for this script capability.
    pub name: String,

    /// Human-readable description of what this script does.
    #[serde(default)]
    pub description: Option<String>,

    /// Rhai script source code.
    ///
    /// The script has access to a `driver` object with methods:
    /// - `driver.read(name)` - Read a named readable capability
    /// - `driver.set(name, value)` - Set a named settable capability
    /// - `driver.get(name)` - Get a named settable capability
    /// - `driver.switch_on(name)` - Turn on a named switchable
    /// - `driver.switch_off(name)` - Turn off a named switchable
    /// - `driver.action(name)` - Execute a named actionable
    /// - `driver.command(cmd)` - Send raw command and get response
    ///
    /// Global functions:
    /// - `sleep(seconds)` - Sleep for specified seconds
    /// - `print(msg)` - Print to log
    pub script: String,

    /// Optional timeout in milliseconds for script execution.
    /// Defaults to 30000ms (30 seconds).
    #[serde(default = "default_script_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_script_timeout_ms() -> u64 {
    30_000
}

/// LOGGABLE capability: For static metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoggableCapability {
    pub name: String,
    pub cmd: String,
    pub pattern: String,
    #[serde(default)]
    pub mock: Option<MockData>,
}

/// FRAME_PRODUCER capability: For camera-like devices that produce 2D images.
///
/// # Example YAML
///
/// ```yaml
/// capabilities:
///   frame_producer:
///     width: 1024
///     height: 1024
///     start_cmd: "START_ACQ"
///     stop_cmd: "STOP_ACQ"
///     frame_cmd: "GET_FRAME"
///     mock:
///       pattern: "checkerboard"  # or "gradient", "noise", "flat"
///       intensity: 1000
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameProducerCapability {
    /// Frame width in pixels
    pub width: u32,
    /// Frame height in pixels
    pub height: u32,
    /// Command to start streaming/acquisition
    pub start_cmd: String,
    /// Command to stop streaming/acquisition
    pub stop_cmd: String,
    /// Command to retrieve a single frame (returns binary data)
    pub frame_cmd: String,
    /// Optional status query command to check if streaming
    #[serde(default)]
    pub status_cmd: Option<String>,
    /// Pattern to parse status response (e.g., "STATUS:{state}")
    #[serde(default)]
    pub status_pattern: Option<String>,
    /// Mock frame generation configuration
    #[serde(default)]
    pub mock: Option<MockFrameConfig>,
}

/// Mock frame generation configuration for simulated cameras.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MockFrameConfig {
    /// Frame pattern type: "checkerboard", "gradient", "noise", "flat"
    #[serde(default = "default_mock_pattern")]
    pub pattern: String,
    /// Base intensity level for mock frames (0-65535 for u16)
    #[serde(default = "default_mock_intensity")]
    pub intensity: u16,
}

fn default_mock_pattern() -> String {
    "checkerboard".to_string()
}
fn default_mock_intensity() -> u16 {
    1000
}

/// Mock data generation for simulation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MockData {
    pub default: f64,
    #[serde(default = "default_mock_jitter")]
    pub jitter: f64,
}

fn default_mock_jitter() -> f64 {
    0.0
}

/// UI Layout elements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UiElement {
    #[serde(rename = "group")]
    Group(UIGroup),
    #[serde(rename = "slider")]
    Slider(UISlider),
    #[serde(rename = "readout")]
    Readout(UIReadout),
    #[serde(rename = "toggle")]
    Toggle(UIToggle),
    #[serde(rename = "button")]
    Button(UIButton),
    #[serde(rename = "dropdown")]
    Dropdown(UIDropdown),
    // Add more UI elements as needed
}

/// UI Grouping element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UIGroup {
    pub label: String,
    #[serde(default)]
    pub children: Vec<UiElement>,
}

/// UI Slider element (for movable/settable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UISlider {
    pub target: String, // Links to a capability name (e.g., movable axis, settable param)
    // Min/Max/Unit can be inherited from capability, or optionally overridden here
    #[serde(default)]
    pub label: Option<String>,
}

/// UI Readout element (for readable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UIReadout {
    pub source: String, // Links to a readable capability name
    #[serde(default)]
    pub label: Option<String>,
}

/// UI Toggle element (for switchable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UIToggle {
    pub target: String, // Links to a switchable capability name
    #[serde(default)]
    pub label: Option<String>,
}

/// UI Button element (for actionable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UIButton {
    pub action: String, // Links to an actionable capability name
    pub label: String,
}

/// UI Dropdown element (for settable enums).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UIDropdown {
    pub target: String, // Links to a settable enum capability
    #[serde(default)]
    pub label: Option<String>,
}
