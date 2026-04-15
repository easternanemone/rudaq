//! Provides a set of validation functions used throughout the application,
//! particularly for checking the configuration settings loaded from files.
//!
//! This module centralizes common validation logic, making it reusable and
//! ensuring that checks are consistent. These functions are designed to be simple
//! predicates that return a `Result<(), &'static str>`, making them easy to
//! integrate with the `anyhow` error handling in the `config` module.
//!
//! ## Validators
//!
//! The module includes functions to validate:
//!
//! - **`is_valid_port`**: Checks if a `u16` is a valid, non-zero network port.
//! - **`is_valid_ip`**: Checks if a string can be parsed as a valid IP address (both IPv4 and IPv6).
//! - **`is_valid_path`**: Performs basic checks on a file path string, ensuring it's not empty
//!   and doesn't contain null bytes.
//! - **`is_in_range`**: A generic function to check if a value falls within a given `RangeInclusive`.
//!   This is useful for settings like sample rates or other numerical parameters.
//! - **`is_not_empty`**: A simple check to ensure that a string setting is not empty.
//!
//! These functions are called from the `Settings::validate` method to ensure that the
//! application doesn't start with a configuration that could lead to runtime errors.

use std::net::IpAddr;
use std::ops::RangeInclusive;

/// Validates if a given `u16` value is a valid port number.
/// By type, the port is already within the 0-65535 range.
/// This function checks that the port is not 0, which is reserved.
///
/// # Arguments
///
/// * `port` - The `u16` value to validate.
///
/// # Returns
///
/// * `Ok(())` if the port is valid.
/// * `Err(&'static str)` if the port is invalid.
pub fn is_valid_port(port: u16) -> Result<(), &'static str> {
    if port > 0 {
        Ok(())
    } else {
        Err("Port number must be greater than 0")
    }
}

/// Validates if a given string is a valid IP address.
///
/// # Arguments
///
/// * `ip` - The string to validate.
///
/// # Returns
///
/// * `Ok(())` if the IP address is valid.
/// * `Err(&'static str)` if the IP address is invalid.
pub fn is_valid_ip(ip: &str) -> Result<(), &'static str> {
    ip.parse::<IpAddr>()
        .map(|_| ())
        .map_err(|_| "Invalid IP address")
}

/// Validates if a given string is a valid file path.
///
/// # Arguments
///
/// * `path` - The string to validate.
///
/// # Returns
///
/// * `Ok(())` if the file path is valid.
/// * `Err(&'static str)` if the file path is invalid.
pub fn is_valid_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("File path cannot be empty");
    }
    if path.contains('\0') {
        return Err("File path cannot contain null bytes");
    }
    Ok(())
}

/// Validates if a given value is within a specified numeric range.
///
/// # Arguments
///
/// * `value` - The value to validate.
/// * `range` - The inclusive range to validate against.
///
/// # Returns
///
/// * `Ok(())` if the value is within the range.
/// * `Err(&'static str)` if the value is outside the range.
pub fn is_in_range<T: PartialOrd>(value: T, range: RangeInclusive<T>) -> Result<(), &'static str> {
    if range.contains(&value) {
        Ok(())
    } else {
        Err("Value is outside the specified range")
    }
}

/// Validates if a given string is not empty.
///
/// # Arguments
///
/// * `value` - The string to validate.
///
/// # Returns
///
/// * `Ok(())` if the string is not empty.
/// * `Err(&'static str)` if the string is empty.
pub fn is_not_empty(value: &str) -> Result<(), &'static str> {
    if !value.is_empty() {
        Ok(())
    } else {
        Err("Value cannot be empty")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_port() {
        assert_eq!(is_valid_port(8080), Ok(()));
        assert_eq!(is_valid_port(1), Ok(()));
        assert_eq!(is_valid_port(65535), Ok(()));
        assert_eq!(is_valid_port(0), Err("Port number must be greater than 0"));
    }

    #[test]
    fn test_is_valid_ip() {
        // Valid IPv4
        assert_eq!(is_valid_ip("192.168.1.1"), Ok(()));
        assert_eq!(is_valid_ip("127.0.0.1"), Ok(()));
        assert_eq!(is_valid_ip("0.0.0.0"), Ok(()));
        assert_eq!(is_valid_ip("255.255.255.255"), Ok(()));

        // Valid IPv6
        assert_eq!(is_valid_ip("::1"), Ok(()));
        assert_eq!(
            is_valid_ip("2001:0db8:85a3:0000:0000:8a2e:0370:7334"),
            Ok(())
        );
        assert_eq!(is_valid_ip("2001:db8::1"), Ok(()));
        assert_eq!(is_valid_ip("::ffff:192.0.2.128"), Ok(()));

        // Invalid IPs
        assert_eq!(is_valid_ip("256.256.256.256"), Err("Invalid IP address"));
        assert_eq!(is_valid_ip("not.an.ip"), Err("Invalid IP address"));
        assert_eq!(is_valid_ip(""), Err("Invalid IP address"));
        assert_eq!(is_valid_ip("192.168.1"), Err("Invalid IP address"));
        assert_eq!(is_valid_ip("192.168.1.1.1"), Err("Invalid IP address"));
        assert_eq!(is_valid_ip("::ffff:192.0.2.256"), Err("Invalid IP address"));
    }

    #[test]
    fn test_is_valid_path() {
        assert_eq!(is_valid_path("/etc/config.toml"), Ok(()));
        assert_eq!(is_valid_path("config.toml"), Ok(()));
        assert_eq!(is_valid_path("C:\\Windows\\System32"), Ok(()));
        assert_eq!(is_valid_path(""), Err("File path cannot be empty"));
        assert_eq!(
            is_valid_path("/etc/config\0.toml"),
            Err("File path cannot contain null bytes")
        );
    }

    #[test]
    fn test_is_in_range() {
        assert_eq!(is_in_range(5, 1..=10), Ok(()));
        assert_eq!(is_in_range(1, 1..=10), Ok(()));
        assert_eq!(is_in_range(10, 1..=10), Ok(()));
        assert_eq!(
            is_in_range(0, 1..=10),
            Err("Value is outside the specified range")
        );
        assert_eq!(
            is_in_range(11, 1..=10),
            Err("Value is outside the specified range")
        );

        assert_eq!(is_in_range(5.0, 1.0..=10.0), Ok(()));
        assert_eq!(
            is_in_range(0.0, 1.0..=10.0),
            Err("Value is outside the specified range")
        );
    }

    #[test]
    fn test_is_not_empty() {
        assert_eq!(is_not_empty("hello"), Ok(()));
        assert_eq!(is_not_empty(" "), Ok(()));
        assert_eq!(is_not_empty(""), Err("Value cannot be empty"));
    }
}
