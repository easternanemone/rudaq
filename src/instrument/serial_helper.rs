//! Temporary serial helper for V1 instruments
//!
//! This module provides serial communication helpers for legacy V1 instruments
//! during the migration to V2 architecture. Will be removed when all instruments
//! are migrated to use SerialAdapter from `src/adapters/serial_adapter.rs`.

#[cfg(feature = "instrument_serial")]
use crate::adapters::{serial::SerialAdapter, Adapter};
#[cfg(feature = "instrument_serial")]
use anyhow::{anyhow, Context, Result};
#[cfg(feature = "instrument_serial")]
use log::debug;
#[cfg(feature = "instrument_serial")]
use std::time::{Duration, Instant};

/// Send a command via the shared [`SerialAdapter`] and wait for a delimited response.
#[cfg(feature = "instrument_serial")]
pub async fn send_command_async(
    mut adapter: SerialAdapter,
    instrument_id: &str,
    command: &str,
    terminator: &str,
    timeout: Duration,
    delimiter: u8,
) -> Result<String> {
    let command_with_term = format!("{}{}", command, terminator);

    adapter
        .write(command_with_term.clone().into_bytes())
        .await
        .with_context(|| format!("Failed to write command to {} serial port", instrument_id))?;

    debug!("[{}] Sent command: {}", instrument_id, command.trim());

    let start = Instant::now();
    let mut response: Vec<u8> = Vec::new();

    loop {
        if start.elapsed() > timeout {
            return Err(anyhow!(
                "[{}] Serial read timeout after {:?}",
                instrument_id,
                timeout
            ));
        }

        let mut buffer = Vec::new();
        let bytes_read = adapter
            .read(&mut buffer)
            .await
            .with_context(|| format!("Failed to read response from {}", instrument_id))?;

        if bytes_read == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            continue;
        }

        response.extend_from_slice(&buffer[..bytes_read]);

        if buffer[..bytes_read].contains(&delimiter) {
            break;
        }
    }

    let response = String::from_utf8_lossy(&response).trim().to_string();
    debug!("[{}] Received response: {}", instrument_id, response);
    Ok(response)
}

#[cfg(not(feature = "instrument_serial"))]
pub async fn send_command_async<A>(
    _adapter: A,
    _instrument_id: &str,
    _command: &str,
    _terminator: &str,
    _timeout: std::time::Duration,
    _delimiter: u8,
) -> anyhow::Result<String> {
    Err(anyhow::anyhow!(
        "Serial support not enabled. Rebuild with --features instrument_serial"
    ))
}
