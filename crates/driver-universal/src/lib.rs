//! Universal config-driven driver for rust-daq.
//!
//! This crate implements a "parse-don't-validate" pipeline that transforms TOML
//! device manifests into validated, internally-consistent driver configurations:
//!
//! ```text
//! TOML file -> RawManifest (serde) -> DeviceManifest (validated IR) -> DeviceComponents (runtime)
//!               Stage 1: deser          Stage 2: parse + cross-validate    Stage 3: build (Phase 2)
//! ```
//!
//! # Schema Version
//!
//! This crate expects `schema_version = 3` in all device manifests.

pub mod config;
pub mod format_parser;
pub mod response;
pub mod template;
pub mod transform;
pub mod transport;
