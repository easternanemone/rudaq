//! DeviceIntrospection trait implementation for Comedi devices.

use anyhow::{Context, Result};
use async_trait::async_trait;
use common::capabilities::{DeviceIntrospection, DeviceIntrospectionInfo, IntrospectedSubdevice};

use crate::device::{ComediDevice, SubdeviceType};

/// A wrapper that implements [`DeviceIntrospection`] for a Comedi device.
///
/// Maps the Comedi-specific board name, driver name, and subdevice info
/// into the common crate's generic [`DeviceIntrospectionInfo`] type.
///
/// All FFI calls are dispatched via `spawn_blocking` since the Comedi
/// library is not async-safe.
pub struct IntrospectableDevice {
    device: ComediDevice,
}

impl IntrospectableDevice {
    /// Create a new introspectable wrapper around a Comedi device.
    pub fn new(device: ComediDevice) -> Self {
        Self { device }
    }
}

/// Map a Comedi [`SubdeviceType`] to a human-readable string.
fn subdevice_type_str(t: SubdeviceType) -> &'static str {
    match t {
        SubdeviceType::Unused => "unused",
        SubdeviceType::AnalogInput => "analog_input",
        SubdeviceType::AnalogOutput => "analog_output",
        SubdeviceType::DigitalInput => "digital_input",
        SubdeviceType::DigitalOutput => "digital_output",
        SubdeviceType::DigitalIO => "digital_io",
        SubdeviceType::Counter => "counter",
        SubdeviceType::Timer => "timer",
        SubdeviceType::Memory => "memory",
        SubdeviceType::Calibration => "calibration",
        SubdeviceType::Processor => "processor",
        SubdeviceType::Serial => "serial",
        SubdeviceType::Pwm => "pwm",
    }
}

#[async_trait]
impl DeviceIntrospection for IntrospectableDevice {
    async fn introspect(&self) -> Result<DeviceIntrospectionInfo> {
        let device = self.device.clone();

        tokio::task::spawn_blocking(move || {
            let info = device
                .info()
                .context("Failed to query Comedi device info")?;

            let subdevices = info
                .subdevices
                .iter()
                .map(|sub| IntrospectedSubdevice {
                    index: sub.index,
                    subdevice_type: subdevice_type_str(sub.subdev_type).to_string(),
                    n_channels: sub.n_channels,
                    n_ranges: sub.n_ranges,
                })
                .collect();

            Ok(DeviceIntrospectionInfo {
                board_name: info.board_name,
                driver_name: info.driver_name,
                subdevices,
            })
        })
        .await
        .context("Task join error")?
    }
}
