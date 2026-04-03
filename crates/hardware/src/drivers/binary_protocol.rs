//! Binary protocol support for config-driven drivers.
//!
//! This module provides frame building and parsing for binary protocols like
//! Modbus RTU, enabling declarative configuration of binary command/response
//! formats in TOML.
//!
//! # Features
//!
//! - **Frame builder**: Construct binary frames from field definitions
//! - **CRC calculation**: Multiple CRC algorithms (CRC-16 Modbus, CRC-32, etc.)
//! - **Response parsing**: Parse binary responses into typed values
//! - **Endianness support**: Big-endian and little-endian field types
//!
//! # Example: Modbus RTU Read Holding Registers
//!
//! ```toml
//! [binary_commands.read_registers]
//! description = "Read holding registers (function 0x03)"
//! fields = [
//!     { name = "address", type = "u8", value = "${device_address}" },
//!     { name = "function", type = "u8", value = "0x03" },
//!     { name = "start_register", type = "u16_be", value = "${start_register}" },
//!     { name = "count", type = "u16_be", value = "${count}" },
//! ]
//! crc = { algorithm = "crc16_modbus", append = true }
//! ```

use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use tracing::{instrument, warn};

#[cfg(feature = "binary_protocol")]
use tracing::debug;

use crate::config::schema::{
    BinaryCommandConfig, BinaryFieldConfig, BinaryFieldType, BinaryResponseConfig,
    BinaryResponseFieldConfig,
};

#[cfg(feature = "binary_protocol")]
use crate::config::schema::{ByteOrder, CrcAlgorithm, CrcConfig};

/// Computed CRC value with its byte representation.
#[derive(Debug, Clone)]
pub struct CrcValue {
    /// The raw CRC value
    pub value: u64,
    /// Byte representation in the configured byte order
    pub bytes: Vec<u8>,
}

/// Calculate CRC for the given data using the specified algorithm.
#[cfg(feature = "binary_protocol")]
#[allow(clippy::cast_possible_truncation, clippy::cast_lossless)]
// SAFETY: CRC values are produced at known widths (8, 16, or 32 bits) and stored in
// u64 for uniformity. The narrowing casts back to u8/u16/u32 for byte encoding are
// exact because the value was originally computed at that width.
pub fn calculate_crc(data: &[u8], config: &CrcConfig) -> CrcValue {
    use crc::{CRC_16_IBM_SDLC, CRC_16_MODBUS, CRC_16_XMODEM, CRC_32_ISCSI, CRC_32_ISO_HDLC, Crc};

    let (value, size): (u64, usize) = match config.algorithm {
        CrcAlgorithm::Crc16Modbus => {
            let crc = Crc::<u16>::new(&CRC_16_MODBUS);
            (crc.checksum(data) as u64, 2)
        }
        CrcAlgorithm::Crc16Ccitt | CrcAlgorithm::Crc16CcittFalse => {
            // CRC-16-IBM-SDLC is also known as CRC-16-CCITT
            let crc = Crc::<u16>::new(&CRC_16_IBM_SDLC);
            (crc.checksum(data) as u64, 2)
        }
        CrcAlgorithm::Crc16Xmodem => {
            let crc = Crc::<u16>::new(&CRC_16_XMODEM);
            (crc.checksum(data) as u64, 2)
        }
        CrcAlgorithm::Crc32 => {
            let crc = Crc::<u32>::new(&CRC_32_ISO_HDLC);
            (crc.checksum(data) as u64, 4)
        }
        CrcAlgorithm::Crc32C => {
            let crc = Crc::<u32>::new(&CRC_32_ISCSI);
            (crc.checksum(data) as u64, 4)
        }
        CrcAlgorithm::Checksum8 => {
            let sum: u8 = data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
            (sum as u64, 1)
        }
        CrcAlgorithm::Xor8 | CrcAlgorithm::Lrc => {
            let xor: u8 = data.iter().fold(0u8, |acc, &b| acc ^ b);
            (xor as u64, 1)
        }
    };

    let bytes = match (size, config.byte_order) {
        (1, _) => vec![value as u8],
        (2, ByteOrder::LittleEndian) => (value as u16).to_le_bytes().to_vec(),
        (2, ByteOrder::BigEndian) => (value as u16).to_be_bytes().to_vec(),
        (4, ByteOrder::LittleEndian) => (value as u32).to_le_bytes().to_vec(),
        (4, ByteOrder::BigEndian) => (value as u32).to_be_bytes().to_vec(),
        _ => vec![],
    };

    CrcValue { value, bytes }
}

