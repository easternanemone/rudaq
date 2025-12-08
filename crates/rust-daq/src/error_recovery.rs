//! Automatic error recovery strategies.
//
// This module will contain implementations for automatic error recovery,
// such as reconnecting on serial timeouts, restarting measurements on
// buffer overflows, and resetting on checksum errors. It will also
// include configurable retry policies.

use crate::error::DaqError;
use async_trait::async_trait;
use std::time::Duration;
use tokio::time::sleep;

/// Defines a policy for retrying an operation.
///
/// Specifies how many times to retry a failed operation and how long to wait
/// between attempts. Used by error recovery handlers to implement automatic
/// retry logic.
///
/// # Example
///
/// ```rust
/// use rust_daq::error_recovery::RetryPolicy;
/// use std::time::Duration;
///
/// let policy = RetryPolicy {
///     max_attempts: 5,
///     backoff_delay: Duration::from_millis(200),
/// };
/// ```
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// The maximum number of retry attempts.
    ///
    /// Total attempts will be max_attempts (not including the initial try).
    /// Set to 0 to disable retries.
    pub max_attempts: u32,

    /// The delay between retry attempts.
    ///
    /// Uses a constant backoff strategy. For exponential backoff,
    /// implement custom retry logic.
    pub backoff_delay: Duration,
}

impl Default for RetryPolicy {
    /// Creates a default retry policy.
    ///
    /// Default policy attempts 3 retries with 100ms delay between attempts.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rust_daq::error_recovery::RetryPolicy;
    /// use std::time::Duration;
    ///
    /// let policy = RetryPolicy::default();
    /// assert_eq!(policy.max_attempts, 3);
    /// assert_eq!(policy.backoff_delay, Duration::from_millis(100));
    /// ```
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_delay: Duration::from_millis(100),
        }
    }
}

/// An asynchronous operation that can be retried.
///
/// Implement this trait for operations that can recover from transient failures
/// through retry logic (e.g., reconnecting to serial ports, retrying network requests).
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::error_recovery::Recoverable;
/// use async_trait::async_trait;
///
/// struct SerialConnection {
///     port: Option<SerialPort>,
/// }
///
/// #[async_trait]
/// impl Recoverable<std::io::Error> for SerialConnection {
///     async fn recover(&mut self) -> Result<(), std::io::Error> {
///         // Attempt to reconnect
///         self.port = Some(SerialPort::open("/dev/ttyUSB0")?);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Recoverable<E> {
    /// Attempts to recover from a failure.
    ///
    /// Called by the retry handler to attempt recovery. Should return `Ok(())`
    /// if recovery succeeds, or `Err(E)` if it fails.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if recovery succeeds
    /// * `Err(E)` if recovery fails
    async fn recover(&mut self) -> Result<(), E>;
}

/// An object that can be restarted.
///
/// Implement this trait for operations that need to be completely restarted
/// after a failure (e.g., restarting a measurement after buffer overflow).
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::error_recovery::Restartable;
/// use async_trait::async_trait;
///
/// struct Acquisition {
///     camera: Camera,
/// }
///
/// #[async_trait]
/// impl Restartable<anyhow::Error> for Acquisition {
///     async fn restart(&mut self) -> Result<(), anyhow::Error> {
///         self.camera.stop_acquisition().await?;
///         self.camera.clear_buffer().await?;
///         self.camera.start_acquisition().await?;
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Restartable<E> {
    /// Restarts the operation from a clean state.
    ///
    /// Called to completely restart an operation after a failure.
    /// Should clean up any intermediate state and reinitialize.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if restart succeeds
    /// * `Err(E)` if restart fails
    async fn restart(&mut self) -> Result<(), E>;
}

/// An object that can be reset.
///
/// Implement this trait for hardware that can be reset to recover from
/// error conditions (e.g., resetting a device after checksum errors).
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::error_recovery::Resettable;
/// use async_trait::async_trait;
///
/// struct LaserController {
///     serial: SerialPort,
/// }
///
/// #[async_trait]
/// impl Resettable<std::io::Error> for LaserController {
///     async fn reset(&mut self) -> Result<(), std::io::Error> {
///         self.serial.write_all(b"*RST\r\n").await?;
///         tokio::time::sleep(Duration::from_secs(2)).await;
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Resettable<E> {
    /// Resets the device to a known good state.
    ///
    /// Called to reset hardware after an error condition.
    /// Should send reset commands and wait for the device to reinitialize.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if reset succeeds
    /// * `Err(E)` if reset fails
    async fn reset(&mut self) -> Result<(), E>;
}

