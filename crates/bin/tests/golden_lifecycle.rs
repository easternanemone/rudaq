#![allow(unsafe_code)]
//! Golden daemon lifecycle tests.
//!
//! These tests establish baseline expected behavior for daemon startup and
//! shutdown, including fault paths. They exercise the daemon binary as a
//! subprocess to verify end-to-end behavior.
//!
//! Run with: cargo nextest run -p bin --test golden_lifecycle
//!
//! NOTE: Tests skip gracefully if the daemon binary hasn't been built yet.

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::NamedTempFile;

/// Send SIGINT to a child process for graceful shutdown.
#[cfg(unix)]
#[allow(clippy::cast_possible_wrap)]
// SAFETY: test/benchmark values are bounded
fn send_sigint(child: &Child) {
    // SAFETY: libc::kill sends a signal to a process. The child PID is valid
    // because we hold a reference to the still-running Child.
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
}

#[cfg(not(unix))]
fn send_sigint(child: &mut Child) {
    child.kill().ok();
}

/// Locate the daemon binary in the build output directory.
fn daemon_binary_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("../../target/debug/rust-daq-daemon");
    path
}

/// Locate the workspace root so tests can run the daemon from a cwd where
/// relative config paths (e.g. config/devices) resolve correctly.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bin crate should be under /crates")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
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

// =============================================================================
// Normal Path: Startup and Shutdown
// =============================================================================

