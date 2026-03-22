//! Mock DeviceIntrospection implementation for testing.
//!
//! Returns static metadata that mimics a simple DAQ board with a single
//! analog input subdevice, suitable for unit and integration tests.

use anyhow::Result;
use async_trait::async_trait;
use common::capabilities::{DeviceIntrospection, DeviceIntrospectionInfo, IntrospectedSubdevice};

/// Mock implementation of [`DeviceIntrospection`].
///
/// Returns a fixed `DeviceIntrospectionInfo` describing a synthetic
/// "mock_daq" board with one 16-channel analog input subdevice.
pub struct MockDeviceIntrospection;

impl MockDeviceIntrospection {
    /// Create a new mock introspection instance.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockDeviceIntrospection {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DeviceIntrospection for MockDeviceIntrospection {
    async fn introspect(&self) -> Result<DeviceIntrospectionInfo> {
        Ok(DeviceIntrospectionInfo {
            board_name: "mock_daq".to_string(),
            driver_name: "mock".to_string(),
            subdevices: vec![IntrospectedSubdevice {
                index: 0,
                subdevice_type: "analog_input".to_string(),
                n_channels: 16,
                n_ranges: 1,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_introspect_returns_expected_metadata() {
        let intro = MockDeviceIntrospection::new();
        let info = intro.introspect().await.expect("introspect should succeed");

        assert_eq!(info.board_name, "mock_daq");
        assert_eq!(info.driver_name, "mock");
        assert_eq!(info.subdevices.len(), 1);

        let sub = &info.subdevices[0];
        assert_eq!(sub.index, 0);
        assert_eq!(sub.subdevice_type, "analog_input");
        assert_eq!(sub.n_channels, 16);
        assert_eq!(sub.n_ranges, 1);
    }
}
