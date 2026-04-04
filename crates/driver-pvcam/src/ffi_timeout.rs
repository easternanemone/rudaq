//! Timeout-guarded wrappers for PVCAM FFI `spawn_blocking` calls.
//!
//! PVCAM SDK calls are synchronous C functions that can hang indefinitely when
//! the USB bus stalls or the camera firmware locks up. These helpers wrap
//! `tokio::task::spawn_blocking` with `tokio::time::timeout` so that a hung
//! FFI call is detected and surfaced as an error rather than silently blocking
//! a Tokio worker thread forever.
//!
//! `tokio::time::timeout` only bounds how long the caller waits. It does not
//! stop the blocking FFI closure if the SDK is already wedged, so the timeout
//! path must be treated as a notification signal rather than an actual cancel.
//!
//! Two variants are provided:
//! - [`ffi_with_timeout`] returns `anyhow::Result<R>` for general use
//!   (e.g., SDK init, reinitialize).
//! - [`ffi_with_timeout_daq`] returns `Result<R, DaqError>` for use inside
//!   `Parameter` hardware-write callbacks, which must return `DaqError`.

use std::time::Duration;

use common::error::DaqError;

// ---------------------------------------------------------------------------
// Timeout constants
// ---------------------------------------------------------------------------

/// Timeout for parameter query/get operations (quick SDK reads).
pub const PARAM_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout for parameter set / configuration operations.
pub const CONFIG_TIMEOUT: Duration = Duration::from_secs(15);

/// Timeout for acquisition start/stop and heavy setup operations.
#[allow(dead_code)]
pub const ACQUISITION_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for SDK init, device open, and reinitialization.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Timeout for single-frame readout operations.
#[allow(dead_code)]
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for serial-port / USB open operations.
#[allow(dead_code)]
pub const SERIAL_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Generic (anyhow) variant
// ---------------------------------------------------------------------------

/// Typed timeout signal returned by this module.
#[derive(Debug)]
pub(crate) struct FfiTimeoutError {
    label: String,
    timeout: Duration,
}

impl FfiTimeoutError {
    fn new(label: &str, timeout: Duration) -> Self {
        Self {
            label: label.to_owned(),
            timeout,
        }
    }
}

impl std::fmt::Display for FfiTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FFI call timed out after {:?} ({}) -- SDK call may be hung",
            self.timeout, self.label
        )
    }
}

impl std::error::Error for FfiTimeoutError {}

/// Return true when the error came from a timed-out FFI call.
pub(crate) fn is_timeout_error(error: &anyhow::Error) -> bool {
    error.is::<FfiTimeoutError>()
}

/// Run a blocking FFI closure on the Tokio blocking pool with a timeout.
///
/// Returns `anyhow::Error` on timeout or if the blocking task panics/joins
/// with an error.
pub async fn ffi_with_timeout<F, R>(label: &str, timeout: Duration, f: F) -> anyhow::Result<R>
where
    F: FnOnce() -> anyhow::Result<R> + Send + 'static,
    R: Send + 'static,
{
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(result)) => result,
        Ok(Err(join_err)) => Err(anyhow::anyhow!("FFI task panicked ({label}): {join_err}")),
        Err(_elapsed) => Err(anyhow::Error::new(FfiTimeoutError::new(label, timeout))),
    }
}

// ---------------------------------------------------------------------------
// DaqError variant (for Parameter hardware-write callbacks)
// ---------------------------------------------------------------------------

/// Run a blocking FFI closure on the Tokio blocking pool with a timeout,
/// mapping errors to [`DaqError`].
///
/// This is the variant used inside `Parameter::connect_to_hardware_write`
/// callbacks, which must return `Result<(), DaqError>`.
pub async fn ffi_with_timeout_daq<F, R>(label: &str, timeout: Duration, f: F) -> Result<R, DaqError>
where
    F: FnOnce() -> Result<R, DaqError> + Send + 'static,
    R: Send + 'static,
{
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(result)) => result,
        Ok(Err(join_err)) => Err(DaqError::Instrument(format!(
            "FFI task panicked ({label}): {join_err}"
        ))),
        Err(_elapsed) => Err(DaqError::Instrument(
            FfiTimeoutError::new(label, timeout).to_string(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fast_success_returns_value() {
        let result = ffi_with_timeout("test", Duration::from_secs(5), || Ok(42)).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn timeout_triggers_error() {
        let result = ffi_with_timeout("slow_op", Duration::from_millis(5), || {
            std::thread::sleep(Duration::from_millis(20));
            Ok(())
        })
        .await;
        let err = result.unwrap_err();
        assert!(
            is_timeout_error(&err),
            "expected typed timeout error, got: {err}"
        );
        assert_eq!(
            err.downcast_ref::<FfiTimeoutError>()
                .map(|timeout| timeout.label.as_str()),
            Some("slow_op")
        );
    }

    #[tokio::test]
    async fn daq_error_mapping() {
        let result: Result<(), DaqError> =
            ffi_with_timeout_daq("daq_test", Duration::from_millis(5), || {
                std::thread::sleep(Duration::from_millis(20));
                Ok(())
            })
            .await;
        let err = result.unwrap_err();
        match &err {
            DaqError::Instrument(msg) => {
                assert!(
                    msg.contains("timed out"),
                    "expected timeout in message, got: {msg}"
                );
                assert!(
                    msg.contains("daq_test"),
                    "expected label in message, got: {msg}"
                );
            }
            other => panic!("expected DaqError::Instrument, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn daq_error_passthrough() {
        let result: Result<(), DaqError> =
            ffi_with_timeout_daq("pass", Duration::from_secs(5), || {
                Err(DaqError::Instrument("sdk failure".to_string()))
            })
            .await;
        let err = result.unwrap_err();
        match &err {
            DaqError::Instrument(msg) => {
                assert!(
                    msg.contains("sdk failure"),
                    "expected original error, got: {msg}"
                );
            }
            other => panic!("expected DaqError::Instrument, got: {other:?}"),
        }
    }
}