/// Validate CRC for the given frame (data + CRC bytes).
#[cfg(feature = "binary_protocol")]
pub fn validate_crc(frame: &[u8], config: &CrcConfig) -> Result<bool> {
    let crc_size = match config.algorithm {
        CrcAlgorithm::Crc16Modbus
        | CrcAlgorithm::Crc16Ccitt
        | CrcAlgorithm::Crc16CcittFalse
        | CrcAlgorithm::Crc16Xmodem => 2,
        CrcAlgorithm::Crc32 | CrcAlgorithm::Crc32C => 4,
        CrcAlgorithm::Checksum8 | CrcAlgorithm::Xor8 | CrcAlgorithm::Lrc => 1,
    };

    if frame.len() < crc_size {
        return Err(anyhow!(
            "Frame too short for CRC validation: {} bytes, need at least {}",
            frame.len(),
            crc_size
        ));
    }

    let data = &frame[..frame.len() - crc_size];
    let received_crc = &frame[frame.len() - crc_size..];

    let calculated = calculate_crc(data, config);
    Ok(calculated.bytes == received_crc)
}

/// Binary frame builder for constructing protocol frames.
#[derive(Debug)]
pub struct BinaryFrameBuilder {
    buffer: Vec<u8>,
}

impl BinaryFrameBuilder {
    /// Create a new frame builder.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Create a new frame builder with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
        }
    }

    /// Build a frame from a binary command configuration.
    #[instrument(skip(self, config, params), fields(field_count = config.fields.len()), err)]
    pub fn build_frame(
        &mut self,
        config: &BinaryCommandConfig,
        params: &HashMap<String, f64>,
    ) -> Result<Vec<u8>> {
        self.buffer.clear();

        for field in &config.fields {
            self.append_field(field, params)
                .with_context(|| format!("Failed to append field '{}'", field.name))?;
        }

        // Append CRC if configured
        #[cfg(feature = "binary_protocol")]
        if let Some(ref crc_config) = config.crc {
            if crc_config.append {
                let crc = calculate_crc(&self.buffer, crc_config);
                debug!(
                    crc_value = %crc.value,
                    crc_bytes = ?crc.bytes,
                    "Appending CRC to frame"
                );
                self.buffer.extend_from_slice(&crc.bytes);
            }
        }

        Ok(self.buffer.clone())
    }

    /// Append a single field to the frame.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    // SAFETY: Binary protocol fields are typed by the TOML config (u8, i16, u32, etc.).
    // The f64 intermediate representation is narrowed to match the declared field type.
    // Values are expected to be within the target type's range per the device protocol spec.
    fn append_field(
        &mut self,
        field: &BinaryFieldConfig,
        params: &HashMap<String, f64>,
    ) -> Result<()> {
        // Handle fixed byte array
        if let Some(ref bytes) = field.bytes {
            self.buffer.extend_from_slice(bytes);
            return Ok(());
        }

        // Handle value template
        let value_str = field
            .value
            .as_ref()
            .ok_or_else(|| anyhow!("Field '{}' has no value or bytes specified", field.name))?;

        let value = self.resolve_value(value_str, params)?;

        match field.field_type {
            BinaryFieldType::U8 => {
                let v = value as u8;
                self.buffer.push(v);
            }
            BinaryFieldType::I8 => {
                let v = value as i8;
                self.buffer.push(v as u8);
            }
            BinaryFieldType::U16Be => {
                let v = value as u16;
                self.buffer.extend_from_slice(&v.to_be_bytes());
            }
            BinaryFieldType::U16Le => {
                let v = value as u16;
                self.buffer.extend_from_slice(&v.to_le_bytes());
            }
            BinaryFieldType::I16Be => {
                let v = value as i16;
                self.buffer.extend_from_slice(&v.to_be_bytes());
            }
            BinaryFieldType::I16Le => {
                let v = value as i16;
                self.buffer.extend_from_slice(&v.to_le_bytes());
            }
            BinaryFieldType::U32Be => {
                let v = value as u32;
                self.buffer.extend_from_slice(&v.to_be_bytes());
            }
            BinaryFieldType::U32Le => {
                let v = value as u32;
                self.buffer.extend_from_slice(&v.to_le_bytes());
            }
            BinaryFieldType::I32Be => {
                let v = value as i32;
                self.buffer.extend_from_slice(&v.to_be_bytes());
            }
            BinaryFieldType::I32Le => {
                let v = value as i32;
                self.buffer.extend_from_slice(&v.to_le_bytes());
            }
            BinaryFieldType::F32Be => {
                let v = value as f32;
                self.buffer.extend_from_slice(&v.to_be_bytes());
            }
            BinaryFieldType::F32Le => {
                let v = value as f32;
                self.buffer.extend_from_slice(&v.to_le_bytes());
            }
            BinaryFieldType::U64Be => {
                let v = value as u64;
                self.buffer.extend_from_slice(&v.to_be_bytes());
            }
            BinaryFieldType::U64Le => {
                let v = value as u64;
                self.buffer.extend_from_slice(&v.to_le_bytes());
            }
            BinaryFieldType::Bytes
            | BinaryFieldType::AsciiString
            | BinaryFieldType::AsciiStringZ => {
                // For string/bytes types, the value should be the raw string
                let s = value_str.trim_start_matches("${").trim_end_matches('}');
                if let Some(&param_value) = params.get(s) {
                    // If it's a parameter, convert to string representation
                    self.buffer
                        .extend_from_slice(param_value.to_string().as_bytes());
                } else {
                    // Otherwise, use the literal string (minus any param syntax)
                    self.buffer.extend_from_slice(value_str.as_bytes());
                }
                if field.field_type == BinaryFieldType::AsciiStringZ {
                    self.buffer.push(0); // Null terminator
                }
            }
        }

        Ok(())
    }

    /// Resolve a value template to a numeric value.
    ///
    /// Supports:
    /// - Hex literals: "0x03", "0xFF"
    /// - Parameter references: "${device_address}", "${count}"
    /// - Decimal literals: "123", "45.6"
    #[allow(clippy::cast_precision_loss)]
    // SAFETY: Hex literals in protocol configs are small register addresses/values
    // that fit within f64's 52-bit mantissa without precision loss.
    fn resolve_value(&self, template: &str, params: &HashMap<String, f64>) -> Result<f64> {
        let template = template.trim();

        // Hex literal
        if template.starts_with("0x") || template.starts_with("0X") {
            let hex_str = template.trim_start_matches("0x").trim_start_matches("0X");
            let value = u64::from_str_radix(hex_str, 16)
                .with_context(|| format!("Invalid hex literal: {}", template))?;
            return Ok(value as f64);
        }

        // Parameter reference
        if template.starts_with("${") && template.ends_with('}') {
            let param_name = &template[2..template.len() - 1];
            let value = params
                .get(param_name)
                .ok_or_else(|| anyhow!("Parameter '{}' not found", param_name))?;
            return Ok(*value);
        }

        // Decimal literal
        template
            .parse::<f64>()
            .with_context(|| format!("Invalid numeric value: {}", template))
    }

    /// Get the current frame contents.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Take ownership of the frame buffer.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }
}

