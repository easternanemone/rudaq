#![allow(unsafe_code)]
//! Process-level runtime-mode resolution precedence tests (bd-zyc8).
//!
//! Validates that the daemon correctly selects the runtime mode based on:
//! 1. CLI flags (--runtime-mode)
//! 2. Environment variables (RUSTDAQ_RUNTIME_MODE)
//! 3. Hardware config flags (--hardware-config / --lab-hardware)
//! 4. Default fallback (hybrid-db)
//!
//! Run with: cargo nextest run -p bin --test runtime_mode_resolution

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Locate the daemon binary in the build output directory.
fn daemon_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../target/debug/rust-daq-daemon");
    path
}

/// Locate the workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bin crate should be under /crates")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
}

/// Send SIGINT to a child process for graceful shutdown.
#[cfg(unix)]
#[allow(clippy::cast_possible_wrap)]
// SAFETY: test/benchmark values are bounded
fn send_sigint(child: &Child) {
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
}

#[cfg(not(unix))]
fn send_sigint(child: &mut Child) {
    child.kill().ok();
}

/// Skip the test if the daemon binary hasn't been built.
macro_rules! require_binary {
    ($binary:expr) => {
        if !$binary.exists() {
            eprintln!(
                "Skipping test: daemon binary not found at {:?} (run `cargo build -p bin` first)",
                $binary
            );
            return;
        }
    };
}

#[test]
fn test_runtime_mode_cli_overrides_env() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    // CLI: mock, ENV: hybrid-db -> Should be mock
    let child = Command::new(&binary)
        .env("RUSTDAQ_RUNTIME_MODE", "hybrid-db")
        .args(["daemon", "--runtime-mode", "mock", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    std::thread::sleep(Duration::from_secs(4));
    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("Runtime mode: mock"),
        "CLI flag should override env var. Output: {combined}"
    );
}

#[test]
fn test_runtime_mode_env_override_applies_when_cli_absent() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    // ENV: universal, no CLI flag -> Should be universal
    let child = Command::new(&binary)
        .env("RUSTDAQ_RUNTIME_MODE", "universal")
        .args(["daemon", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    std::thread::sleep(Duration::from_secs(4));
    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("Runtime mode: universal"),
        "Env var should apply when CLI absent. Output: {combined}"
    );
}

#[test]
fn test_runtime_mode_hardware_config_implies_custom_without_cli_or_env() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let workspace = workspace_root();
    let config_path = workspace.join("config/demo.toml");

    // --hardware-config, no CLI mode, no ENV -> Should be custom
    let child = Command::new(&binary)
        .current_dir(&workspace)
        .args([
            "daemon",
            "--port",
            "0",
            "--hardware-config",
            config_path.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    std::thread::sleep(Duration::from_secs(4));
    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("Runtime mode: custom"),
        "Hardware config should imply custom mode. Output: {combined}"
    );
}

#[test]
fn test_runtime_mode_invalid_env_falls_back_to_default() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    // ENV: invalid-mode -> Should fall back to hybrid-db and warn
    let child = Command::new(&binary)
        .env("RUSTDAQ_RUNTIME_MODE", "invalid-mode")
        .args(["daemon", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    std::thread::sleep(Duration::from_secs(4));
    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("Invalid runtime mode \"invalid-mode\" in env var"),
        "Should warn about invalid env var. Output: {combined}"
    );
    assert!(
        combined.contains("Runtime mode: hybrid-db"),
        "Should fall back to hybrid-db. Output: {combined}"
    );
}

#[test]
#[cfg(feature = "db-surreal-mem")]
fn test_runtime_mode_default_hybrid_db_starts_from_workspace_root() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let workspace = workspace_root();

    // No flags, CWD = workspace root -> Should be hybrid-db
    let child = Command::new(&binary)
        .current_dir(&workspace)
        .args(["daemon", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    std::thread::sleep(Duration::from_secs(4));
    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("Runtime mode: hybrid-db"),
        "Default should be hybrid-db. Output: {combined}"
    );
    assert!(
        combined.contains("Starting Headless DAQ Daemon"),
        "Should start successfully"
    );
}
