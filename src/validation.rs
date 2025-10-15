use std::net::IpAddr;
use std::ops::RangeInclusive;

/// Validates if a given u16 value is a valid port number.
/// By type, the port is already within the 0-65535 range.
/// This function checks that the port is not 0, which is reserved.
///
/// # Arguments
///
/// * `port` - The u16 value to validate.
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
    ip.parse::<IpAddr>().map(|_| ()).map_err(|_| "Invalid IP address")
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