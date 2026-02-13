//! Process-level lifecycle tests for the rust-daq daemon.
//!
//! These tests exercise the daemon binary via `std::process::Command`,
//! verifying CLI argument validation and error handling without
//! needing access to private modules.
//!
//! Run with: cargo nextest run -p bin --test daemon_lifecycle

use std::path::PathBuf;
use std::process::Command;

/// Locate the daemon binary in the build output directory.
fn daemon_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../target/debug/rust-daq-daemon");
    path
}

/// The CLI declares `--hardware-config` and `--lab-hardware` as
/// `conflicts_with` each other. Passing both must produce a non-zero
/// exit code.
#[test]
fn test_daemon_rejects_conflicting_args() {
    let binary = daemon_binary_path();
    if !binary.exists() {
        eprintln!(
            "Skipping test: daemon binary not found at {:?} (run `cargo build -p bin` first)",
            binary
        );
        return;
    }

    let output = Command::new(&binary)
        .args(["daemon", "--hardware-config", "foo.toml", "--lab-hardware"])
        .output()
        .expect("Failed to execute daemon binary");

    assert!(
        !output.status.success(),
        "Daemon should reject conflicting --hardware-config and --lab-hardware flags"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used with")
            || stderr.contains("conflict")
            || stderr.contains("error"),
        "stderr should mention the argument conflict, got: {}",
        stderr
    );
}

/// Pointing `--hardware-config` at a file that does not exist should
/// cause the daemon to exit with a non-zero status rather than
/// silently falling back to mock devices.
#[test]
fn test_daemon_missing_config_file() {
    let binary = daemon_binary_path();
    if !binary.exists() {
        eprintln!(
            "Skipping test: daemon binary not found at {:?} (run `cargo build -p bin` first)",
            binary
        );
        return;
    }

    let output = Command::new(&binary)
        .args([
            "daemon",
            "--hardware-config",
            "nonexistent_config_file.toml",
        ])
        .output()
        .expect("Failed to execute daemon binary");

    assert!(
        !output.status.success(),
        "Daemon should fail when given a nonexistent hardware config file"
    );
}
