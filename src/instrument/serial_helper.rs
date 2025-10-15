//! A helper module for serial port communication.
use anyhow::{Context, Result};
use log::trace;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// Sends a command to a serial port and reads the response.
///
/// # Arguments
///
/// * `port` - A mutable reference to the serial port.
/// * `instrument_id` - The ID of the instrument for logging purposes.
/// * `command` - The command string to send.
/// * `terminator` - The terminator string to append to the command.
/// * `timeout` - The maximum time to wait for a response.
/// * `response_terminator` - The character that indicates the end of a response.
///
/// # Returns
///
/// A `Result` containing the response string or an error.
pub fn send_command(
    port: &mut Box<dyn SerialPort>,
    instrument_id: &str,
    command: &str,
    terminator: &str,
    timeout: Duration,
    response_terminator: char,
) -> Result<String> {
    // Append the terminator to the command.
    let cmd = format!("{}{}", command, terminator);
    trace!(
        "Sending command to {}: '{}'",
        instrument_id,
        cmd.escape_default()
    );

    // Write the command to the serial port.
    port.write_all(cmd.as_bytes())
        .with_context(|| format!("Failed to send command to '{}'", instrument_id))?;

    // Create a buffer to store the response.
    let mut buffer = [0u8; 1024];
    let mut response = String::new();
    let start = Instant::now();

    // Read from the port until the response terminator is found or the timeout is reached.
    while start.elapsed() < timeout {
        if let Ok(n) = port.read(&mut buffer) {
            if n > 0 {
                response.push_str(&String::from_utf8_lossy(&buffer[..n]));
                if response.contains(response_terminator) {
                    break;
                }
            }
        }
        // Sleep for a short duration to avoid busy-waiting.
        std::thread::sleep(Duration::from_millis(10));
    }

    trace!(
        "Received response from {}: '{}'",
        instrument_id,
        response.escape_default()
    );

    // Return the trimmed response.
    Ok(response.trim().to_string())
}