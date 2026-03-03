//! SurrealDB E2E Validation with Mock Hardware (bd-p33c)
//!
//! Validates the full SurrealDB persistence layer with a realistic 9-device,
//! 5-driver-type mock hardware profile (mock_maitai_lab).
//!
//! Covers:
//! - Daemon startup path (shadow write, factory registration)
//! - Config hash convergence (initial reconcile reports unchanged)
//! - gRPC ConfigService CRUD with multi-device lab
//! - Watch reconciler hot-swap (add/remove/modify via gRPC)
//! - Error resilience (DB init failure, clean shutdown)
//! - Concurrent operations and MeasurementLock safety
//!
//! Run with: cargo nextest run -p integration-tests --features db-surreal-mem --test surrealdb_e2e
#![cfg(any(feature = "db-surreal-mem", feature = "db-surreal-rocksdb"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    unused_imports,
    dead_code,
    missing_docs
)]

pub mod helpers;

#[cfg(feature = "db-surreal-mem")]
mod startup_tests;

#[cfg(all(feature = "server", feature = "db-surreal-mem"))]
mod watch_reconciler_tests;

#[cfg(feature = "db-surreal-mem")]
mod resilience_tests;

#[cfg(feature = "db-surreal-mem")]
mod safety_tests;

#[cfg(feature = "db-surreal-rocksdb")]
mod rocksdb_tests;
