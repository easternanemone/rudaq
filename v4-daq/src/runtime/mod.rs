//! Dual Runtime Management for V2/V4 Coexistence
//!
//! This module provides infrastructure for running V2 (tokio-based) and V4 (Kameo-based)
//! actors simultaneously within the same application.
//!
//! # Overview
//!
//! The `DualRuntimeManager` coordinates:
//! - **V2 Subsystem**: Traditional tokio-based actor model (legacy)
//! - **V4 Subsystem**: Kameo fault-tolerant actor model (new)
//!
//! Both subsystems operate independently with safe shutdown coordination and timeout protection.
//!
//! # Lifecycle
//!
//! ```text
//! start() -> V2 Runtime -> V4 Runtime -> Running
//!
//! shutdown() -> V4 Shutdown -> V2 Shutdown -> Stopped
//! ```
//!
//! # Example
//!
//! ```no_run
//! use v4_daq::runtime::DualRuntimeManager;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut manager = DualRuntimeManager::new();
//!     manager.start().await?;
//!
//!     // System runs...
//!     tokio::time::sleep(Duration::from_secs(10)).await;
//!
//!     // Graceful shutdown
//!     manager.shutdown(Duration::from_secs(30)).await?;
//!     Ok(())
//! }
//! ```

pub mod dual_runtime_manager;

pub use dual_runtime_manager::{DualRuntimeManager, ManagerState};
