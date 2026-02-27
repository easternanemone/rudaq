//! Cross-platform time types.
//!
//! On native: re-exports from `std::time`.
//! On WASM: uses `web_time::Instant` which delegates to `performance.now()`.
//!
//! All GUI code should `use crate::time::Instant` instead of `std::time::Instant`.

pub use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::Instant;

#[cfg(target_arch = "wasm32")]
pub use web_time::Instant;
