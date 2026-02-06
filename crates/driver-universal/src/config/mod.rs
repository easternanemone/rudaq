//! Configuration types and validation for universal device manifests.
//!
//! This module implements a two-stage pipeline:
//! - **Stage 1** (`raw`): Permissive serde deserialization types (`RawManifest`)
//! - **Stage 2** (`validated`): Newtypes with enforced invariants (`DeviceManifest`)
//! - **Bridge** (`parse`): Validation pipeline converting raw -> validated

pub mod error;
pub mod parse;
pub mod raw;
pub mod validated;

pub use error::ConfigError;
pub use parse::parse_manifest;
pub use raw::RawManifest;
pub use validated::DeviceManifest;
