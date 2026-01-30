//! Seeded RNG wrapper for reproducible behavior.
//!
//! Provides a thread-safe, seeded random number generator for mock devices
//! to enable deterministic failure scenarios in tests.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::sync::Mutex;

/// Seeded RNG wrapper for reproducible random behavior
pub struct MockRng {
    inner: Mutex<ChaCha8Rng>,
}

impl MockRng {
    /// Create a new RNG with optional seed.
    /// If seed is None, uses a random seed from the OS.
    pub fn new(seed: Option<u64>) -> Self {
        let rng = match seed {
            Some(s) => ChaCha8Rng::seed_from_u64(s),
            None => ChaCha8Rng::from_entropy(),
        };
        Self {
            inner: Mutex::new(rng),
        }
    }

    /// Check if an operation should fail based on the given failure rate.
    ///
    /// # Arguments
    /// * `rate` - Failure probability from 0.0 (never fail) to 1.0 (always fail)
    ///
    /// # Returns
    /// true if the operation should fail, false otherwise
    pub fn should_fail(&self, rate: f64) -> bool {
        if rate <= 0.0 {
            return false;
        }
        if rate >= 1.0 {
            return true;
        }
        let mut rng = self.inner.lock().unwrap();
        rng.r#gen::<f64>() < rate
    }

    /// Generate a random u64 value
    pub fn next_u64(&self) -> u64 {
        let mut rng = self.inner.lock().unwrap();
        rng.r#gen()
    }

    /// Generate a random f64 value in the range [0.0, 1.0)
    pub fn next_f64(&self) -> f64 {
        let mut rng = self.inner.lock().unwrap();
        rng.r#gen()
    }

    /// Generate a random value in the given range
    pub fn gen_range<T, R>(&self, range: R) -> T
    where
        T: rand::distributions::uniform::SampleUniform,
        R: rand::distributions::uniform::SampleRange<T>,
    {
        let mut rng = self.inner.lock().unwrap();
        rng.gen_range(range)
    }
}

impl Default for MockRng {
    fn default() -> Self {
        Self::new(None)
    }
}

impl std::fmt::Debug for MockRng {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockRng")
            .field("inner", &"<Mutex<ChaCha8Rng>>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seeded_rng_deterministic() {
        let rng1 = MockRng::new(Some(42));
        let rng2 = MockRng::new(Some(42));

        let val1 = rng1.next_u64();
        let val2 = rng2.next_u64();

        assert_eq!(val1, val2, "Same seed should produce same values");
    }

    #[test]
    fn test_should_fail_never() {
        let rng = MockRng::new(Some(42));
        for _ in 0..100 {
            assert!(!rng.should_fail(0.0), "Rate 0.0 should never fail");
        }
    }

    #[test]
    fn test_should_fail_always() {
        let rng = MockRng::new(Some(42));
        for _ in 0..100 {
            assert!(rng.should_fail(1.0), "Rate 1.0 should always fail");
        }
    }

    #[test]
    fn test_should_fail_probability() {
        let rng = MockRng::new(Some(42));
        let rate = 0.3;
        let samples = 10000;
        let failures = (0..samples).filter(|_| rng.should_fail(rate)).count();

        // With 10000 samples at 30% rate, expect roughly 3000 failures
        // Allow 10% deviation (2700-3300)
        let expected = (rate * samples as f64) as usize;
        let tolerance = (expected as f64 * 0.1) as usize;
        assert!(
            failures > expected - tolerance && failures < expected + tolerance,
            "Expected ~{} failures, got {}",
            expected,
            failures
        );
    }

    #[test]
    fn test_next_f64_range() {
        let rng = MockRng::new(Some(42));
        for _ in 0..100 {
            let val = rng.next_f64();
            assert!(val >= 0.0 && val < 1.0, "f64 should be in [0.0, 1.0)");
        }
    }

    #[test]
    fn test_gen_range() {
        let rng = MockRng::new(Some(42));
        for _ in 0..100 {
            let val = rng.gen_range(10..20);
            assert!(val >= 10 && val < 20, "Value should be in range [10, 20)");
        }
    }

    #[test]
    fn test_default_rng() {
        let rng = MockRng::default();
        // Just verify it works - can't test randomness without seed
        let _val = rng.next_u64();
    }
}