impl Default for BinaryFrameBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Parsed value from a binary response field.
#[derive(Debug, Clone)]
pub enum ParsedValue {
    /// Unsigned integer value
    Unsigned(u64),
    /// Signed integer value
    Signed(i64),
    /// Floating point value
    Float(f64),
    /// Raw bytes
    Bytes(Vec<u8>),
    /// ASCII string
    String(String),
}

impl ParsedValue {
    /// Convert to f64 if possible.
    #[allow(clippy::cast_precision_loss)]
    // SAFETY: Lossy conversion is inherent to the f64 representation. Callers
    // accept precision loss for display/calculation purposes.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParsedValue::Unsigned(v) => Some(*v as f64),
            ParsedValue::Signed(v) => Some(*v as f64),
            ParsedValue::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// Convert to i64 if possible.
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    // SAFETY: Lossy conversion is inherent. Unsigned values > i64::MAX will wrap;
    // float values are truncated. Callers accept this for protocol field access.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ParsedValue::Unsigned(v) => Some(*v as i64),
            ParsedValue::Signed(v) => Some(*v),
            ParsedValue::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    /// Convert to bytes.
    pub fn as_bytes(&self) -> Vec<u8> {
        match self {
            ParsedValue::Bytes(b) => b.clone(),
            ParsedValue::String(s) => s.as_bytes().to_vec(),
            ParsedValue::Unsigned(v) => v.to_le_bytes().to_vec(),
            ParsedValue::Signed(v) => v.to_le_bytes().to_vec(),
            ParsedValue::Float(v) => v.to_le_bytes().to_vec(),
        }
    }
}

