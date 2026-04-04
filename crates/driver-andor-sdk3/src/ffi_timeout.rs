//! Timeout-protected wrappers for `spawn_blocking` FFI calls.
//!
//! All Andor SDK3 FFI calls run on `spawn_blocking` to avoid blocking the Tokio
//! runtime. This module adds timeout protection so that a hung SDK call cannot
//! stall the entire daemon indefinitely.
//!
//! Timeout categories are calibrated to real hardware behavior:
//! - **Query** (5s): parameter reads, feature checks, temperature reads
//! - **Config** (15s): set exposure, modes, gain, shutter, general config
//! - **Acquisition** (30s): start/stop acquisition, buffer queue/flush
//! - **Motion** (60s): grating moves, wavelength changes, slit motors
//! - **Init** (120s): SDK library init, device open
//!
//! `AT_WaitBuffer` is intentionally NOT wrapped here -- it has its own
//! SDK-level timeout parameter managed by the acquisition loop.

use std::time::Duration;

use common::error::DaqError;

// ── Timeout constants ────────────────────────────────────────────────────

/// Parameter reads, feature queries, temperature reads.
pub(crate) const FFI_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Set exposure, modes, gain, shutter, general configuration.
pub(crate) const FFI_CONFIG_TIMEOUT: Duration = Duration::from_secs(15);

/// Start/stop acquisition, buffer queue/flush.
pub(crate) const FFI_ACQ_TIMEOUT: Duration = Duration::from_secs(30);

/// Grating moves, wavelength changes, slit motors.
pub(crate) const FFI_MOTION_TIMEOUT: Duration = Duration::from_secs(60);

/// SDK library init, device open.
pub(crate) const FFI_INIT_TIMEOUT: Duration = Duration::from_secs(120);

// ── Wrapper returning anyhow::Result ─────────────────────────────────────

/// Run an FFI closure on `spawn_blocking` with a timeout.
///
/// Returns `anyhow::Result<R>` -- suitable for public API methods that use
/// `anyhow` error propagation.
///
/// # Errors
///
/// - Timeout elapsed: returns a descriptive error including `label`.
/// - `spawn_blocking` join error (task panic): propagated.
/// - Inner closure error: propagated as-is.
pub(crate) async fn ffi_call<F, R>(f: F, timeout: Duration, label: &str) -> anyhow::Result<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(join_err)) => Err(anyhow::anyhow!(
            "FFI spawn_blocking join error in {label}: {join_err}"
        )),
        Err(_elapsed) => Err(anyhow::anyhow!(
            "FFI timeout after {timeout:?} in {label} -- SDK call may be hung"
        )),
    }
}

// ── Wrapper returning Result<R, DaqError> ────────────────────────────────

/// Run an FFI closure on `spawn_blocking` with a timeout, returning `DaqError`.
///
/// Designed for `Parameter<T>` hardware-write callbacks where the return type
/// must be `Result<T, DaqError>`.
pub(crate) async fn ffi_call_daq<F, R>(f: F, timeout: Duration, label: &str) -> Result<R, DaqError>
where
    F: FnOnce() -> anyhow::Result<R> + Send + 'static,
    R: Send + 'static,
{
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(Ok(result))) => Ok(result),
        Ok(Ok(Err(inner_err))) => Err(DaqError::Instrument(inner_err.to_string())),
        Ok(Err(join_err)) => Err(DaqError::Instrument(format!(
            "FFI spawn_blocking join error in {label}: {join_err}"
        ))),
        Err(_elapsed) => Err(DaqError::Instrument(format!(
            "FFI timeout after {timeout:?} in {label} -- SDK call may be hung"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ffi_call_returns_value() {
        let result = ffi_call(|| 42, Duration::from_secs(1), "test_value").await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn ffi_call_timeout_fires() {
        let result = ffi_call(
            || {
                std::thread::sleep(Duration::from_secs(10));
                42
            },
            Duration::from_millis(50),
            "test_timeout",
        )
        .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("FFI timeout"), "got: {err}");
        assert!(err.contains("test_timeout"), "got: {err}");
    }

    #[tokio::test]
    async fn ffi_call_daq_returns_ok() {
        let result = ffi_call_daq(|| Ok(7), Duration::from_secs(1), "test_daq_ok").await;
        assert_eq!(result.unwrap(), 7);
    }

    #[tokio::test]
    async fn ffi_call_daq_propagates_inner_error() {
        let result: Result<i32, DaqError> = ffi_call_daq(
            || Err(anyhow::anyhow!("sensor fault")),
            Duration::from_secs(1),
            "test_daq_err",
        )
        .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sensor fault"), "got: {err}");
    }

    #[tokio::test]
    async fn ffi_call_daq_timeout_fires() {
        let result: Result<i32, DaqError> = ffi_call_daq(
            || {
                std::thread::sleep(Duration::from_secs(10));
                Ok(0)
            },
            Duration::from_millis(50),
            "test_daq_timeout",
        )
        .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("FFI timeout"), "got: {err}");
    }
}
