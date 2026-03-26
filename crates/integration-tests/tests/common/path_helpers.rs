//! Shared path helpers for integration tests.

use std::path::PathBuf;

/// Resolve repository workspace root from integration-tests crate location.
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("integration-tests crate should be under /crates")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
}
