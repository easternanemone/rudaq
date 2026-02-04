//! NI-DAQmx Driver using PyO3 bridge
//!
//! This driver provides a Rust interface to National Instruments DAQ hardware
//! by bridging to the Python `nidaqmx` package via PyO3. This approach avoids
//! the complexity of directly interfacing with the massive NI-DAQmx C API while
//! leveraging a battle-tested Python wrapper.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────┐
//! │   Rust Application                  │
//! │   (uses Triggerable trait)          │
//! └─────────────────┬───────────────────┘
//!                   │
//! ┌─────────────────▼───────────────────┐
//! │   NiDaqTrigger (Rust)               │
//! │   - Implements Triggerable          │
//! │   - Holds Python GIL references     │
//! └─────────────────┬───────────────────┘
//!                   │ PyO3
//! ┌─────────────────▼───────────────────┐
//! │   nidaqmx (Python package)          │
//! │   - Task management                 │
//! │   - Channel configuration           │
//! └─────────────────┬───────────────────┘
//!                   │ ctypes/CFFI
//! ┌─────────────────▼───────────────────┐
//! │   NI-DAQmx C API                    │
//! │   - Hardware drivers                │
//! └─────────────────────────────────────┘
//! ```
//!
//! # Features
//!
//! - **Mock Mode (default)**: Works without hardware for development/testing
//! - **Hardware Mode**: Requires Python 3.x with `nidaqmx` package installed
//!
//! # Usage
//!
//! ```rust,ignore
//! use driver_nidaqmx::{NiDaqTrigger, TriggerMode};
//!
//! // Digital pulse (simple software trigger)
//! let trigger = NiDaqTrigger::new(
//!     TriggerMode::Digital,
//!     0.1,   // high_time (seconds)
//!     0.001, // low_time (seconds)
//!     1,     // samps_per_chan
//! ).await?;
//!
//! // Trigger On Position (external digital edge trigger)
//! let trigger = NiDaqTrigger::new(
//!     TriggerMode::TriggerOnPosition {
//!         trigger_source: "/Dev1/PFI0".to_string(),
//!         rising_edge: true,
//!         retriggerable: false,
//!     },
//!     0.001, // high_time
//!     0.001, // low_time
//!     1,     // samps_per_chan
//! ).await?;
//!
//! // Use with Triggerable trait
//! trigger.arm().await?;
//! trigger.trigger().await?;
//! ```

pub mod error;
pub mod factory;
pub mod trigger;

pub use error::{NiDaqError, Result};
pub use factory::NiDaqTriggerFactory;
pub use trigger::{NiDaqTrigger, TriggerMode};

// Re-export common traits for convenience
pub use common::capabilities::Triggerable;
