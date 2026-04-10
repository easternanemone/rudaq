//! Timeout-guarded wrappers for PVCAM FFI calls.
//!
//! Re-exports the shared FFI timeout utilities from `common::ffi_timeout`
//! with PVCAM-specific timeout constants calibrated to real hardware behavior.

use std::time::Duration;

// Re-export shared utilities
#[cfg(test)]
pub use common::ffi_timeout::FfiTimeoutError;
pub use common::ffi_timeout::{ffi_with_timeout, ffi_with_timeout_daq, is_timeout_error};

// ---------------------------------------------------------------------------
// PVCAM-specific timeout constants
// ---------------------------------------------------------------------------

/// Timeout for parameter query/get operations (quick SDK reads).
pub const PARAM_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Timeout for parameter set / configuration operations.
pub const CONFIG_TIMEOUT: Duration = Duration::from_secs(15);

/// Timeout for acquisition start/stop and heavy setup operations.
#[expect(
    dead_code,
    reason = "timeout tier defined for completeness; used when pvcam_sdk feature is enabled"
)]
pub const ACQUISITION_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for SDK init, device open, and reinitialization.
pub const INIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Timeout for single-frame readout operations.
#[expect(
    dead_code,
    reason = "timeout tier defined for completeness; used when pvcam_sdk feature is enabled"
)]
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for serial-port / USB open operations.
#[expect(
    dead_code,
    reason = "timeout tier defined for completeness; used when pvcam_sdk feature is enabled"
)]
pub const SERIAL_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

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
        let result: Result<(), common::error::DaqError> =
            ffi_with_timeout_daq("daq_test", Duration::from_millis(5), || {
                std::thread::sleep(Duration::from_millis(20));
                Ok(())
            })
            .await;
        let err = result.unwrap_err();
        match &err {
            common::error::DaqError::Instrument(msg) => {
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
        let result: Result<(), common::error::DaqError> =
            ffi_with_timeout_daq("pass", Duration::from_secs(5), || {
                Err(common::error::DaqError::Instrument(
                    "sdk failure".to_string(),
                ))
            })
            .await;
        let err = result.unwrap_err();
        match &err {
            common::error::DaqError::Instrument(msg) => {
                assert!(
                    msg.contains("sdk failure"),
                    "expected original error, got: {msg}"
                );
            }
            other => panic!("expected DaqError::Instrument, got: {other:?}"),
        }
    }
}
