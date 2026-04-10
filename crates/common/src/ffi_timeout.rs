//! Timeout-guarded wrappers for FFI `spawn_blocking` calls.
//!
//! Native SDK drivers (PVCAM, Andor SDK3, etc.) make synchronous C calls that
//! can hang indefinitely when USB stalls or firmware locks up. These helpers
//! wrap `tokio::task::spawn_blocking` with `tokio::time::timeout` so a hung
//! call is detected and surfaced as an error rather than silently blocking a
//! Tokio worker thread.
//!
//! `tokio::time::timeout` only bounds how long the *caller* waits — it does
//! not cancel the underlying blocking closure. Timeout errors here mean the
//! daemon can recover control, not that the SDK work itself stopped.
//!
//! # Variants
//!
//! | Function | Closure returns | Wrapper returns | Use case |
//! |----------|----------------|-----------------|----------|
//! | [`ffi_with_timeout`] | `anyhow::Result<R>` | `anyhow::Result<R>` | General SDK calls |
//! | [`ffi_with_timeout_daq`] | `Result<R, DaqError>` | `Result<R, DaqError>` | `Parameter` callbacks |
//! | [`ffi_with_timeout_infallible`] | `R` | `anyhow::Result<R>` | SDK calls that can't fail internally |
//! | [`ffi_with_timeout_anyhow_to_daq`] | `anyhow::Result<R>` | `Result<R, DaqError>` | SDK calls mapped to DaqError |
//!
//! Timeout *constants* (query, config, acquisition, init tiers) are defined
//! per-driver since acceptable latencies differ by SDK.

use std::time::Duration;

use crate::error::DaqError;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Typed timeout signal for FFI calls.
#[derive(Debug)]
pub struct FfiTimeoutError {
    /// Human-readable label for the operation that timed out.
    pub label: String,
    /// How long we waited before giving up.
    pub timeout: Duration,
}

impl FfiTimeoutError {
    /// Create a new timeout error.
    pub fn new(label: &str, timeout: Duration) -> Self {
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
pub fn is_timeout_error(error: &anyhow::Error) -> bool {
    error.is::<FfiTimeoutError>()
}

// ---------------------------------------------------------------------------
// ffi_with_timeout — closure returns anyhow::Result<R>
// ---------------------------------------------------------------------------

/// Run a blocking FFI closure with a timeout. Closure returns `anyhow::Result<R>`.
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
// ffi_with_timeout_daq — closure returns Result<R, DaqError>
// ---------------------------------------------------------------------------

/// Run a blocking FFI closure with a timeout, returning `DaqError`.
///
/// Designed for `Parameter<T>` hardware-write callbacks where the return
/// type must be `Result<(), DaqError>`.
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
// ffi_with_timeout_infallible — closure returns R (no Result)
// ---------------------------------------------------------------------------

/// Run a blocking FFI closure with a timeout. Closure returns `R` directly
/// (infallible). Used for SDK calls that return values without error codes.
pub async fn ffi_with_timeout_infallible<F, R>(
    label: &str,
    timeout: Duration,
    f: F,
) -> anyhow::Result<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(join_err)) => Err(anyhow::anyhow!(
            "FFI spawn_blocking join error in {label}: {join_err}"
        )),
        Err(_elapsed) => Err(anyhow::Error::new(FfiTimeoutError::new(label, timeout))),
    }
}

// ---------------------------------------------------------------------------
// ffi_with_timeout_anyhow_to_daq — closure returns anyhow::Result<R>,
//                                   mapped to DaqError
// ---------------------------------------------------------------------------

/// Run a blocking FFI closure with a timeout. Closure returns
/// `anyhow::Result<R>`, mapped to `Result<R, DaqError>` for `Parameter`
/// callbacks that need `DaqError`.
pub async fn ffi_with_timeout_anyhow_to_daq<F, R>(
    label: &str,
    timeout: Duration,
    f: F,
) -> Result<R, DaqError>
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
        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn infallible_returns_value() {
        let result = ffi_with_timeout_infallible("test", Duration::from_secs(1), || 42).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn infallible_timeout_fires() {
        let result = ffi_with_timeout_infallible("slow", Duration::from_millis(1), || {
            std::thread::sleep(Duration::from_millis(20));
            42
        })
        .await;
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn anyhow_to_daq_returns_ok() {
        let result = ffi_with_timeout_anyhow_to_daq("test", Duration::from_secs(1), || Ok(7)).await;
        assert_eq!(result.unwrap(), 7);
    }

    #[tokio::test]
    async fn anyhow_to_daq_propagates_inner_error() {
        let result: Result<i32, DaqError> =
            ffi_with_timeout_anyhow_to_daq("test", Duration::from_secs(1), || {
                Err(anyhow::anyhow!("sensor fault"))
            })
            .await;
        assert!(result.unwrap_err().to_string().contains("sensor fault"));
    }
}
