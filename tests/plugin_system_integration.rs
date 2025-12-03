//! Integration tests for the plugin system.
//!
//! These tests verify the plugin loading, schema parsing, and handle type creation.
//! Full serial communication tests require mock serial support (Phase 7).

// Only compile these tests when tokio_serial feature is enabled
#[cfg(feature = "tokio_serial")]
mod plugin_tests {
    use anyhow::Result;
    use rust_daq::hardware::capabilities::{
        Actionable, Loggable, Movable, Readable, Settable, Switchable,
    };
    use rust_daq::hardware::plugin::handles::{
        PluginActionableHandle, PluginAxisHandle, PluginLoggableHandle, PluginSensorHandle,
        PluginSettableHandle, PluginSwitchableHandle,
    };
    use rust_daq::hardware::plugin::registry::PluginFactory;
    #[allow(unused_imports)]
    use rust_daq::hardware::plugin::schema::InstrumentConfig;
    use std::path::Path;

    #[tokio::test]
    async fn test_plugin_loading() -> Result<()> {
        let mut factory = PluginFactory::new();
        let plugins_dir = Path::new("./plugins");
        factory.load_plugins(plugins_dir).await?;

        // Verify test plugin is loaded
        let plugins = factory.available_plugins();
        assert!(
            plugins.contains(&"test-device-mock".to_string()),
            "Expected 'test-device-mock' in plugins: {:?}",
            plugins
        );

        // Verify display name
        assert_eq!(
            factory.plugin_display_name("test-device-mock"),
            Some("Mock Test Device")
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_plugin_config_parsing() -> Result<()> {
        let mut factory = PluginFactory::new();
        let plugins_dir = Path::new("./plugins");
        factory.load_plugins(plugins_dir).await?;

        // Get and verify config
        let config = factory
            .get_config("test-device-mock")
            .expect("Config should exist for test-device-mock");

        // Verify metadata
        assert_eq!(config.metadata.id, "test-device-mock");
        assert_eq!(config.metadata.name, "Mock Test Device");
        assert_eq!(config.metadata.version, "1.0.0");

        // Verify protocol settings
        assert_eq!(config.protocol.baud_rate, 9600);
        assert_eq!(config.protocol.termination, "\r\n");

        // Verify capabilities are parsed
        assert_eq!(config.capabilities.readable.len(), 2);
        assert_eq!(config.capabilities.settable.len(), 2);
        assert_eq!(config.capabilities.switchable.len(), 1);
        assert_eq!(config.capabilities.actionable.len(), 1);
        assert_eq!(config.capabilities.loggable.len(), 1);

        // Verify specific readable capability
        let temp_cap = config
            .capabilities
            .readable
            .iter()
            .find(|c| c.name == "temperature")
            .expect("Should have temperature capability");
        assert_eq!(temp_cap.command, "TEMP?");
        assert_eq!(temp_cap.unit, Some("C".to_string()));

        // Verify settable capability
        let setpoint = config
            .capabilities
            .settable
            .iter()
            .find(|c| c.name == "setpoint_temp")
            .expect("Should have setpoint_temp capability");
        assert_eq!(setpoint.min, Some(10.0));
        assert_eq!(setpoint.max, Some(40.0));

        // Verify switchable capability
        let heater = config
            .capabilities
            .switchable
            .iter()
            .find(|c| c.name == "heater")
            .expect("Should have heater capability");
        assert_eq!(heater.on_cmd, "HEAT ON");
        assert_eq!(heater.off_cmd, "HEAT OFF");

        Ok(())
    }

    #[test]
    fn test_handle_types_implement_traits() {
        // Compile-time verification that handles implement the expected traits.
        // These functions are never called, they just verify trait bounds at compile time.
        fn assert_movable<T: Movable>() {}
        fn assert_readable<T: Readable>() {}
        fn assert_settable<T: Settable>() {}
        fn assert_switchable<T: Switchable>() {}
        fn assert_actionable<T: Actionable>() {}
        fn assert_loggable<T: Loggable>() {}

        assert_movable::<PluginAxisHandle>();
        assert_readable::<PluginSensorHandle>();
        assert_settable::<PluginSettableHandle>();
        assert_switchable::<PluginSwitchableHandle>();
        assert_actionable::<PluginActionableHandle>();
        assert_loggable::<PluginLoggableHandle>();
    }

    #[tokio::test]
    async fn test_duplicate_plugin_id_error() -> Result<()> {
        // Create a temporary directory with two plugins having the same ID
        let temp_dir = tempfile::tempdir()?;

        let plugin1 = r#"
metadata:
  id: "duplicate-id"
  name: "Plugin 1"
  version: "1.0.0"
  driver_type: "serial_scpi"

protocol:
  baud_rate: 9600
  termination: "\r\n"
"#;

        let plugin2 = r#"
metadata:
  id: "duplicate-id"
  name: "Plugin 2"
  version: "1.0.0"
  driver_type: "serial_scpi"

protocol:
  baud_rate: 9600
  termination: "\r\n"
"#;

        std::fs::write(temp_dir.path().join("plugin1.yaml"), plugin1)?;
        std::fs::write(temp_dir.path().join("plugin2.yaml"), plugin2)?;

        let mut factory = PluginFactory::new();
        let result = factory.load_plugins(temp_dir.path()).await;

        assert!(result.is_err(), "Should error on duplicate plugin IDs");

        Ok(())
    }

    #[tokio::test]
    async fn test_invalid_plugin_path() -> Result<()> {
        let mut factory = PluginFactory::new();
        let result = factory.load_plugins(Path::new("/nonexistent/path")).await;

        assert!(result.is_err(), "Should error on invalid path");

        Ok(())
    }

    #[tokio::test]
    async fn test_spawn_nonexistent_plugin() -> Result<()> {
        let factory = PluginFactory::new();
        let result = factory.spawn("nonexistent-plugin", "/dev/null").await;

        assert!(
            result.is_err(),
            "Should error when spawning nonexistent plugin"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_movable_capability() -> Result<()> {
        let mut factory = PluginFactory::new();
        let plugins_dir = Path::new("./plugins");
        factory.load_plugins(plugins_dir).await?;

        // Get config to verify movable capability exists
        let config = factory
            .get_config("test-device-mock")
            .expect("Config should exist for test-device-mock");

        // Verify movable capability is parsed
        assert!(
            config.capabilities.movable.is_some(),
            "Should have movable capability"
        );
        let movable = config.capabilities.movable.as_ref().unwrap();
        assert_eq!(movable.axes.len(), 2, "Should have 2 axes");

        // Verify axis configurations
        let x_axis = movable
            .axes
            .iter()
            .find(|a| a.name == "x")
            .expect("Should have x axis");
        assert_eq!(x_axis.unit, Some("mm".to_string()));
        assert_eq!(x_axis.min, Some(0.0));
        assert_eq!(x_axis.max, Some(100.0));

        let y_axis = movable
            .axes
            .iter()
            .find(|a| a.name == "y")
            .expect("Should have y axis");
        assert_eq!(y_axis.unit, Some("mm".to_string()));

        // Verify command templates
        assert_eq!(movable.set_cmd, "POS:{axis} {val}");
        assert_eq!(movable.get_cmd, "POS:{axis}?");
        assert_eq!(movable.get_pattern, "{val:f}");

        Ok(())
    }

    // Note: Skipped until mock connection support is added to GenericDriver.
    // The Movable trait implementation is verified via compile-time checks in
    // test_handle_types_implement_traits and schema parsing in test_movable_capability.
    #[tokio::test]
    #[ignore = "Requires mock connection support in GenericDriver"]
    async fn test_movable_trait_in_mock_mode() -> Result<()> {
        use std::sync::Arc;

        let mut factory = PluginFactory::new();
        let plugins_dir = Path::new("./plugins");
        factory.load_plugins(plugins_dir).await?;

        // Spawn driver in mock mode (no serial port needed)
        let driver = Arc::new(factory.spawn("test-device-mock", "/dev/null").await?);

        // Create axis handles
        let x_axis = driver.axis_handle("x", true); // is_mocking = true
        let y_axis = driver.axis_handle("y", true);

        // Test absolute move
        x_axis.move_abs(50.0).await?;
        let x_pos = x_axis.position().await?;
        assert_eq!(x_pos, 50.0, "X position should be 50.0");

        // Test relative move
        x_axis.move_rel(10.0).await?;
        let x_pos = x_axis.position().await?;
        assert_eq!(x_pos, 60.0, "X position should be 60.0 after relative move");

        // Test independent axis control
        y_axis.move_abs(25.0).await?;
        let y_pos = y_axis.position().await?;
        assert_eq!(y_pos, 25.0, "Y position should be 25.0");

        // Verify x hasn't changed
        let x_pos = x_axis.position().await?;
        assert_eq!(x_pos, 60.0, "X position should still be 60.0");

        // Test wait_settled (should return quickly in mock mode)
        x_axis.wait_settled().await?;
        y_axis.wait_settled().await?;

        Ok(())
    }
}
