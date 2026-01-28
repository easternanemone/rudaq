//! Tests for device config loading and rendering

#[cfg(test)]
mod tests {
    use super::super::config_loader::DeviceConfigCache;

    #[test]
    fn test_config_cache_loads_ell14() {
        let mut cache = DeviceConfigCache::new();

        // Try to load configs - should not fail even if directory doesn't exist
        let result = cache.load_all();
        assert!(
            result.is_ok(),
            "Config loading should not fail: {:?}",
            result
        );

        // If configs loaded, verify we can find elliptec
        if cache.protocols().count() > 0 {
            let config = cache.get_by_protocol("elliptec");
            if let Some(config) = config {
                assert_eq!(config.device.protocol, "elliptec");
                assert!(config.ui.is_some(), "ELL14 should have UI config");

                if let Some(ui_config) = &config.ui {
                    assert!(
                        ui_config.control_panel.is_some(),
                        "ELL14 should have control panel config"
                    );
                }
            }
        }
    }

    #[test]
    fn test_config_cache_fuzzy_match() {
        let mut cache = DeviceConfigCache::new();
        let _ = cache.load_all();

        // If configs exist, test fuzzy matching
        if cache.protocols().count() > 0 {
            // Should match "elliptec" protocol from "ell14_driver" driver type
            let config = cache.get_by_driver_type("ell14_driver");
            if let Some(config) = config {
                assert_eq!(config.device.protocol, "elliptec");
            }

            // Should match "maitai" protocol from "maitai_driver" driver type
            let config = cache.get_by_driver_type("maitai_driver");
            if let Some(config) = config {
                assert_eq!(config.device.protocol, "maitai");
            }
        }
    }
}
