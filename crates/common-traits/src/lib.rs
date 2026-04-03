//! # common-traits
//!
//! Capability traits, reactive parameters, and driver plugin API for the
//! rust-daq data acquisition system.
//!
//! This crate was extracted from `common` to break circular dependency chains
//! and establish a clean trait layer. The `common` crate re-exports everything
//! from here, so downstream consumers need zero import changes.

// Suppressed: generic types in doc comments (e.g., `Parameter<T>`) parse as HTML tags
#![allow(rustdoc::invalid_html_tags)]
#![allow(rustdoc::broken_intra_doc_links)]

pub mod capabilities;
pub mod driver;
pub mod modules;
pub mod observable;
pub mod parameter;
pub mod pipeline;
