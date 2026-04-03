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

/// Backoff strategy for retry policies.
///
/// Determines how the delay between retry attempts is calculated.
/// Use [`BackoffStrategy::Constant`] for fixed delays, or
/// [`BackoffStrategy::Exponential`] for delays that grow each attempt.
///
/// # Example
///
/// ```rust
/// use common::error_recovery::{BackoffStrategy, ExponentialBackoff};
/// use std::time::Duration;
///
/// // Constant 500ms between each retry
/// let constant = BackoffStrategy::Constant;
///
/// // Exponential: 100ms, 200ms, 400ms, ... capped at 5s
/// let exponential = BackoffStrategy::Exponential(ExponentialBackoff {
///     initial_delay: Duration::from_millis(100),
///     max_delay: Duration::from_secs(5),
///     multiplier: 2.0,
///     jitter: false,
/// });
/// ```
#[derive(Clone, Debug)]
pub enum BackoffStrategy {
    /// Fixed delay between each retry attempt.
    ///
    /// Uses the `backoff_delay` field from [`RetryPolicy`] for every attempt.
    Constant,

    /// Exponentially increasing delay between retry attempts.
    ///
    /// Delay grows as `initial_delay * multiplier^attempt`, capped at `max_delay`.
    /// The `backoff_delay` field from [`RetryPolicy`] is ignored when this strategy
    /// is active; the [`ExponentialBackoff`] parameters control timing instead.
    Exponential(ExponentialBackoff),
}

/// Configuration for exponential backoff retry delays.
///
/// Computes the delay for attempt `n` as:
/// `min(initial_delay * multiplier^n, max_delay)`
///
/// When `jitter` is enabled, a random value between 0% and 25% of the computed
/// delay is added to prevent thundering herd effects when multiple clients retry
/// simultaneously.
///
/// # Example
///
/// ```rust
/// use common::error_recovery::ExponentialBackoff;
/// use std::time::Duration;
///
/// let backoff = ExponentialBackoff {
///     initial_delay: Duration::from_millis(100),
///     max_delay: Duration::from_secs(30),
///     multiplier: 2.0,
///     jitter: true,
/// };
///
/// // Attempt 0: ~100ms (+ up to 25ms jitter)
/// // Attempt 1: ~200ms (+ up to 50ms jitter)
/// // Attempt 2: ~400ms (+ up to 100ms jitter)
/// // ...capped at 30s
/// ```
#[derive(Clone, Debug)]
pub struct ExponentialBackoff {
    /// Delay before the first retry attempt.
    pub initial_delay: Duration,

    /// Maximum delay between retry attempts.
    ///
    /// The computed delay will never exceed this value (before jitter).
    pub max_delay: Duration,

    /// Multiplier applied to the delay each attempt.
    ///
    /// Typical values are 2.0 (doubling) or 1.5 (50% growth).
    /// Must be >= 1.0; values less than 1.0 are treated as 1.0.
    pub multiplier: f64,

    /// Whether to add random jitter (0-25% of delay) to prevent thundering herd.
    pub jitter: bool,
}

impl ExponentialBackoff {
    /// Computes the delay for a given attempt number (0-indexed).
    ///
    /// Returns `min(initial_delay * multiplier^attempt, max_delay)`, with
    /// optional random jitter added on top.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    // cast_precision_loss: as_nanos() returns u128, but realistic delay values
    // (up to ~30s = 30e9 nanos) are well within f64's 53-bit mantissa range.
    // cast_possible_truncation + cast_sign_loss: we clamp the f64 multiplication
    // result to max_delay and jitter_nanos is bounded by delay/4, both non-negative
    // and well within u64 range.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let multiplier = self.multiplier.max(1.0);
        let base_nanos = self.initial_delay.as_nanos() as f64;
        // attempt is a retry count (realistically < 100), safe to convert to i32
        let scaled = base_nanos * multiplier.powi(attempt.cast_signed());

        let max_nanos = self.max_delay.as_nanos() as f64;
        let clamped_nanos = scaled.min(max_nanos);
        let delay = Duration::from_nanos(clamped_nanos as u64);

        if self.jitter {
            // Add random jitter: 0-25% of the computed delay
            let jitter_max_nanos = clamped_nanos * 0.25;
            let jitter_nanos = fastrand::f64() * jitter_max_nanos;
            delay + Duration::from_nanos(jitter_nanos as u64)
        } else {
            delay
        }
    }
}

impl Default for ExponentialBackoff {
    /// Creates a default exponential backoff configuration.
    ///
    /// Defaults: 100ms initial delay, 30s max delay, 2.0 multiplier, no jitter.
    ///
    /// # Example
    ///
    /// ```rust
    /// use common::error_recovery::ExponentialBackoff;
    /// use std::time::Duration;
    ///
    /// let backoff = ExponentialBackoff::default();
    /// assert_eq!(backoff.initial_delay, Duration::from_millis(100));
    /// assert_eq!(backoff.max_delay, Duration::from_secs(30));
    /// assert!((backoff.multiplier - 2.0).abs() < f64::EPSILON);
    /// assert!(!backoff.jitter);
    /// ```
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            jitter: false,
        }
    }
}