/// Handles a recoverable error by retrying the operation according to a policy.
///
/// Attempts recovery up to `max_attempts` times with `backoff_delay` between attempts.
/// Returns `Ok(())` if any attempt succeeds, or an error if all attempts fail.
///
/// # Arguments
///
/// * `recoverable` - Object implementing the `Recoverable` trait
/// * `policy` - Retry policy specifying max attempts and backoff delay
///
/// # Returns
///
/// * `Ok(())` if recovery succeeds within max_attempts
/// * `Err(DaqError)` if all recovery attempts fail
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::error_recovery::{handle_recoverable_error, RetryPolicy};
/// use std::time::Duration;
///
/// let mut connection = SerialConnection::new();
/// let policy = RetryPolicy {
///     max_attempts: 5,
///     backoff_delay: Duration::from_millis(500),
/// };
///
/// handle_recoverable_error(&mut connection, &policy).await?;
/// ```
pub async fn handle_recoverable_error<T: Recoverable<DaqError>>(
    recoverable: &mut T,
    policy: &RetryPolicy,
) -> Result<(), DaqError> {
    for _attempt in 0..policy.max_attempts {
        if recoverable.recover().await.is_ok() {
            return Ok(());
        }
        sleep(policy.backoff_delay).await;
    }
    Err(DaqError::Instrument(format!(
        "Failed to recover after {} attempts.",
        policy.max_attempts
    )))
}

/// Handles a buffer overflow error by restarting the measurement.
///
/// Calls the `restart()` method on the provided object to reinitialize
/// the measurement from a clean state.
///
/// # Arguments
///
/// * `restartable` - Object implementing the `Restartable` trait
///
/// # Returns
///
/// * `Ok(())` if restart succeeds
/// * `Err(DaqError)` if restart fails
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::error_recovery::handle_buffer_overflow;
///
/// let mut acquisition = CameraAcquisition::new(camera);
/// handle_buffer_overflow(&mut acquisition).await?;
/// ```
pub async fn handle_buffer_overflow<T: Restartable<DaqError>>(
    restartable: &mut T,
) -> Result<(), DaqError> {
    restartable.restart().await
}

/// Handles a checksum error by resetting the device.
///
/// Calls the `reset()` method on the provided object to reset the hardware
/// to a known good state.
///
/// # Arguments
///
/// * `resettable` - Object implementing the `Resettable` trait
///
/// # Returns
///
/// * `Ok(())` if reset succeeds
/// * `Err(DaqError)` if reset fails
///
/// # Example
///
/// ```rust,ignore
/// use rust_daq::error_recovery::handle_checksum_error;
///
/// let mut controller = MotionController::new();
/// handle_checksum_error(&mut controller).await?;
/// ```
pub async fn handle_checksum_error<T: Resettable<DaqError>>(
    resettable: &mut T,
) -> Result<(), DaqError> {
    resettable.reset().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct MockRecoverable {
        attempts: RefCell<u32>,
        succeed_on_attempt: u32,
    }

    #[async_trait]
    impl Recoverable<DaqError> for MockRecoverable {
        async fn recover(&mut self) -> Result<(), DaqError> {
            let mut attempts = self.attempts.borrow_mut();
            *attempts += 1;
            if *attempts >= self.succeed_on_attempt {
                Ok(())
            } else {
                Err(DaqError::Instrument("Failed to recover".to_string()))
            }
        }
    }

    #[tokio::test]
    async fn test_retry_logic_succeeds() {
        let mut recoverable = MockRecoverable {
            attempts: RefCell::new(0),
            succeed_on_attempt: 2,
        };
        let policy = RetryPolicy {
            max_attempts: 3,
            backoff_delay: Duration::from_millis(10),
        };
        let result = handle_recoverable_error(&mut recoverable, &policy).await;
        assert!(result.is_ok());
        assert_eq!(*recoverable.attempts.borrow(), 2);
    }

    #[tokio::test]
    async fn test_retry_logic_fails() {
        let mut recoverable = MockRecoverable {
            attempts: RefCell::new(0),
            succeed_on_attempt: 4,
        };
        let policy = RetryPolicy {
            max_attempts: 3,
            backoff_delay: Duration::from_millis(10),
        };
        let result = handle_recoverable_error(&mut recoverable, &policy).await;
        assert!(result.is_err());
        assert_eq!(*recoverable.attempts.borrow(), 3);
    }
}
