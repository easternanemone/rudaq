//! V4 GUI Components
//!
//! egui-based user interface components for instrument control and data visualization.

pub mod v4_data_bridge;
pub mod v4_instrument_panel;

pub use self::v4_data_bridge::V4DataBridge;
pub use self::v4_instrument_panel::V4InstrumentPanel;
