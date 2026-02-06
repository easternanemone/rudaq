//! Integration test for IPG laser TCP communication (read-only).
//!
//! Run with: cargo test --package hardware --features serial --test ipg_tcp_test -- --ignored --nocapture
//!
//! Requires the IPG laser to be connected at 10.0.0.15:10001.

use anyhow::Result;
use hardware::config::loader_v2::load_device_manifest_v2;
use hardware::drivers::generic_scpi::{DynDeviceIo, GenericScpiDriver};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// Test the GenericScpiDriver with the IPG TOML config.
/// This is the REAL declarative driver test.
#[tokio::test]
#[ignore = "requires IPG laser on 10.0.0.15:10001"]
async fn test_ipg_declarative_driver() -> Result<()> {
    // Load the TOML manifest (path relative to workspace root)
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/devices/ipg_laser.scpi.toml");
    let manifest = load_device_manifest_v2(&manifest_path)?;
    println!("Loaded manifest: {}", manifest.name);

    // Connect via TCP (as the factory would do)
    let addr = "10.0.0.15:10001";
    let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await??;
    stream.set_nodelay(true)?;
    println!("Connected to {}", addr);

    // Create the declarative driver
    let io: DynDeviceIo = Box::new(stream);
    let shared_io = Arc::new(Mutex::new(io));
    let driver = GenericScpiDriver::new(manifest, shared_io)?;

    // Test read commands using the TOML-defined command names
    println!("\n--- Testing TOML-defined commands ---");

    // read_status -> "STA"
    let response = driver.exec("read_status", ()).await?;
    println!("read_status: {}", response.trim());

    // read_emission -> "REM"
    let response = driver.exec("read_emission", ()).await?;
    println!("read_emission: {}", response.trim());

    // read_power -> "ROP"
    let response = driver.exec("read_power", ()).await?;
    println!("read_power: {}", response.trim());

    // read_rep_rate -> "RRR"
    let response = driver.exec("read_rep_rate", ()).await?;
    println!("read_rep_rate: {}", response.trim());

    // read_current -> "RCS"
    let response = driver.exec("read_current", ()).await?;
    println!("read_current: {}", response.trim());

    println!("\nDeclarative driver test PASSED!");
    Ok(())
}

/// Test that the GenericScpiDriver can load the IPG config.
#[test]
fn test_ipg_config_loads() {
    let config_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/devices/ipg_laser.scpi.toml");

    let manifest = load_device_manifest_v2(&config_path).expect("Failed to load IPG config");
    assert_eq!(manifest.name, "IPG Fiber Laser");

    // Verify TCP connection config
    use hardware::config::schema_v2::ConnectionConfig;
    match &manifest.connection {
        ConnectionConfig::Tcp {
            host,
            port,
            timeout_ms,
            ..
        } => {
            assert_eq!(host, "10.0.0.15");
            assert_eq!(*port, 10001);
            assert_eq!(*timeout_ms, 1000);
        }
        _ => panic!("Expected TCP connection config"),
    }
}

/// Test the FULL user experience: registry loads device from TOML config only.
/// This is how a real user would configure the IPG laser - NO RUST CODE.
///
/// User writes two TOML files:
/// 1. Device manifest: config/devices/ipg_laser.scpi.toml (defines commands)
/// 2. Hardware config: [[devices]] entry pointing to the manifest
#[tokio::test]
#[ignore = "requires IPG laser on 10.0.0.15:10001"]
async fn test_ipg_via_registry_toml_only() -> Result<()> {
    use hardware::registry::DeviceRegistry;

    // Resolve manifest path relative to workspace
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/devices/ipg_laser.scpi.toml");

    // This TOML is what a USER would write - no Rust code needed!
    // The only thing they configure is the manifest path.
    let driver_config: toml::Value = toml::from_str(&format!(
        r#"
type = "generic_scpi"
manifest = "{}"
"#,
        manifest_path.display()
    ))?;

    // Create registry and register factories
    let registry = DeviceRegistry::new();
    hardware::registry::register_all_factories(&registry, None).await?;

    // Register device from TOML (as the daemon would)
    registry
        .register_from_toml(
            "ipg_laser",
            "IPG Fiber Laser",
            "generic_scpi",
            driver_config,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to register: {}", e))?;

    // Verify device is registered
    println!("\n--- Testing via Registry (TOML-only config) ---");

    let device_info = registry
        .get_device_info("ipg_laser")
        .expect("IPG laser should be registered");

    println!(
        "Device registered: {} ({})",
        device_info.name, device_info.id
    );
    println!("Driver type: {}", device_info.driver_type);
    println!("Capabilities: {:?}", device_info.capabilities);

    // Verify readable capability is available
    let _readable = registry
        .get_readable("ipg_laser")
        .expect("IPG laser should have readable capability");

    // Note: read() would fail because ROP returns "Off" when emission is disabled.
    // In production, you'd handle this in the TOML response parsing.
    // For this test, we just verify the capability is wired up.
    println!("Readable capability: available");

    println!("\nRegistry-based TOML-only instantiation PASSED!");
    println!("User wrote ZERO lines of Rust code to configure this device.");
    Ok(())
}
