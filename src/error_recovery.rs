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
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// The maximum number of retry attempts.
    pub max_attempts: u32,
    /// The delay between retry attempts.
    pub backoff_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_delay: Duration::from_millis(100),
        }
    }
}

/// An asynchronous operation that can be retried.
#[async_trait]
pub trait Recoverable<E> {
    async fn recover(&mut self) -> Result<(), E>;
}

/// An object that can be restarted.
#[async_trait]
pub trait Restartable<E> {
    async fn restart(&mut self) -> Result<(), E>;
}

/// An object that can be reset.
#[async_trait]
pub trait Resettable<E> {
    async fn reset(&mut self) -> Result<(), E>;
}

/// Handles a recoverable error by retrying the operation according to a policy.
pub async fn handle_recoverable_error<T: Recoverable<DaqError>>(
    recoverable: &mut T,
    policy: &RetryPolicy,
) -> Result<(), DaqError> {
    for attempt in 0..policy.max_attempts {
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
pub async fn handle_buffer_overflow<T: Restartable<DaqError>>(
    restartable: &mut T,
) -> Result<(), DaqError> {
    restartable.restart().await
}

/// Handles a checksum error by resetting the device.
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
