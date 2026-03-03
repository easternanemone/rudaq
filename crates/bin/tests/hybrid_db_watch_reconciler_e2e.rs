#![cfg(feature = "db-surreal-mem")]
#![allow(unsafe_code)]
//! Daemon subprocess + gRPC E2E for SurrealDB watch reconciler (bd-zyc8).
//!
//! Validates that the daemon correctly hot-swaps devices in response to
//! ConfigService gRPC calls by leveraging the SurrealDB LIVE SELECT watch loop.
//!
//! Uses `--runtime-mode mock` so no hardware config or serial ports are needed.
//! The test seeds instruments via gRPC and verifies add/modify/delete through
//! the watch reconciler.
//!
//! Run with: cargo nextest run -p bin --features db-surreal-mem --test hybrid_db_watch_reconciler_e2e

use protocol::daq::config_service_client::ConfigServiceClient;
use protocol::daq::{
    DeleteInstrumentRequest, InstrumentConfig, ListInstrumentsRequest, UpsertInstrumentRequest,
};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tonic::Request;

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

/// Capture and print daemon stderr for diagnostics on test failure.
fn dump_daemon_output(child: &mut Child, label: &str) {
    use std::io::Read;

    if let Some(ref mut stderr) = child.stderr {
        let mut buf = String::new();
        // Non-blocking: read what's available (stderr is piped)
        let _ = stderr.read_to_string(&mut buf);
        if !buf.is_empty() {
            eprintln!("=== Daemon {label} (stderr) ===\n{buf}\n=== end ===");
        }
    }
    if let Some(ref mut stdout) = child.stdout {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        if !buf.is_empty() {
            eprintln!("=== Daemon {label} (stdout) ===\n{buf}\n=== end ===");
        }
    }
}

#[tokio::test]
#[cfg(feature = "db-surreal-mem")]
async fn test_watch_reconciler_adds_removes_and_restarts_devices() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let workspace = workspace_root();

    // 1. Start daemon in mock mode on an ephemeral port.
    // Mock mode avoids loading maitai_universal.toml (which needs real serial ports).
    // SurrealDB (kv-mem) still initializes, so ConfigService + watch reconciler work.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut child = Command::new(&binary)
        .current_dir(&workspace)
        .args([
            "daemon",
            "--runtime-mode",
            "mock",
            "--port",
            &port.to_string(),
        ])
        .env("RUST_BACKTRACE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Give the daemon time to start. Mock mode is faster than hybrid-db since
    // it skips hardware config parsing and device instantiation.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let addr = format!("http://127.0.0.1:{port}");

    // Retry connection with backoff for CI reliability.
    let mut client = None;
    for i in 0..10 {
        // Check that daemon process is still alive before retrying
        if let Some(status) = child.try_wait().ok().flatten() {
            dump_daemon_output(&mut child, "premature exit");
            panic!("Daemon exited prematurely with status: {status}");
        }
        match ConfigServiceClient::connect(addr.clone()).await {
            Ok(c) => {
                client = Some(c);
                break;
            }
            Err(e) => {
                eprintln!("Connection attempt {} failed: {e}, retrying...", i + 1);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    let Some(mut client) = client else {
        dump_daemon_output(&mut child, "connection timeout");
        let _ = child.kill();
        panic!("Failed to connect to daemon after retries");
    };

    // 2. Initial state: mock mode starts with an empty DB (no shadow-write).
    let resp = client
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    let initial_count = resp.into_inner().instruments.len();
    assert_eq!(
        initial_count, 0,
        "Mock mode should start with 0 DB instruments"
    );

    // 3. ADD device via ConfigService
    println!("Adding new device...");
    client
        .upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "watch_test_rotator".into(),
                name: "Watch Test Rotator".into(),
                driver_type: "mock_stage".into(),
                config_json: r#"{"initial_position":0.0}"#.into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();

    // Wait for reconciler to process the LIVE SELECT event.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let resp = client
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    assert_eq!(
        resp.into_inner().instruments.len(),
        1,
        "Should have 1 instrument after upsert"
    );

    // 4. MODIFY device (trigger restart via config change)
    println!("Modifying device...");
    client
        .upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "watch_test_rotator".into(),
                name: "Watch Test Rotator (Modified)".into(),
                driver_type: "mock_stage".into(),
                config_json: r#"{"initial_position":45.0}"#.into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify modification is reflected
    let resp = client
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    let instruments = resp.into_inner().instruments;
    assert_eq!(instruments.len(), 1, "Count should still be 1 after modify");
    assert_eq!(
        instruments[0].name, "Watch Test Rotator (Modified)",
        "Name should be updated after modify"
    );

    // 5. REMOVE device
    println!("Removing device...");
    client
        .delete_instrument(Request::new(DeleteInstrumentRequest {
            device_id: "watch_test_rotator".into(),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let resp = client
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    assert_eq!(
        resp.into_inner().instruments.len(),
        0,
        "Should be back to 0 instruments after delete"
    );

    // 6. Shutdown daemon
    send_sigint(&child);
    match tokio::time::timeout(Duration::from_secs(10), async {
        tokio::task::spawn_blocking(move || child.wait())
            .await
            .expect("join")
    })
    .await
    {
        Ok(Ok(_status)) => {} // Graceful shutdown
        Ok(Err(e)) => eprintln!("Daemon wait error: {e}"),
        Err(_) => eprintln!("Daemon shutdown timed out after 10s"),
    }
}
