//! Reusable UI widgets for the DAQ GUI.
//!
//! This module contains parameter editors and other UI components
//! that can be shared across different panels.

pub mod gauge;
pub mod histogram;
pub mod offline_notice;
pub mod parameter_editor;
pub mod pp_editor;
pub mod roi_selector;
pub mod smart_stream_editor;
pub mod status_bar;
pub mod toast;
pub mod toggle;

pub use gauge::*;
pub use histogram::*;
pub use offline_notice::*;
pub use parameter_editor::*;
pub use pp_editor::*;
pub use roi_selector::*;
pub use smart_stream_editor::*;
pub use status_bar::*;
pub use toast::*;
pub use toggle::*;
