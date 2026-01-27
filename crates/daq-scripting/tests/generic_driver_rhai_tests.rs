//! Rhai Script Integration Tests for GenericDriverHandle
//!
//! Tests that execute actual Rhai scripts with GenericDriverHandle.
//! Uses mock serial ports to avoid hardware dependencies.
//!
//! Run with: `cargo nextest run -p daq-scripting --test generic_driver_rhai_tests`

use daq_hardware::config::load_device_config;
use daq_hardware::drivers::generic_serial::{DynSerial, GenericSerialDriver, SharedPort};
use daq_scripting::generic_driver_bindings::{
    register_generic_driver_functions, GenericDriverHandle,
};
use daq_scripting::SoftLimits;
use rhai::{Engine, Scope};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

// =============================================================================
// Mock Serial Port (reused from generic_driver_mock_tests.rs)
// =============================================================================

struct MockSerial {
    write_buf: Arc<std::sync::Mutex<Vec<u8>>>,
    read_buf: Arc<std::sync::Mutex<Cursor<Vec<u8>>>>,
}

impl MockSerial {
    fn new() -> Self {
        Self {
            write_buf: Arc::new(std::sync::Mutex::new(Vec::new())),
            read_buf: Arc::new(std::sync::Mutex::new(Cursor::new(Vec::new()))),
        }
    }

    fn set_response(&self, response: &str) {
        let mut buf = self.read_buf.lock().unwrap();
        *buf = Cursor::new(response.as_bytes().to_vec());
    }
}

impl Clone for MockSerial {
    fn clone(&self) -> Self {
        Self {
            write_buf: Arc::clone(&self.write_buf),
            read_buf: Arc::clone(&self.read_buf),
        }
    }
}

impl tokio::io::AsyncRead for MockSerial {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let mut read_buf = self.read_buf.lock().unwrap();
        let data = read_buf.get_ref();
        let pos = read_buf.position() as usize;
        let remaining = &data[pos..];
        let to_copy = std::cmp::min(remaining.len(), buf.remaining());
        buf.put_slice(&remaining[..to_copy]);
        read_buf.set_position((pos + to_copy) as u64);
        std::task::Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncWrite for MockSerial {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let mut write_buf = self.write_buf.lock().unwrap();
        write_buf.extend_from_slice(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl Unpin for MockSerial {}

fn create_mock_port() -> (SharedPort, MockSerial) {
    let mock = MockSerial::new();
    let boxed: DynSerial = Box::new(mock.clone());
    let port: SharedPort = Arc::new(Mutex::new(boxed));
    (port, mock)
}

fn ell14_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("config/devices/ell14.toml")
}

/// Create a test handle with mock serial port
fn create_test_handle() -> Option<GenericDriverHandle> {
    let config_path = ell14_config_path();
    if !config_path.exists() {
        return None;
    }

    let (port, _mock) = create_mock_port();
    let config = load_device_config(&config_path).ok()?;
    let driver = GenericSerialDriver::new(config, port, "2").ok()?;

    Some(GenericDriverHandle::new(
        driver,
        SoftLimits::new(0.0, 360.0),
        config_path.to_string_lossy().to_string(),
    ))
}

// =============================================================================
// Rhai Integration Tests
// =============================================================================

/// Test that soft limits are enforced in Rhai scripts
#[tokio::test(flavor = "multi_thread")]
async fn test_rhai_soft_limit_enforced() {
    let handle = match create_test_handle() {
        Some(h) => h,
        None => {
            eprintln!("Skipping test: ELL14 config not found");
            return;
        }
    };

    let mut engine = Engine::new();
    register_generic_driver_functions(&mut engine);

    let mut scope = Scope::new();
    scope.push("driver", handle);

    // Try to move beyond soft limit (360.0)
    let result = engine.eval_with_scope::<()>(
        &mut scope,
        r#"
        driver.move_abs(500.0);
    "#,
    );

    assert!(result.is_err(), "Should fail when exceeding soft limit");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("500") || err.contains("limit") || err.contains("360"),
        "Error should mention the limit violation: {}",
        err
    );
}

/// Test setting soft limits from Rhai script
#[tokio::test(flavor = "multi_thread")]
async fn test_rhai_set_soft_limits() {
    let handle = match create_test_handle() {
        Some(h) => h,
        None => {
            eprintln!("Skipping test: ELL14 config not found");
            return;
        }
    };

    let mut engine = Engine::new();
    register_generic_driver_functions(&mut engine);

    let mut scope = Scope::new();
    scope.push("driver", handle);

    // Set new soft limits
    let result = engine.eval_with_scope::<()>(
        &mut scope,
        r#"
        driver.set_soft_limits(0.0, 180.0);
    "#,
    );

    assert!(result.is_ok(), "set_soft_limits should succeed");

    // Now 200.0 should fail (new limit is 180.0)
    let result2 = engine.eval_with_scope::<()>(
        &mut scope,
        r#"
        driver.move_abs(200.0);
    "#,
    );

    assert!(
        result2.is_err(),
        "Should fail when exceeding new soft limit"
    );
}

/// Test address() method returns correct address
#[tokio::test(flavor = "multi_thread")]
async fn test_rhai_address_method() {
    let handle = match create_test_handle() {
        Some(h) => h,
        None => {
            eprintln!("Skipping test: ELL14 config not found");
            return;
        }
    };

    let mut engine = Engine::new();
    register_generic_driver_functions(&mut engine);

    let mut scope = Scope::new();
    scope.push("driver", handle);

    let result: String = engine
        .eval_with_scope(
            &mut scope,
            r#"
        driver.address()
    "#,
        )
        .expect("address() should succeed");

    assert_eq!(result, "2", "Address should be '2'");
}

/// Test that GenericDriverHandle can be cloned in Rhai (multiple references)
#[tokio::test(flavor = "multi_thread")]
async fn test_rhai_handle_clone() {
    let handle = match create_test_handle() {
        Some(h) => h,
        None => {
            eprintln!("Skipping test: ELL14 config not found");
            return;
        }
    };

    let mut engine = Engine::new();
    register_generic_driver_functions(&mut engine);

    let mut scope = Scope::new();
    scope.push("driver", handle);

    // Clone handle in script and verify both work
    let result = engine.eval_with_scope::<()>(
        &mut scope,
        r#"
        let driver2 = driver;
        let addr1 = driver.address();
        let addr2 = driver2.address();
        if addr1 != addr2 {
            throw "Addresses should match after clone";
        }
    "#,
    );

    assert!(result.is_ok(), "Handle cloning should work: {:?}", result);
}