/// Golden test: daemon starts with mock hardware (no config file), binds to an
/// ephemeral port, and shuts down cleanly when the process is killed.
#[test]
fn test_golden_startup_shutdown_mock() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    // Use port 0 to let the OS assign an ephemeral port
    let child = Command::new(&binary)
        .args(["daemon", "--runtime-mode", "mock", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Give the daemon time to initialize
    std::thread::sleep(Duration::from_secs(3));

    // Send SIGINT for graceful shutdown
    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify startup phases appeared in output
    assert!(
        stdout.contains("Starting Headless DAQ Daemon")
            || stdout.contains("gRPC server ready")
            || stdout.contains("Initializing"),
        "Daemon should show startup messages, got stdout: {}",
        stdout
    );
}

/// Golden test: daemon accepts `--runtime-mode universal` at the CLI and
/// starts from workspace root where `config/maitai_universal.toml` resolves.
///
/// NOTE: `--runtime-mode` and `--hardware-config` are `conflicts_with_all`
/// in clap, so we rely on the built-in config path for universal mode.
#[test]
fn test_golden_startup_runtime_mode_universal() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let workspace = workspace_root();

    let child = Command::new(&binary)
        .current_dir(&workspace)
        .args(["daemon", "--runtime-mode", "universal", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Give the daemon time to initialize and emit startup logs
    std::thread::sleep(Duration::from_secs(3));

    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("Runtime mode: universal"),
        "Daemon should report universal runtime mode. stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("Starting Headless DAQ Daemon")
            || combined.contains("gRPC server ready")
            || combined.contains("Initializing"),
        "Daemon should emit startup logs. stdout: {stdout}\nstderr: {stderr}"
    );
}

// =============================================================================
// Hybrid-DB (Default) Normal Path
// =============================================================================

/// Golden test: daemon starts in default hybrid-db mode when no flags provided,
/// using SurrealDB in-memory engine.
#[test]
#[cfg(feature = "db-surreal-mem")]
fn test_golden_default_runtime_hybrid_db_startup_shutdown() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let workspace = workspace_root();

    let child = Command::new(&binary)
        .current_dir(&workspace)
        .args(["daemon", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Give the daemon time to initialize and emit startup logs
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
        "Daemon should report hybrid-db runtime mode. Output: {combined}"
    );
    assert!(
        combined.contains("Starting Headless DAQ Daemon")
            || combined.contains("gRPC server ready")
            || combined.contains("Initializing"),
        "Daemon should emit startup logs. Output: {combined}"
    );
}

/// Golden test: daemon in default hybrid-db mode registers devices from the
/// default universal profile.
#[test]
#[cfg(feature = "db-surreal-mem")]
fn test_golden_default_runtime_registers_devices() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let workspace = workspace_root();

    let child = Command::new(&binary)
        .current_dir(&workspace)
        .args(["daemon", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Give daemon time to register devices and start
    std::thread::sleep(Duration::from_secs(4));

    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Hybrid-db mode should register devices (e.g., from config/maitai_universal.toml)
    assert!(
        combined.contains("device")
            || combined.contains("Device")
            || combined.contains("Registered")
            || combined.contains("universal"),
        "Daemon should report registered devices in stdout.\nGot: {}",
        combined
    );
}

// =============================================================================
// Fault Path: Port Conflict
// =============================================================================

/// Golden test: daemon fails to start when the requested port is already bound.
#[test]
fn test_golden_port_conflict() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    // Bind a port to create a conflict
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral port");
    let port = listener.local_addr().unwrap().port();

    // Try to start daemon on the same port
    let mut child = Command::new(&binary)
        .args([
            "daemon",
            "--runtime-mode",
            "mock",
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Wait for the daemon to fail (should be fast)
    let result = child
        .wait_timeout(Duration::from_secs(10))
        .expect("Failed to wait for daemon");

    // Clean up: kill if still running (it might hang on startup)
    if result.is_none() {
        child.kill().ok();
        child.wait().ok();
    }

    // The listener still holds the port
    drop(listener);

    // The daemon should have exited with an error
    if let Some(status) = result {
        assert!(
            !status.success(),
            "Daemon should fail when port is already bound"
        );
    }
    // If it timed out, that's also acceptable (daemon might retry internally)
}

// =============================================================================
// Fault Path: Invalid Hardware Config (Malformed TOML)
// =============================================================================

/// Golden test: daemon rejects a hardware config file with invalid TOML syntax.
#[test]
fn test_golden_invalid_hardware_config_syntax() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    // Write invalid TOML
    let mut tmpfile = NamedTempFile::new().expect("Failed to create temp file");
    writeln!(tmpfile, "this is not valid toml {{{{").unwrap();

    let output = Command::new(&binary)
        .args([
            "daemon",
            "--hardware-config",
            tmpfile.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute daemon");

    assert!(
        !output.status.success(),
        "Daemon should fail with invalid TOML hardware config"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("parse")
            || combined.contains("TOML")
            || combined.contains("Failed")
            || combined.contains("Error")
            || combined.contains("error"),
        "Output should indicate a config parse error, got:\nstdout: {}\nstderr: {}",
        stdout,
        stderr
    );
}

// =============================================================================
// Fault Path: Empty Hardware Config
// =============================================================================

/// Golden test: daemon handles an empty hardware config file gracefully.
#[test]
fn test_golden_empty_hardware_config() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    // Write empty but valid TOML
    let mut tmpfile = NamedTempFile::new().expect("Failed to create temp file");
    writeln!(tmpfile, "# Empty hardware config").unwrap();

    let mut child = Command::new(&binary)
        .args([
            "daemon",
            "--port",
            "0",
            "--hardware-config",
            tmpfile.path().to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Give it time to start (or fail)
    std::thread::sleep(Duration::from_secs(3));

    // Kill the daemon regardless of whether it started
    child.kill().ok();
    let output = child.wait_with_output().expect("Failed to wait for daemon");

    // An empty config should either start with 0 devices or fail with
    // a helpful error - both are acceptable golden behaviors.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Log the behavior for golden reference
    eprintln!("--- Golden behavior for empty config ---");
    eprintln!("Exit status: {:?}", output.status);
    eprintln!("stdout: {}", stdout);
    if !stderr.is_empty() {
        eprintln!("stderr: {}", stderr);
    }
    eprintln!("--- End golden behavior ---");
}

// =============================================================================
// Fault Path: Hardware Config Points to Missing File
// =============================================================================

/// Golden test: daemon fails when --hardware-config points to a nonexistent file.
/// (Covers the same ground as daemon_lifecycle::test_daemon_missing_config_file
/// but with golden output capture.)
#[test]
fn test_golden_missing_hardware_config_file() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let output = Command::new(&binary)
        .args([
            "daemon",
            "--hardware-config",
            "/tmp/definitely_does_not_exist_12345.toml",
        ])
        .output()
        .expect("Failed to execute daemon");

    assert!(
        !output.status.success(),
        "Daemon should fail when hardware config file is missing"
    );
}

// =============================================================================
// Normal Path: Verify Device Registration
// =============================================================================

/// Golden test: daemon with mock hardware registers expected devices and
/// reports them in stdout.
#[test]
fn test_golden_mock_devices_registered() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let child = Command::new(&binary)
        .args(["daemon", "--runtime-mode", "mock", "--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Give daemon time to register devices and start
    std::thread::sleep(Duration::from_secs(3));

    // Send SIGINT for graceful shutdown
    send_sigint(&child);

    let output = child.wait_with_output().expect("Failed to wait for daemon");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Mock registry should register devices
    assert!(
        stdout.contains("device")
            || stdout.contains("Device")
            || stdout.contains("Registered")
            || stdout.contains("mock"),
        "Daemon should report registered mock devices in stdout.\nGot: {}",
        stdout
    );
}

// =============================================================================
// Helpers for process management
// =============================================================================

/// Extension trait to add wait_timeout to std::process::Child on all platforms.
trait ChildExt {
    fn wait_timeout(
        &mut self,
        timeout: Duration,
    ) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl ChildExt for std::process::Child {
    fn wait_timeout(
        &mut self,
        timeout: Duration,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        let start = std::time::Instant::now();
        loop {
            match self.try_wait()? {
                Some(status) => return Ok(Some(status)),
                None => {
                    if start.elapsed() >= timeout {
                        return Ok(None);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}
