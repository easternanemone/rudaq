//! Reusable UI widgets for the DAQ GUI.
//!
//! This module contains parameter editors and other UI components
//! that can be shared across different panels.

pub mod parameter_editor;
pub mod pp_editor;
pub mod smart_stream_editor;
pub mod roi_selector;
pub mod histogram;

pub use parameter_editor::*;
pub use pp_editor::*;
pub use smart_stream_editor::*;
pub use roi_selector::*;
pub use histogram::*;