/// Binary response parser.
#[derive(Debug)]
pub struct BinaryResponseParser;

impl BinaryResponseParser {
    /// Parse a binary response frame using the provided configuration.
    #[allow(clippy::cast_sign_loss)]
    // SAFETY: min_length/max_length from config are small positive protocol frame
    // sizes that fit in usize.
    #[instrument(skip(data, config), fields(data_len = data.len()), err)]
    pub fn parse(
        data: &[u8],
        config: &BinaryResponseConfig,
    ) -> Result<HashMap<String, ParsedValue>> {
        // Validate length constraints
        if let Some(min_len) = config.min_length {
            if data.len() < min_len as usize {
                return Err(anyhow!(
                    "Response too short: {} bytes, expected at least {}",
                    data.len(),
                    min_len
                ));
            }
        }
        if let Some(max_len) = config.max_length {
            if data.len() > max_len as usize {
                return Err(anyhow!(
                    "Response too long: {} bytes, expected at most {}",
                    data.len(),
                    max_len
                ));
            }
        }

        // Validate CRC if configured
        #[cfg(feature = "binary_protocol")]
        if let Some(ref crc_config) = config.crc {
            if crc_config.validate {
                let valid = validate_crc(data, crc_config)?;
                if !valid {
                    return Err(anyhow!("CRC validation failed"));
                }
                debug!("CRC validation passed");
            }
        }

        let mut result = HashMap::new();
        let mut offset = 0;

        for field in &config.fields {
            let (value, consumed) = Self::parse_field(data, field, &result, offset)?;
            result.insert(field.name.clone(), value);

            // Update offset if the field has a fixed position
            if let Some(pos) = field.position {
                offset = pos + consumed;
            } else if let Some(start) = field.start {
                offset = start + consumed;
            } else {
                offset += consumed;
            }
        }

        Ok(result)
    }

    /// Parse a single field from the response.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
    // SAFETY: Length fields from binary protocols are small positive values that
    // fit in usize. Signed-to-unsigned comparison for expected value validation is
    // intentional for protocol field matching.
    fn parse_field(
        data: &[u8],
        field: &BinaryResponseFieldConfig,
        parsed_so_far: &HashMap<String, ParsedValue>,
        current_offset: usize,
    ) -> Result<(ParsedValue, usize)> {
        // Determine start position
        let start = field.position.or(field.start).unwrap_or(current_offset);

        if start >= data.len() {
            return Err(anyhow!(
                "Field '{}' start position {} exceeds data length {}",
                field.name,
                start,
                data.len()
            ));
        }

        // Determine length for variable-length fields
        let length = if let Some(len) = field.length {
            len
        } else if let Some(ref len_field) = field.length_field {
            let len_value = parsed_so_far
                .get(len_field)
                .ok_or_else(|| anyhow!("Length field '{}' not found", len_field))?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                len_value
                    .as_i64()
                    .ok_or_else(|| anyhow!("Length field '{}' is not numeric", len_field))?
                    as usize
            }
        } else {
            field.field_type.fixed_size().unwrap_or(1)
        };