/// Defines a policy for retrying an operation.
///
/// Specifies how many times to retry a failed operation and how long to wait
/// between attempts. Used by error recovery handlers to implement automatic
/// retry logic.
///
/// # Example
///
/// ```rust
/// use common::error_recovery::{RetryPolicy, BackoffStrategy};
/// use std::time::Duration;
///
/// let policy = RetryPolicy {
///     max_attempts: 5,
///     backoff_delay: Duration::from_millis(200),
///     backoff_strategy: BackoffStrategy::Constant,
/// };
/// ```
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// The maximum number of retry attempts.
    ///
    /// Total attempts will be max_attempts (not including the initial try).
    /// Set to 0 to disable retries.
    pub max_attempts: u32,

    /// The delay between retry attempts (used with constant backoff).
    ///
    /// When using [`BackoffStrategy::Constant`] (the default), this delay is
    /// applied between every attempt. When using [`BackoffStrategy::Exponential`],
    /// this field is ignored in favor of the [`ExponentialBackoff`] parameters.
    pub backoff_delay: Duration,

    /// The backoff strategy to use between retry attempts.
    ///
    /// Defaults to [`BackoffStrategy::Constant`], which uses `backoff_delay` for
    /// every attempt. Set to [`BackoffStrategy::Exponential`] for growing delays.
    pub backoff_strategy: BackoffStrategy,
}

impl Default for RetryPolicy {
    /// Creates a default retry policy.
    ///
    /// Default policy attempts 3 retries with 100ms delay between attempts.
    ///
    /// # Example
    ///
    /// ```rust
    /// use common::error_recovery::RetryPolicy;
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
            backoff_strategy: BackoffStrategy::Constant,
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
/// use common::error_recovery::Recoverable;
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
/// use common::error_recovery::Restartable;
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
/// use common::error_recovery::Resettable;
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
/// use common::error_recovery::{handle_recoverable_error, RetryPolicy, BackoffStrategy};
/// use std::time::Duration;
///
/// let mut connection = SerialConnection::new();
/// let policy = RetryPolicy {
///     max_attempts: 5,
///     backoff_delay: Duration::from_millis(500),
///     backoff_strategy: BackoffStrategy::Constant,
/// };
///
/// handle_recoverable_error(&mut connection, &policy).await?;
/// ```
pub async fn handle_recoverable_error<T: Recoverable<DaqError>>(
    recoverable: &mut T,
    policy: &RetryPolicy,
) -> Result<(), DaqError> {
    for attempt in 0..policy.max_attempts {
        if recoverable.recover().await.is_ok() {
            return Ok(());
        }
        let delay = match &policy.backoff_strategy {
            BackoffStrategy::Constant => policy.backoff_delay,
            BackoffStrategy::Exponential(backoff) => backoff.delay_for_attempt(attempt),
        };
        sleep(delay).await;
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
/// use common::error_recovery::handle_buffer_overflow;
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
/// use common::error_recovery::handle_checksum_error;
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
            backoff_strategy: BackoffStrategy::Constant,
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
            backoff_strategy: BackoffStrategy::Constant,
        };
        let result = handle_recoverable_error(&mut recoverable, &policy).await;
        assert!(result.is_err());
        assert_eq!(*recoverable.attempts.borrow(), 3);
    }

    // --- Exponential backoff tests ---

    #[test]
    fn test_exponential_backoff_basic_growth() {
        let backoff = ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            jitter: false,
        };

