use crate::config::ResponseType;
use std::str::FromStr;
use winnow::ascii::{dec_int, float, multispace0, Caseless};
use winnow::combinator::{alt, delimited, separated};
use winnow::{ModalResult, Parser};

#[derive(Debug, Clone, PartialEq)]
pub enum ParsedValue {
    None,
    String(String),
    Float(f64),
    Integer(i64),
    Boolean(bool),
    ArrayFloat(Vec<f64>),
}

pub fn parse_scpi_bool(input: &mut &str) -> ModalResult<bool> {
    alt((
        Caseless("ON").value(true),
        Caseless("OFF").value(false),
        Caseless("1").value(true),
        Caseless("0").value(false),
    ))
    .parse_next(input)
}

pub fn parse_scpi_float(input: &mut &str) -> ModalResult<f64> {
    delimited(multispace0, float, multispace0).parse_next(input)
}

pub fn parse_scpi_int(input: &mut &str) -> ModalResult<i64> {
    delimited(multispace0, dec_int, multispace0).parse_next(input)
}

pub fn parse_float_array(input: &mut &str) -> ModalResult<Vec<f64>> {
    separated(0.., parse_scpi_float, ',').parse_next(input)
}

pub fn parse_response(raw: &str, response_type: ResponseType) -> Result<ParsedValue, String> {
    let trimmed = raw.trim();
    let mut input = trimmed;
    match response_type {
        ResponseType::None => Ok(ParsedValue::None),
        ResponseType::String => Ok(ParsedValue::String(trimmed.to_string())),
        ResponseType::Float => parse_scpi_float(&mut input)
            .map(ParsedValue::Float)
            .map_err(|e| format!("Failed to parse float response: {e}")),
        ResponseType::Integer => parse_scpi_int(&mut input)
            .map(ParsedValue::Integer)
            .map_err(|e| format!("Failed to parse integer response: {e}")),
        ResponseType::Boolean => parse_scpi_bool(&mut input)
            .map(ParsedValue::Boolean)
            .map_err(|e| format!("Failed to parse boolean response: {e}")),
        ResponseType::ArrayFloat => parse_float_array(&mut input)
            .map(ParsedValue::ArrayFloat)
            .map_err(|e| format!("Failed to parse float array response: {e}")),
    }
}

impl ParsedValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParsedValue::Float(value) => Some(*value),
            ParsedValue::Integer(value) => Some(*value as f64),
            ParsedValue::Boolean(value) => Some(if *value { 1.0 } else { 0.0 }),
            ParsedValue::String(value) => f64::from_str(value).ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParsedValue::Boolean(value) => Some(*value),
            ParsedValue::Integer(value) => Some(*value != 0),
            ParsedValue::Float(value) => Some(*value != 0.0),
            ParsedValue::String(value) => match value.as_str() {
                "ON" | "on" | "1" => Some(true),
                "OFF" | "off" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_scpi_bool() {
        let mut input = "ON";
        assert!(parse_scpi_bool(&mut input).unwrap());
        let mut input = "0";
        assert!(!parse_scpi_bool(&mut input).unwrap());
    }

    #[test]
    fn test_parse_scpi_float() {
        let mut input = " 1.23E+02 ";
        assert!((parse_scpi_float(&mut input).unwrap() - 123.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_float_array() {
        let mut input = "1.0, 2.0,3.5";
        let parsed = parse_float_array(&mut input).unwrap();
        assert_eq!(parsed, vec![1.0, 2.0, 3.5]);
    }
}
