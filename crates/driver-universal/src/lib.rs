//! Universal config-driven driver for rust-daq.
//!
//! This crate implements a "parse-don't-validate" pipeline that transforms TOML
//! device manifests into validated, internally-consistent driver configurations:
//!
//! ```text
//! TOML file -> RawManifest (serde) -> DeviceManifest (validated IR) -> DeviceComponents (runtime)
//!               Stage 1: deser          Stage 2: parse + cross-validate    Stage 3: build
//! ```
//!
//! # Schema Version
//!
//! This crate expects `schema_version = 3` in all device manifests.
//!
//! # Modules
//!
//! - [`config`]: TOML parsing and validation pipeline
//! - [`driver`]: `UniversalDriver` -- dispatches capability traits through config
//! - [`factory`]: `UniversalDriverFactory` -- implements `DriverFactory` for registry integration
//! - [`template`]: MiniJinja template engine with custom filters
//! - [`response`]: Tiered response parsing (SCPI, format, transform, regex)
//! - [`transport`]: Communication abstraction (mock, serial, TCP, UDP)

pub mod config;
pub mod driver;
pub mod factory;
pub mod format_parser;
pub mod response;
pub mod string_utils;
pub mod template;
pub mod transform;
pub mod transport;

#[cfg(feature = "emulator")]
pub mod emulator;

#[cfg(test)]
pub(crate) mod test_fixtures;
