use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstrumentConfig {
    pub device: DeviceConfig,
    #[serde(default)]
    pub connection: ConnectionConfig,
    #[serde(default)]
    pub parameters: HashMap<String, ParameterConfig>,
    #[serde(default)]
    pub commands: HashMap<String, CommandConfig>,
    #[serde(default)]
    pub responses: HashMap<String, ResponseConfig>,
    #[serde(default)]
    pub conversions: HashMap<String, ConversionConfig>,
    #[serde(default)]
    pub trait_mapping: TraitMappingConfig,
    #[serde(default)]
    pub error_codes: HashMap<String, ErrorCodeConfig>,
    #[serde(default)]
    pub default_retry: Option<RetryConfig>,
}

impl InstrumentConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (name, response) in &self.responses {
            Regex::new(&response.pattern)
                .map_err(|e| format!("Invalid regex in response '{}': {}", name, e))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceConfig {
    pub name: String,
    pub protocol: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub description: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionConfig {
    #[serde(default)]
    pub r#type: ConnectionType,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    #[serde(default)]
    pub parity: Parity,
    #[serde(default)]
    pub flow_control: FlowControl,
    #[serde(default)]
    pub terminator_tx: String,
    #[serde(default = "default_terminator_rx")]
    pub terminator_rx: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub bus: Option<BusConfig>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            r#type: ConnectionType::Serial,
            baud_rate: default_baud_rate(),
            data_bits: default_data_bits(),
            stop_bits: default_stop_bits(),
            parity: Parity::None,
            flow_control: FlowControl::None,
            terminator_tx: String::new(),
            terminator_rx: default_terminator_rx(),
            timeout_ms: default_timeout_ms(),
            bus: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionType { Serial, Tcp, Udp }

impl Default for ConnectionType { fn default() -> Self { Self::Serial } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Parity { None, Odd, Even }

impl Default for Parity { fn default() -> Self { Self::None } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowControl { None, Software, Hardware }

impl Default for FlowControl { fn default() -> Self { Self::None } }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BusConfig {
    pub r#type: String,
    #[serde(default = "default_address_format")]
    pub address_format: AddressFormat,
    #[serde(default = "default_bus_address")]
    pub default_address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressFormat { HexChar, Decimal, HexByte }

impl Default for AddressFormat { fn default() -> Self { Self::HexChar } }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParameterConfig {
    pub r#type: ParameterType,
    #[serde(default)]
    pub default: serde_json::Value,
    pub range: Option<(serde_json::Value, serde_json::Value)>,
    pub unit: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType { String, Int, Float, Bool }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommandConfig {
    pub template: String,
    pub description: Option<String>,
    #[serde(default = "default_expects_response")]
    pub expects_response: bool,
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    pub response: Option<String>,
    #[serde(default)]
    pub retry: Option<RetryConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseConfig {
    pub pattern: String,
    #[serde(default)]
    pub fields: HashMap<String, ResponseFieldConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResponseFieldConfig {
    pub r#type: ResponseFieldType,
    #[serde(default)]
    pub signed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFieldType { HexU32, HexI32, Int, Float, String }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConversionConfig { pub formula: String }

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TraitMappingConfig {
    #[serde(rename = "Movable")]
    pub movable: Option<HashMap<String, TraitCommandMapping>>,
    #[serde(flatten)]
    pub traits: HashMap<String, HashMap<String, TraitCommandMapping>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TraitCommandMapping {
    pub command: Option<String>,
    pub poll_command: Option<String>,
    pub input_conversion: Option<String>,
    pub input_param: Option<String>,
    pub from_param: Option<String>,
    pub output_conversion: Option<String>,
    pub output_field: Option<String>,
    pub success_condition: Option<String>,
    pub poll_interval_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorCodeConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub severity: ErrorSeverity,
    #[serde(default = "default_recoverable")]
    pub recoverable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity { Warning, Error, Critical }

impl Default for ErrorSeverity { fn default() -> Self { Self::Error } }

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RetryConfig {
    #[serde(default = "default_max_retries")]
    pub max_retries: u8,
    #[serde(default = "default_initial_delay_ms")]
    pub initial_delay_ms: u32,
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u32,
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
    #[serde(default)]
    pub retry_on_errors: Vec<String>,
    #[serde(default)]
    pub no_retry_on_errors: Vec<String>,
}

fn default_baud_rate() -> u32 { 9600 }
fn default_data_bits() -> u8 { 8 }
fn default_stop_bits() -> u8 { 1 }
fn default_terminator_rx() -> String { "\r\n".to_string() }
fn default_timeout_ms() -> u64 { 1000 }
fn default_expects_response() -> bool { true }
fn default_address_format() -> AddressFormat { AddressFormat::HexChar }
fn default_bus_address() -> String { "0".to_string() }
fn default_recoverable() -> bool { true }
fn default_max_retries() -> u8 { 3 }
fn default_initial_delay_ms() -> u32 { 100 }
fn default_max_delay_ms() -> u32 { 1000 }
fn default_backoff_multiplier() -> f64 { 2.0 }
