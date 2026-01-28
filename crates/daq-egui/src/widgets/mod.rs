//! Reusable UI widgets for the DAQ GUI.
//!
//! This module contains parameter editors and other UI components
//! that can be shared across different panels.

pub mod adaptive_alert;
pub mod auto_scale_plot;
pub mod device_controls;
pub mod device_selector;
pub mod double_slider;
pub mod gauge;
pub mod histogram;
pub mod metadata_editor;
pub mod node_palette;
pub mod offline_notice;
pub mod parameter_editor;
pub mod pp_editor;
pub mod property_inspector;
pub mod roi_selector;
pub mod runtime_parameter_editor;
pub mod smart_stream_editor;
pub mod status_bar;
pub mod toast;
pub mod toggle;

#[allow(unused_imports)]
pub use auto_scale_plot::{AutoScalePlot, AxisLockState};
pub use device_controls::{
    AnalogOutputControlPanel, DeviceControlWidget, MaiTaiControlPanel, PowerMeterControlPanel,
    RotatorControlPanel, StageControlPanel,
};
#[allow(unused_imports)]
pub use device_selector::DeviceSelector;
pub use double_slider::{double_slider, DoubleSlider};
pub use gauge::*;
pub use histogram::*;
pub use metadata_editor::MetadataEditor;
#[allow(unused_imports)]
pub use node_palette::{NodePalette, NodeType};
pub use offline_notice::*;
pub use parameter_editor::*;
pub use pp_editor::*;
#[allow(unused_imports)]
pub use property_inspector::PropertyInspector;
pub use roi_selector::*;
#[allow(unused_imports)]
pub use runtime_parameter_editor::{
    EditableParameter, ParameterType, RuntimeParameterEditResult, RuntimeParameterEditor,
};
pub use smart_stream_editor::*;
pub use status_bar::*;
#[allow(unused_imports)]
pub use toast::*;

#[allow(unused_imports)]
pub use adaptive_alert::{show_adaptive_alert, AdaptiveAlertData, AdaptiveAlertResponse};