        if start + length > data.len() {
            return Err(anyhow!(
                "Field '{}' extends beyond data: start={}, length={}, data_len={}",
                field.name,
                start,
                length,
                data.len()
            ));
        }

        let field_data = &data[start..start + length];

        let value = match field.field_type {
            BinaryFieldType::U8 => ParsedValue::Unsigned(u64::from(field_data[0])),
            BinaryFieldType::I8 => ParsedValue::Signed(i64::from(field_data[0] as i8)),
            BinaryFieldType::U16Be => {
                let v = u16::from_be_bytes([field_data[0], field_data[1]]);
                ParsedValue::Unsigned(u64::from(v))
            }
            BinaryFieldType::U16Le => {
                let v = u16::from_le_bytes([field_data[0], field_data[1]]);
                ParsedValue::Unsigned(u64::from(v))
            }
            BinaryFieldType::I16Be => {
                let v = i16::from_be_bytes([field_data[0], field_data[1]]);
                ParsedValue::Signed(i64::from(v))
            }
            BinaryFieldType::I16Le => {
                let v = i16::from_le_bytes([field_data[0], field_data[1]]);
                ParsedValue::Signed(i64::from(v))
            }
            BinaryFieldType::U32Be => {
                let v = u32::from_be_bytes([
                    field_data[0],
                    field_data[1],
                    field_data[2],
                    field_data[3],
                ]);
                ParsedValue::Unsigned(u64::from(v))
            }
            BinaryFieldType::U32Le => {
                let v = u32::from_le_bytes([
                    field_data[0],
                    field_data[1],
                    field_data[2],
                    field_data[3],
                ]);
                ParsedValue::Unsigned(u64::from(v))
            }
            BinaryFieldType::I32Be => {
                let v = i32::from_be_bytes([
                    field_data[0],
                    field_data[1],
                    field_data[2],
                    field_data[3],
                ]);
                ParsedValue::Signed(i64::from(v))
            }
            BinaryFieldType::I32Le => {
                let v = i32::from_le_bytes([
                    field_data[0],
                    field_data[1],
                    field_data[2],
                    field_data[3],
                ]);
                ParsedValue::Signed(i64::from(v))
            }
            BinaryFieldType::F32Be => {
                let v = f32::from_be_bytes([
                    field_data[0],
                    field_data[1],
                    field_data[2],
                    field_data[3],
                ]);
                ParsedValue::Float(f64::from(v))
            }
            BinaryFieldType::F32Le => {
                let v = f32::from_le_bytes([
                    field_data[0],
                    field_data[1],
                    field_data[2],
                    field_data[3],
                ]);
                ParsedValue::Float(f64::from(v))
            }
            BinaryFieldType::U64Be => {
                let v = u64::from_be_bytes([
                    field_data[0],
                    field_data[1],
                    field_data[2],
                    field_data[3],
                    field_data[4],
                    field_data[5],
                    field_data[6],
                    field_data[7],
                ]);
                ParsedValue::Unsigned(v)
            }
            BinaryFieldType::U64Le => {
                let v = u64::from_le_bytes([
                    field_data[0],
                    field_data[1],
                    field_data[2],
                    field_data[3],
                    field_data[4],
                    field_data[5],
                    field_data[6],
                    field_data[7],
                ]);
                ParsedValue::Unsigned(v)
            }
            BinaryFieldType::Bytes => ParsedValue::Bytes(field_data.to_vec()),
            BinaryFieldType::AsciiString => {
                let s = String::from_utf8_lossy(field_data).to_string();
                ParsedValue::String(s)
            }
            BinaryFieldType::AsciiStringZ => {
                // Find null terminator
                let end = field_data.iter().position(|&b| b == 0).unwrap_or(length);
                let s = String::from_utf8_lossy(&field_data[..end]).to_string();
                ParsedValue::String(s)
            }
        };

