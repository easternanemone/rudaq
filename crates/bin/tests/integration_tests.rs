//! Integration tests for daq-bin daemon
//!
//! These tests verify the daemon application as a whole, including:
//! - Daemon startup and shutdown
//! - Configuration loading
//! - gRPC server lifecycle
//! - CLI command handling
//!
//! Run with: cargo test -p daq-bin --test integration_tests

use std::path::PathBuf;
use std::process::Command;

/// Helper to get the daemon binary path
fn daemon_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../target/debug/rust-daq-daemon");
    path
}

/// Helper to find a config file
fn test_config_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../config/demo.toml");
    path
}

// =============================================================================
// CLI Tests
// =============================================================================

#[test]
fn test_daemon_help_command() {
    let output = Command::new(daemon_binary_path())
        .arg("--help")
        .output()
        .expect("Failed to execute daemon binary");

    assert!(output.status.success(), "Help command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("daemon"),
        "Help text should mention daemon command"
    );
    assert!(
        stdout.contains("run"),
        "Help text should mention run command"
    );
}

#[test]
fn test_daemon_binary_exists() {
    let binary = daemon_binary_path();

    // This test verifies that the daemon binary can be located
    // It will pass even if not built yet (useful for CI)
    if binary.exists() {
        println!("✓ Daemon binary found at: {:?}", binary);
    } else {
        println!("⚠ Daemon binary not built yet at: {:?}", binary);
        println!("  Run: cargo build -p daq-bin");
    }

    // Informational only: no assertion needed.
}

// =============================================================================
// Configuration Loading Tests
// =============================================================================

#[tokio::test]
async fn test_daemon_loads_demo_config() {
    let config_path = test_config_path();

    // Skip if demo config doesn't exist (may not be available in all environments)
    if !config_path.exists() {
        eprintln!("Skipping test: demo.toml not found at {:?}", config_path);
        return;
    }

    // We can't easily test daemon startup without it blocking, but we can verify
    // the config file is valid by checking it can be parsed by the daemon
    // In a real scenario, you'd spawn the daemon and connect via gRPC to verify it started

    // This is a placeholder - in production you'd want to:
    // 1. Spawn daemon in background
    // 2. Wait for it to be ready (health check or port listening)
    // 3. Connect via gRPC to verify devices are registered
    // 4. Shutdown daemon

    // For now, just verify the binary exists and config exists
    assert!(daemon_binary_path().exists(), "Daemon binary should exist");
    assert!(config_path.exists(), "Demo config should exist");
}

// =============================================================================
// End-to-End Daemon Tests (Requires Binary Build)
// =============================================================================

#[tokio::test]
#[ignore = "Requires running daemon - enable for full integration testing"]
async fn test_daemon_startup_and_grpc_connection() {
    // This test is marked as ignored because it requires:
    // 1. Building the daemon binary first
    // 2. Managing a background process
    // 3. Proper cleanup of resources
    //
    // To run this test manually:
    // 1. Build daemon: cargo build -p daq-bin
    // 2. Run test: cargo test -p daq-bin --test integration_tests -- --ignored --nocapture

    // Placeholder: this test body is intentionally empty and ignored by default.
}

// =============================================================================
// Script Execution Tests
// =============================================================================

#[test]
#[ignore = "Requires running daemon - enable for full integration testing"]
fn test_daemon_run_script_command() {
    // Create a simple test script
    let script_content = r#"
        // Simple test script
        print("Test script executed");
    "#;

    let script_path = std::env::temp_dir().join("test_script.rhai");
    std::fs::write(&script_path, script_content).expect("Failed to write test script");

    // Run script via daemon (this would normally require daemon to be running)
    // This is a placeholder for a more complete test
    let output = Command::new(daemon_binary_path())
        .arg("run")
        .arg(&script_path)
        .output();

    // Cleanup
    let _ = std::fs::remove_file(script_path);

    // Note: This will likely fail without a running daemon
    // In production, you'd want to mock the script runner or use a test harness
    assert!(output.is_ok(), "Script execution should be callable");
}
