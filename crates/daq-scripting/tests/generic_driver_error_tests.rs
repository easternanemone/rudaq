#![cfg(feature = "generic_driver")]
//! Error Handling Tests for GenericDriverHandle
//!
//! Tests verifying helpful error messages for common failure cases.
//!
//! Run with: `cargo nextest run -p daq-scripting --features generic_driver --test generic_driver_error_tests`

use daq_scripting::generic_driver_bindings::register_generic_driver_functions;
use rhai::Engine;

// =============================================================================
// Error Handling Tests
// =============================================================================

/// Test that loading a nonexistent config gives a helpful error
#[tokio::test(flavor = "multi_thread")]
async fn test_config_not_found_error() {
    let mut engine = Engine::new();
    register_generic_driver_functions(&mut engine);

    let result = engine.eval::<()>(
        r#"
        let driver = create_generic_driver("/nonexistent/config.toml", "/dev/null", "0");
    "#,
    );

    assert!(result.is_err(), "Should fail with nonexistent config");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("config") || err.contains("load") || err.contains("not found"),
        "Error should mention config loading failure: {}",
        err
    );
}

/// Test that an invalid port path gives a helpful error
#[tokio::test(flavor = "multi_thread")]
async fn test_port_not_found_error() {
    let mut engine = Engine::new();
    register_generic_driver_functions(&mut engine);

    // Use real config but nonexistent port
    let config_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("config/devices/ell14.toml");

    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found");
        return;
    }

    let script = format!(
        r#"
        let driver = create_generic_driver("{}", "/dev/nonexistent_port_12345", "0");
    "#,
        config_path.to_string_lossy().replace("\\", "\\\\")
    );

    let result = engine.eval::<()>(&script);

    assert!(result.is_err(), "Should fail with nonexistent port");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("port")
            || err.contains("open")
            || err.contains("serial")
            || err.contains("No such file"),
        "Error should mention port opening failure: {}",
        err
    );
}

/// Test that soft limit errors are clear
#[tokio::test(flavor = "multi_thread")]
async fn test_soft_limit_error_message() {
    use daq_hardware::config::load_device_config;
    use daq_hardware::drivers::generic_serial::{DynSerial, GenericSerialDriver, SharedPort};
    use daq_scripting::generic_driver_bindings::GenericDriverHandle;
    use daq_scripting::SoftLimits;
    use rhai::Scope;
    use std::io::Cursor;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // Create mock serial
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

    let config_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("config/devices/ell14.toml");

    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found");
        return;
    }

    let mock = MockSerial::new();
    let boxed: DynSerial = Box::new(mock);
    let port: SharedPort = Arc::new(Mutex::new(boxed));

    let config = load_device_config(&config_path).expect("Failed to load config");
    let driver = GenericSerialDriver::new(config, port, "2").expect("Failed to create driver");

    let handle = GenericDriverHandle::new(
        driver,
        SoftLimits::new(0.0, 100.0), // Limit to 0-100
        config_path.to_string_lossy().to_string(),
    );

    let mut engine = Engine::new();
    register_generic_driver_functions(&mut engine);

    let mut scope = Scope::new();
    scope.push("driver", handle);

    // Try to exceed soft limit
    let result = engine.eval_with_scope::<()>(
        &mut scope,
        r#"
        driver.move_abs(150.0);
    "#,
    );

    assert!(result.is_err(), "Should fail when exceeding soft limit");
    let err = result.unwrap_err().to_string();
    // Error should contain useful information about the violation
    assert!(
        err.contains("150")
            || err.contains("100")
            || err.contains("limit")
            || err.contains("exceeds"),
        "Error should mention the limit violation: {}",
        err
    );
}

/// Test that negative soft limit violations give clear errors
#[tokio::test(flavor = "multi_thread")]
async fn test_soft_limit_below_min_error() {
    use daq_hardware::config::load_device_config;
    use daq_hardware::drivers::generic_serial::{DynSerial, GenericSerialDriver, SharedPort};
    use daq_scripting::generic_driver_bindings::GenericDriverHandle;
    use daq_scripting::SoftLimits;
    use rhai::Scope;
    use std::io::Cursor;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // Simplified mock (same as above)
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

    let config_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("config/devices/ell14.toml");

    if !config_path.exists() {
        eprintln!("Skipping test: ELL14 config not found");
        return;
    }

    let mock = MockSerial::new();
    let boxed: DynSerial = Box::new(mock);
    let port: SharedPort = Arc::new(Mutex::new(boxed));

    let config = load_device_config(&config_path).expect("Failed to load config");
    let driver = GenericSerialDriver::new(config, port, "2").expect("Failed to create driver");

    let handle = GenericDriverHandle::new(
        driver,
        SoftLimits::new(10.0, 100.0), // Minimum is 10
        config_path.to_string_lossy().to_string(),
    );

    let mut engine = Engine::new();
    register_generic_driver_functions(&mut engine);

    let mut scope = Scope::new();
    scope.push("driver", handle);

    // Try to go below minimum
    let result = engine.eval_with_scope::<()>(
        &mut scope,
        r#"
        driver.move_abs(5.0);
    "#,
    );

    assert!(result.is_err(), "Should fail when below soft limit minimum");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("5") || err.contains("10") || err.contains("limit") || err.contains("below"),
        "Error should mention the limit violation: {}",
        err
    );
}