        // delay = initial_delay * multiplier^attempt
        assert_eq!(backoff.delay_for_attempt(0), Duration::from_millis(100)); // 100 * 2^0
        assert_eq!(backoff.delay_for_attempt(1), Duration::from_millis(200)); // 100 * 2^1
        assert_eq!(backoff.delay_for_attempt(2), Duration::from_millis(400)); // 100 * 2^2
        assert_eq!(backoff.delay_for_attempt(3), Duration::from_millis(800)); // 100 * 2^3
        assert_eq!(backoff.delay_for_attempt(4), Duration::from_millis(1600)); // 100 * 2^4
    }

    #[test]
    fn test_exponential_backoff_max_delay_cap() {
        let backoff = ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(500),
            multiplier: 2.0,
            jitter: false,
        };

        assert_eq!(backoff.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(backoff.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(backoff.delay_for_attempt(2), Duration::from_millis(400));
        // 100 * 2^3 = 800ms, but capped at 500ms
        assert_eq!(backoff.delay_for_attempt(3), Duration::from_millis(500));
        // Higher attempts remain capped
        assert_eq!(backoff.delay_for_attempt(10), Duration::from_millis(500));
    }

    #[test]
    fn test_exponential_backoff_non_doubling_multiplier() {
        let backoff = ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            multiplier: 1.5,
            jitter: false,
        };

        assert_eq!(backoff.delay_for_attempt(0), Duration::from_millis(100)); // 100 * 1.5^0
        assert_eq!(backoff.delay_for_attempt(1), Duration::from_millis(150)); // 100 * 1.5^1
        assert_eq!(backoff.delay_for_attempt(2), Duration::from_millis(225)); // 100 * 1.5^2
    }

    #[test]
    fn test_exponential_backoff_multiplier_below_one_clamped() {
        // Multiplier < 1.0 should be treated as 1.0 (no shrinking)
        let backoff = ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            multiplier: 0.5,
            jitter: false,
        };

        assert_eq!(backoff.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(backoff.delay_for_attempt(1), Duration::from_millis(100));
        assert_eq!(backoff.delay_for_attempt(5), Duration::from_millis(100));
    }

    #[test]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap
    )]
    fn test_exponential_backoff_jitter_within_bounds() {
        let backoff = ExponentialBackoff {
            initial_delay: Duration::from_millis(1000),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            jitter: true,
        };

        // With jitter, delay should be in [base, base * 1.25]
        // Run multiple samples to verify bounds
        for attempt in 0..5u32 {
            // Test values are small enough that these casts are exact.
            let base = backoff.initial_delay.as_nanos() as f64
                * backoff.multiplier.powi(attempt as i32);
            let base_dur = Duration::from_nanos(base as u64);
            let max_with_jitter = base_dur + Duration::from_nanos((base * 0.25) as u64);

            for _ in 0..50 {
                let delay = backoff.delay_for_attempt(attempt);
                assert!(
                    delay >= base_dur,
                    "Delay {delay:?} should be >= base {base_dur:?} for attempt {attempt}"
                );
                assert!(
                    delay <= max_with_jitter + Duration::from_nanos(1),
                    "Delay {delay:?} should be <= {max_with_jitter:?} for attempt {attempt}"
                );
            }
        }
    }

    #[test]
    fn test_exponential_backoff_jitter_produces_variation() {
        let backoff = ExponentialBackoff {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            jitter: true,
        };

        // Collect a set of delays; with jitter on a 1s base, we should see
        // some variation across 20 samples.
        let delays: Vec<Duration> = (0..20).map(|_| backoff.delay_for_attempt(0)).collect();
        let unique_count = {
            let mut unique = delays.clone();
            unique.dedup();
            unique.len()
        };

        // With random jitter on a 1-second base (250ms jitter range),
        // getting all 20 identical is astronomically unlikely.
        assert!(
            unique_count > 1,
            "Jitter should produce varying delays, but got {unique_count} unique values out of 20"
        );
    }

    #[test]
    fn test_exponential_backoff_no_jitter_deterministic() {
        let backoff = ExponentialBackoff {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            jitter: false,
        };

        // Without jitter, delays should be perfectly deterministic
        for attempt in 0..5 {
            let d1 = backoff.delay_for_attempt(attempt);
            let d2 = backoff.delay_for_attempt(attempt);
            assert_eq!(d1, d2, "Without jitter, delay should be deterministic");
        }
    }

    #[test]
    fn test_exponential_backoff_default() {
        let backoff = ExponentialBackoff::default();
        assert_eq!(backoff.initial_delay, Duration::from_millis(100));
        assert_eq!(backoff.max_delay, Duration::from_secs(30));
        assert!((backoff.multiplier - 2.0).abs() < f64::EPSILON);
        assert!(!backoff.jitter);
    }

    #[tokio::test]
    async fn test_handle_recoverable_error_with_exponential_backoff() {
        let mut recoverable = MockRecoverable {
            attempts: RefCell::new(0),
            succeed_on_attempt: 3,
        };
        let policy = RetryPolicy {
            max_attempts: 5,
            backoff_delay: Duration::ZERO, // ignored for exponential
            backoff_strategy: BackoffStrategy::Exponential(ExponentialBackoff {
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(50),
                multiplier: 2.0,
                jitter: false,
            }),
        };

        let result = handle_recoverable_error(&mut recoverable, &policy).await;
        assert!(result.is_ok());
        assert_eq!(*recoverable.attempts.borrow(), 3);
    }

    #[tokio::test]
    async fn test_handle_recoverable_error_exponential_exhausted() {
        let mut recoverable = MockRecoverable {
            attempts: RefCell::new(0),
            succeed_on_attempt: 10, // will never succeed within 3 attempts
        };
        let policy = RetryPolicy {
            max_attempts: 3,
            backoff_delay: Duration::ZERO,
            backoff_strategy: BackoffStrategy::Exponential(ExponentialBackoff {
                initial_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
                multiplier: 2.0,
                jitter: false,
            }),
        };

        let result = handle_recoverable_error(&mut recoverable, &policy).await;
        assert!(result.is_err());
        assert_eq!(*recoverable.attempts.borrow(), 3);
    }

    #[test]
    fn test_retry_policy_default_uses_constant_backoff() {
        let policy = RetryPolicy::default();
        assert!(
            matches!(policy.backoff_strategy, BackoffStrategy::Constant),
            "Default RetryPolicy should use Constant backoff"
        );
    }
}
