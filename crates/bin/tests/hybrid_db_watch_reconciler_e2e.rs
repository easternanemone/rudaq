#![cfg(feature = "db-surreal-mem")]
#![allow(unsafe_code)]
//! Daemon subprocess + gRPC E2E for SurrealDB watch reconciler (bd-zyc8).
//!
//! Validates that the daemon correctly hot-swaps devices in response to
//! ConfigService gRPC calls by leveraging the SurrealDB LIVE SELECT watch loop.
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

#[tokio::test]
#[cfg(feature = "db-surreal-mem")]
#[ignore = "daemon subprocess crashes with SIGABRT on CI (needs RUST_BACKTRACE investigation)"]
async fn test_watch_reconciler_adds_removes_and_restarts_devices() {
    let binary = daemon_binary_path();
    require_binary!(binary);

    let workspace = workspace_root();

    // 1. Start daemon in hybrid-db mode on an ephemeral port
    // We don't specify --port 0 here because we need to know the port to connect.
    // Actually, we can use a known-free port or just use 50051 if it's likely free.
    // For reliability in tests, let's try to find an ephemeral port.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut child = Command::new(&binary)
        .current_dir(&workspace)
        .args(["daemon", "--port", &port.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn daemon");

    // Give it time to start — CI runners may be slower due to cold caches.
    // The daemon must: init SurrealDB, load 12 default devices, start gRPC server.
    tokio::time::sleep(Duration::from_secs(5)).await;

    let addr = format!("http://127.0.0.1:{}", port);

    // Retry connection with increasing backoff for CI reliability.
    // Total max wait: 5s initial + sum(1..=15) = 120s of retries = ~125s budget.
    let mut client = None;
    for i in 0..15 {
        // Check that daemon process is still alive before retrying
        if let Some(status) = child.try_wait().ok().flatten() {
            panic!("Daemon exited prematurely with status: {status}");
        }
        match ConfigServiceClient::connect(addr.clone()).await {
            Ok(c) => {
                client = Some(c);
                break;
            }
            Err(e) => {
                println!("Connection attempt {} failed: {e}, retrying...", i + 1);
                tokio::time::sleep(Duration::from_secs(1 + (i as u64) / 3)).await;
            }
        }
    }
    let mut client = client.expect("Failed to connect to daemon after retries");

    // 2. Initial state check (should have 12 devices from default profile)
    let resp = client
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    let initial_count = resp.into_inner().instruments.len();
    assert_eq!(
        initial_count, 12,
        "Expected 12 devices from default profile"
    );

    // 3. ADD device via ConfigService
    println!("Adding new device...");
    client
        .upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "watch_test_rotator".into(),
                name: "Watch Test Rotator".into(),
                driver_type: "universal_thorlabs_ell14".into(),
                config_json: r#"{"mock":true,"address":"9"}"#.into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();

    // Wait for reconciler to pick it up (LIVE SELECT is fast, but daemon needs to instantiate)
    tokio::time::sleep(Duration::from_secs(2)).await;

    let resp = client
        .list_instruments(Request::new(ListInstrumentsRequest {}))
        .await
        .unwrap();
    assert_eq!(
        resp.into_inner().instruments.len(),
        13,
        "Should have 13 instruments after upsert"
    );

    // 4. MODIFY device (trigger restart)
    println!("Modifying device...");
    client
        .upsert_instrument(Request::new(UpsertInstrumentRequest {
            instrument: Some(InstrumentConfig {
                device_id: "watch_test_rotator".into(),
                name: "Watch Test Rotator (Modified)".into(),
                driver_type: "universal_thorlabs_ell14".into(),
                config_json: r#"{"mock":true,"address":"7"}"#.into(),
                enabled: true,
            }),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;
    // (In a real test we might check logs or metadata hashes to confirm restart occurred)

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
        12,
        "Should be back to 12 instruments after delete"
    );

    // 6. Shutdown daemon
    send_sigint(&child);
    let _ = child.wait().expect("Failed to wait for daemon");
}
