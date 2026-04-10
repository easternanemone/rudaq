//! Timeout-protected wrappers for Andor SDK3 FFI calls.
//!
//! Re-exports the shared FFI timeout utilities from `common::ffi_timeout`
//! with Andor-specific timeout constants and convenience aliases matching
//! the existing call-site conventions.

use std::time::Duration;

// Re-export shared utilities with Andor-style aliases
pub(crate) use common::ffi_timeout::ffi_with_timeout_anyhow_to_daq as ffi_call_daq;
pub(crate) use common::ffi_timeout::ffi_with_timeout_infallible;

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

// ── Andor-style wrapper ─────────────────────────────────────────────────

/// Run an FFI closure on `spawn_blocking` with a timeout.
///
/// Matches the original Andor calling convention: `ffi_call(closure, timeout, label)`
/// with closure returning `R` directly (infallible).
pub(crate) async fn ffi_call<F, R>(f: F, timeout: Duration, label: &str) -> anyhow::Result<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    ffi_with_timeout_infallible(label, timeout, f).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::error::DaqError;

    #[tokio::test]
    async fn ffi_call_returns_value() {
        let result = ffi_call(|| 42, Duration::from_secs(1), "test_value").await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn ffi_call_timeout_fires() {
        let result = ffi_call(
            || {
                std::thread::sleep(Duration::from_millis(20));
                42
            },
            Duration::from_millis(1),
            "test_timeout",
        )
        .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timed out"), "got: {err}");
        assert!(err.contains("test_timeout"), "got: {err}");
    }

    #[tokio::test]
    async fn ffi_call_daq_returns_ok() {
        let result = ffi_call_daq("test_daq_ok", Duration::from_secs(1), || Ok(7)).await;
        assert_eq!(result.unwrap(), 7);
    }

    #[tokio::test]
    async fn ffi_call_daq_propagates_inner_error() {
        let result: Result<i32, DaqError> =
            ffi_call_daq("test_daq_err", Duration::from_secs(1), || {
                Err(anyhow::anyhow!("sensor fault"))
            })
            .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sensor fault"), "got: {err}");
    }

    #[tokio::test]
    async fn ffi_call_daq_timeout_fires() {
        let result: Result<i32, DaqError> =
            ffi_call_daq("test_daq_timeout", Duration::from_millis(1), || {
                std::thread::sleep(Duration::from_millis(20));
                Ok(0)
            })
            .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timed out"), "got: {err}");
    }
}