        // Validate expected value if specified
        if let Some(ref expected) = field.expected {
            let expected_value = if expected.starts_with("0x") || expected.starts_with("0X") {
                let hex = expected.trim_start_matches("0x").trim_start_matches("0X");
                u64::from_str_radix(hex, 16).ok()
            } else {
                expected.parse::<u64>().ok()
            };

            if let Some(expected_num) = expected_value {
                if let Some(actual_num) = match &value {
                    ParsedValue::Unsigned(v) => Some(*v),
                    ParsedValue::Signed(v) => Some(*v as u64),
                    _ => None,
                } {
                    if actual_num != expected_num {
                        warn!(
                            field = %field.name,
                            expected = %expected_num,
                            actual = %actual_num,
                            "Field value does not match expected"
                        );
                    }
                }
            }
        }

        Ok((value, length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_builder_basic() {
        let mut builder = BinaryFrameBuilder::new();

        let config = BinaryCommandConfig {
            description: "Test command".to_string(),
            fields: vec![
                BinaryFieldConfig {
                    name: "address".to_string(),
                    field_type: BinaryFieldType::U8,
                    value: Some("0x01".to_string()),
                    bytes: None,
                    length: None,
                },
                BinaryFieldConfig {
                    name: "function".to_string(),
                    field_type: BinaryFieldType::U8,
                    value: Some("0x03".to_string()),
                    bytes: None,
                    length: None,
                },
                BinaryFieldConfig {
                    name: "register".to_string(),
                    field_type: BinaryFieldType::U16Be,
                    value: Some("${start_register}".to_string()),
                    bytes: None,
                    length: None,
                },
            ],
            crc: None,
            expects_response: true,
            response: None,
            timeout_ms: None,
            retry: None,
        };

        let mut params = HashMap::new();
        params.insert("start_register".to_string(), 100.0);

        let frame = builder.build_frame(&config, &params).unwrap();
        assert_eq!(frame, vec![0x01, 0x03, 0x00, 0x64]); // 100 = 0x0064 big-endian
    }

    #[test]
    fn test_frame_builder_with_fixed_bytes() {
        let mut builder = BinaryFrameBuilder::new();

        let config = BinaryCommandConfig {
            description: "Test with fixed bytes".to_string(),
            fields: vec![
                BinaryFieldConfig {
                    name: "header".to_string(),
                    field_type: BinaryFieldType::Bytes,
                    value: None,
                    bytes: Some(vec![0xAA, 0x55]),
                    length: None,
                },
                BinaryFieldConfig {
                    name: "value".to_string(),
                    field_type: BinaryFieldType::U16Le,
                    value: Some("1000".to_string()),
                    bytes: None,
                    length: None,
                },
            ],
            crc: None,
            expects_response: true,
            response: None,
            timeout_ms: None,
            retry: None,
        };

        let frame = builder.build_frame(&config, &HashMap::new()).unwrap();
        assert_eq!(frame, vec![0xAA, 0x55, 0xE8, 0x03]); // 1000 = 0x03E8 little-endian
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_resolve_hex_value() {
        let builder = BinaryFrameBuilder::new();
        let params = HashMap::new();

        assert_eq!(builder.resolve_value("0x03", &params).unwrap(), 3.0);
        assert_eq!(builder.resolve_value("0xFF", &params).unwrap(), 255.0);
        assert_eq!(builder.resolve_value("0x0100", &params).unwrap(), 256.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_resolve_param_value() {
        let builder = BinaryFrameBuilder::new();
        let mut params = HashMap::new();
        params.insert("count".to_string(), 42.0);

        assert_eq!(builder.resolve_value("${count}", &params).unwrap(), 42.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_resolve_decimal_value() {
        let builder = BinaryFrameBuilder::new();
        let params = HashMap::new();

        assert_eq!(builder.resolve_value("123", &params).unwrap(), 123.0);
        assert_eq!(builder.resolve_value("45.6", &params).unwrap(), 45.6);
    }

    #[test]
    fn test_response_parser_basic() {
        let config = BinaryResponseConfig {
            description: "Test response".to_string(),
            fields: vec![
                BinaryResponseFieldConfig {
                    name: "address".to_string(),
                    field_type: BinaryFieldType::U8,
                    position: Some(0),
                    start: None,
                    length: None,
                    length_field: None,
                    expected: None,
                    is_error_code: false,
                },
                BinaryResponseFieldConfig {
                    name: "function".to_string(),
                    field_type: BinaryFieldType::U8,
                    position: Some(1),
                    start: None,
                    length: None,
                    length_field: None,
                    expected: Some("0x03".to_string()),
                    is_error_code: false,
                },
                BinaryResponseFieldConfig {
                    name: "value".to_string(),
                    field_type: BinaryFieldType::U16Be,
                    position: Some(2),
                    start: None,
                    length: None,
                    length_field: None,
                    expected: None,
                    is_error_code: false,
                },
            ],
            crc: None,
            min_length: Some(4),
            max_length: Some(10),
        };

        let data = vec![0x01, 0x03, 0x01, 0x00]; // address=1, function=3, value=256
        let result = BinaryResponseParser::parse(&data, &config).unwrap();

        assert_eq!(result.get("address").unwrap().as_i64().unwrap(), 1);
        assert_eq!(result.get("function").unwrap().as_i64().unwrap(), 3);
        assert_eq!(result.get("value").unwrap().as_i64().unwrap(), 256);
    }

    #[test]
    fn test_response_parser_variable_length() {
        let config = BinaryResponseConfig {
            description: "Test variable length".to_string(),
            fields: vec![
                BinaryResponseFieldConfig {
                    name: "byte_count".to_string(),
                    field_type: BinaryFieldType::U8,
                    position: Some(0),
                    start: None,
                    length: None,
                    length_field: None,
                    expected: None,
                    is_error_code: false,
                },
                BinaryResponseFieldConfig {
                    name: "data".to_string(),
                    field_type: BinaryFieldType::Bytes,
                    position: None,
                    start: Some(1),
                    length: None,
                    length_field: Some("byte_count".to_string()),
                    expected: None,
                    is_error_code: false,
                },
            ],
            crc: None,
            min_length: None,
            max_length: None,
        };

        let data = vec![0x03, 0xAA, 0xBB, 0xCC]; // byte_count=3, data=[AA, BB, CC]
        let result = BinaryResponseParser::parse(&data, &config).unwrap();

        assert_eq!(result.get("byte_count").unwrap().as_i64().unwrap(), 3);
        assert_eq!(
            result.get("data").unwrap().as_bytes(),
            vec![0xAA, 0xBB, 0xCC]
        );
    }

    #[cfg(feature = "binary_protocol")]
    #[test]
    fn test_crc16_modbus() {
        // Known Modbus CRC test vector: [01 03 00 00 00 01] -> CRC = 0x0A84
        // Little-endian byte order: [0x84, 0x0A]
        let data = vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x01];
        let config = CrcConfig {
            algorithm: CrcAlgorithm::Crc16Modbus,
            append: true,
            validate: true,
            byte_order: ByteOrder::LittleEndian,
        };

        let crc = calculate_crc(&data, &config);
        assert_eq!(crc.value, 0x0A84);
        assert_eq!(crc.bytes, vec![0x84, 0x0A]); // Little-endian
    }

    #[cfg(feature = "binary_protocol")]
    #[test]
    fn test_crc_validation() {
        let data = vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x01];
        let config = CrcConfig {
            algorithm: CrcAlgorithm::Crc16Modbus,
            append: true,
            validate: true,
            byte_order: ByteOrder::LittleEndian,
        };

        // Calculate CRC and append
        let crc = calculate_crc(&data, &config);
        let mut frame = data.clone();
        frame.extend_from_slice(&crc.bytes);

        // Validate should pass
        assert!(validate_crc(&frame, &config).unwrap());

        // Corrupt the frame
        let mut corrupted = frame.clone();
        corrupted[2] = 0xFF;
        assert!(!validate_crc(&corrupted, &config).unwrap());
    }
}
