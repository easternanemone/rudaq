//! Device-specific control panel widgets.
//!
//! This module provides specialized control panels for different device types,
//! including lasers, power meters, rotators, and stages.

mod maitai_panel;
mod power_meter_panel;
mod rotator_panel;
mod stage_panel;

pub use maitai_panel::MaiTaiControlPanel;
pub use power_meter_panel::PowerMeterControlPanel;
pub use rotator_panel::RotatorControlPanel;
pub use stage_panel::StageControlPanel;

use egui::Ui;
use tokio::runtime::Runtime;

use crate::client::DaqClient;
use daq_proto::daq::DeviceInfo;

/// Trait for device-specific control panel widgets
pub trait DeviceControlWidget {
    /// Render the control panel UI
    ///
    /// # Arguments
    /// * `ui` - egui UI context
    /// * `device` - Device info from the daemon
    /// * `client` - Optional gRPC client for making requests
    /// * `runtime` - Tokio runtime for async operations
    fn ui(
        &mut self,
        ui: &mut Ui,
        device: &DeviceInfo,
        client: Option<&mut DaqClient>,
        runtime: &Runtime,
    );

    /// Return the device type this widget handles
    fn device_type(&self) -> &'static str;
}
