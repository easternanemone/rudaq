//! Comedi DAQ Control Panels
//!
//! This module provides control panels for Comedi-based DAQ hardware,
//! specifically targeting the NI PCI-MIO-16XE-10 and similar multifunction cards.
//!
//! # Control Panels
//!
//! - [`AnalogInputPanel`] - 16-channel AI with voltage range selection
//! - [`AnalogOutputPanel`] - 2-channel DAC control with waveform generation
//! - [`DigitalIOPanel`] - 24-channel DIO with per-pin direction control
//! - [`CounterPanel`] - 3-channel counter/timer control
//! - [`ComediPanel`] - Unified control panel with tabbed interface
//! - [`TriggerConfigPanel`] - Hardware trigger and timing configuration
//!
//! # Viewer Panels
//!
//! - [`OscilloscopePanel`] - Real-time waveform visualization with triggering
//! - [`VoltmeterPanel`] - DMM-style digital voltmeter display
//! - [`DioMonitorPanel`] - LED-style DIO state visualization
//! - [`CounterDisplayPanel`] - Large counter value display with rate
//! - [`DataLoggerPanel`] - Scrolling data table with export
//!
//! # Note
//!
//! These panels are work-in-progress and not yet integrated into the main UI.
//! They are retained for future use when the Comedi gRPC interface is complete.

// Comedi panels are under development; blanket allow justified until gRPC integration (bd-m5fh.4.2).
#![allow(dead_code)]

mod analog_input;
mod analog_output;
mod counter;
mod counter_display;
mod data_logger;
mod digital_io;
mod dio_monitor;
mod oscilloscope;
mod trigger;
mod unified;
mod voltmeter;

// WIP: These panels are not yet integrated. Uncomment when Comedi gRPC is complete.
#[allow(unused_imports)]
pub use analog_input::AnalogInputPanel;
#[allow(unused_imports)]
pub use analog_output::AnalogOutputPanel;
#[allow(unused_imports)]
pub use counter::CounterPanel;
#[allow(unused_imports)]
pub use counter_display::{
    counter_display_channel, CounterDisplayPanel, CounterDisplayReceiver, CounterDisplaySender,
    CounterUpdate,
};
#[allow(unused_imports)]
pub use data_logger::{
    data_logger_channel, DataLoggerPanel, DataLoggerReceiver, DataLoggerSender, LogEntry,
};
#[allow(unused_imports)]
pub use digital_io::DigitalIOPanel;
#[allow(unused_imports)]
pub use dio_monitor::{
    dio_monitor_channel, DioMonitorPanel, DioMonitorReceiver, DioMonitorSender, DioStateUpdate,
};
#[allow(unused_imports)]
pub use oscilloscope::{
    oscilloscope_channel, OscilloscopePanel, OscilloscopeReceiver, OscilloscopeSample,
    OscilloscopeSender, SignalSource, SyntheticSignal, TriggerEdge, TriggerMode,
};
#[allow(unused_imports)]
pub use trigger::{
    ClockSource, SubsystemTriggerConfig, SyncMode, TriggerConfigPanel, TriggerPolarity,
    TriggerSource,
};
#[allow(unused_imports)]
pub use unified::ComediPanel;
#[allow(unused_imports)]
pub use voltmeter::{
    voltmeter_channel, VoltmeterPanel, VoltmeterReading, VoltmeterReceiver, VoltmeterSender,
};

use serde::{Deserialize, Serialize};

/// Voltage range configuration for analog channels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoltageRange {
    pub index: u32,
    pub min: f64,
    pub max: f64,
}

impl Default for VoltageRange {
    fn default() -> Self {
        Self {
            index: 0,
            min: -10.0,
            max: 10.0,
        }
    }
}

impl VoltageRange {
    /// Create a new voltage range.
    pub const fn new(index: u32, min: f64, max: f64) -> Self {
        Self { index, min, max }
    }

    /// Get the span of the range.
    pub fn span(&self) -> f64 {
        self.max - self.min
    }

    /// Format as display string.
    pub fn label(&self) -> String {
        format_voltage_range(self)
    }
}

/// Common voltage ranges for NI PCI-MIO-16XE-10.
pub const NI_VOLTAGE_RANGES: &[VoltageRange] = &[
    VoltageRange::new(0, -10.0, 10.0),
    VoltageRange::new(1, -5.0, 5.0),
    VoltageRange::new(2, -2.5, 2.5),
    VoltageRange::new(3, -1.0, 1.0),
    VoltageRange::new(4, -0.5, 0.5),
    VoltageRange::new(5, -0.25, 0.25),
    VoltageRange::new(6, -0.1, 0.1),
    VoltageRange::new(7, 0.0, 10.0),
    VoltageRange::new(8, 0.0, 5.0),
    VoltageRange::new(9, 0.0, 2.5),
    VoltageRange::new(10, 0.0, 1.0),
    VoltageRange::new(11, 0.0, 0.5),
    VoltageRange::new(12, 0.0, 0.25),
    VoltageRange::new(13, 0.0, 0.1),
];

/// Format a voltage range as a display string.
pub fn format_voltage_range(range: &VoltageRange) -> String {
    if range.min >= 0.0 {
        format!("0 to {:+.2}V", range.max)
    } else {
        format!("{:+.2}V to {:+.2}V", range.min, range.max)
    }
}

/// Analog reference type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AnalogReference {
    /// Ground-referenced single-ended
    #[default]
    Ground,
    /// Common-referenced single-ended
    Common,
    /// Differential
    Differential,
    /// Other reference type
    Other,
}

impl AnalogReference {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ground => "Ground (RSE)",
            Self::Common => "Common",
            Self::Differential => "Differential",
            Self::Other => "Other",
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Ground, Self::Common, Self::Differential, Self::Other]
    }
}

/// Digital I/O direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DioDirection {
    #[default]
    Input,
    Output,
}

impl DioDirection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "In",
            Self::Output => "Out",
        }
    }
}

/// Counter operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CounterMode {
    #[default]
    EventCount,
    FrequencyMeasurement,
    PeriodMeasurement,
    PulseGeneration,
    QuadratureEncoder,
    PulseWidth,
}

impl CounterMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::EventCount => "Event Count",
            Self::FrequencyMeasurement => "Frequency",
            Self::PeriodMeasurement => "Period",
            Self::PulseGeneration => "Pulse Gen",
            Self::QuadratureEncoder => "Quadrature",
            Self::PulseWidth => "Pulse Width",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::EventCount,
            Self::FrequencyMeasurement,
            Self::PeriodMeasurement,
            Self::PulseGeneration,
            Self::QuadratureEncoder,
            Self::PulseWidth,
        ]
    }
}
