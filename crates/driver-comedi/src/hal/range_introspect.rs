//! RangeIntrospectable trait implementation for Comedi subsystems.
//!
//! Provides analog range introspection for both input and output subsystems,
//! enabling generic range queries through the capability trait system (bd-3bjp).

use anyhow::{Context, Result};
use async_trait::async_trait;
use common::capabilities::{AnalogRange, RangeIntrospectable, RangeUnit};

use crate::subsystem::analog_input::AnalogInput;
use crate::subsystem::analog_output::AnalogOutput;

/// Maps a Comedi range unit code to the common crate's [`RangeUnit`].
///
/// Comedi uses: 0 = volts, 1 = milliamps, 2 = none.
fn comedi_unit_to_range_unit(unit: u32) -> RangeUnit {
    match unit {
        0 => RangeUnit::Volts,
        1 => RangeUnit::MilliAmps,
        _ => RangeUnit::Volts, // conservative default
    }
}

/// Wraps a Comedi analog input subsystem to implement [`RangeIntrospectable`].
///
/// Delegates to [`AnalogInput::n_ranges`] and [`AnalogInput::range_info`],
/// running the blocking FFI calls inside `spawn_blocking`.
pub struct RangeIntrospectableAnalogInput {
    inner: AnalogInput,
}

impl RangeIntrospectableAnalogInput {
    /// Create a new range-introspectable wrapper for an analog input subsystem.
    pub fn new(inner: AnalogInput) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl RangeIntrospectable for RangeIntrospectableAnalogInput {
    async fn get_ranges(&self, subdevice: u32) -> Result<Vec<AnalogRange>> {
        let ai = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let ranges = ai
                .ranges(subdevice)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("Failed to get analog input ranges")?;
            Ok(ranges
                .into_iter()
                .map(|r| AnalogRange {
                    index: r.index,
                    min: r.min,
                    max: r.max,
                    unit: comedi_unit_to_range_unit(r.unit),
                })
                .collect())
        })
        .await
        .context("Task join error")?
    }
}

/// Wraps a Comedi analog output subsystem to implement [`RangeIntrospectable`].
///
/// Delegates to [`AnalogOutput::n_ranges`] and [`AnalogOutput::range_info`],
/// running the blocking FFI calls inside `spawn_blocking`.
pub struct RangeIntrospectableAnalogOutput {
    inner: AnalogOutput,
}

impl RangeIntrospectableAnalogOutput {
    /// Create a new range-introspectable wrapper for an analog output subsystem.
    pub fn new(inner: AnalogOutput) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl RangeIntrospectable for RangeIntrospectableAnalogOutput {
    async fn get_ranges(&self, subdevice: u32) -> Result<Vec<AnalogRange>> {
        let ao = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let ranges = ao
                .ranges(subdevice)
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("Failed to get analog output ranges")?;
            Ok(ranges
                .into_iter()
                .map(|r| AnalogRange {
                    index: r.index,
                    min: r.min,
                    max: r.max,
                    unit: comedi_unit_to_range_unit(r.unit),
                })
                .collect())
        })
        .await
        .context("Task join error")?
    }
}
