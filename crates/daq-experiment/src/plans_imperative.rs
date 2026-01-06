//! Imperative Plan - Wrapper for legacy direct hardware commands (bd-94zq.4)
//!
//! This module provides `ImperativePlan`, a Plan implementation that wraps
//! direct hardware commands. This allows legacy imperative-style scripts
//! to still work while emitting proper Documents through the RunEngine.
//!
//! # Migration Path
//!
//! **Before (v0.6.x - no Documents):**
//! ```rhai
//! stage.move_abs(10.0);  // Direct hardware call
//! camera.trigger();      // No reproducibility
//! ```
//!
//! **After (v0.7.0+ - Documents emitted):**
//! ```rhai
//! // Internally becomes:
//! yield ImperativePlan([MoveTo("stage", 10.0)])
//! yield ImperativePlan([Trigger("camera")])
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use daq_experiment::plans_imperative::ImperativePlan;
//! use daq_experiment::plans::PlanCommand;
//!
//! // Wrap a single move command
//! let plan = ImperativePlan::move_to("stage_x", 10.0);
//!
//! // Wrap a trigger command
//! let plan = ImperativePlan::trigger("camera");
//!
//! // Wrap multiple commands
//! let plan = ImperativePlan::new(vec![
//!     PlanCommand::MoveTo { device_id: "stage_x".into(), position: 10.0 },
//!     PlanCommand::Wait { seconds: 0.1 },
//!     PlanCommand::Trigger { device_id: "camera".into() },
//!     PlanCommand::Read { device_id: "camera".into() },
//! ]).with_emit_event(true);
//! ```

use std::collections::HashMap;

use super::plans::{Plan, PlanCommand};

/// A plan that wraps imperative commands for RunEngine execution
///
/// This bridges the gap between legacy direct hardware calls and the
/// declarative plan system. Each imperative command gets wrapped in
/// an ImperativePlan, ensuring Documents are emitted.
#[derive(Debug, Clone)]
pub struct ImperativePlan {
    /// The commands to execute
    commands: Vec<PlanCommand>,
    /// Current command index
    current_idx: usize,
    /// Whether to emit an event after all commands
    emit_event: bool,
    /// Have we emitted the event yet?
    event_emitted: bool,
    /// Device ID for emitted event (if any)
    primary_device: Option<String>,
}

impl ImperativePlan {
    /// Create a new ImperativePlan with the given commands
    pub fn new(commands: Vec<PlanCommand>) -> Self {
        // Try to infer primary device from first command
        let primary_device = commands.first().and_then(|cmd| match cmd {
            PlanCommand::MoveTo { device_id, .. } => Some(device_id.clone()),
            PlanCommand::Read { device_id } => Some(device_id.clone()),
            PlanCommand::Trigger { device_id } => Some(device_id.clone()),
            PlanCommand::Set { device_id, .. } => Some(device_id.clone()),
            _ => None,
        });

        Self {
            commands,
            current_idx: 0,
            emit_event: false,
            event_emitted: false,
            primary_device,
        }
    }

    /// Create an ImperativePlan for a single move command
    pub fn move_to(device_id: impl Into<String>, position: f64) -> Self {
        let device = device_id.into();
        Self::new(vec![PlanCommand::MoveTo {
            device_id: device.clone(),
            position,
        }])
        .with_primary_device(device)
    }

    /// Create an ImperativePlan for a single read command
    pub fn read(device_id: impl Into<String>) -> Self {
        let device = device_id.into();
        Self::new(vec![PlanCommand::Read {
            device_id: device.clone(),
        }])
        .with_primary_device(device)
        .with_emit_event(true) // Reads typically want event emission
    }

    /// Create an ImperativePlan for a single trigger command
    pub fn trigger(device_id: impl Into<String>) -> Self {
        let device = device_id.into();
        Self::new(vec![PlanCommand::Trigger {
            device_id: device.clone(),
        }])
        .with_primary_device(device)
    }

    /// Create an ImperativePlan for a wait command
    pub fn wait(seconds: f64) -> Self {
        Self::new(vec![PlanCommand::Wait { seconds }])
    }

    /// Create an ImperativePlan for a parameter set command
    pub fn set_parameter(
        device_id: impl Into<String>,
        parameter: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let device = device_id.into();
        Self::new(vec![PlanCommand::Set {
            device_id: device.clone(),
            parameter: parameter.into(),
            value: value.into(),
        }])
        .with_primary_device(device)
    }

    /// Set whether to emit an event after commands complete
    pub fn with_emit_event(mut self, emit: bool) -> Self {
        self.emit_event = emit;
        self
    }

    /// Set the primary device for event metadata
    pub fn with_primary_device(mut self, device: impl Into<String>) -> Self {
        self.primary_device = Some(device.into());
        self
    }

    /// Add a command to the plan
    pub fn add_command(mut self, command: PlanCommand) -> Self {
        self.commands.push(command);
        self
    }

    /// Add a move command
    pub fn then_move(self, device_id: impl Into<String>, position: f64) -> Self {
        self.add_command(PlanCommand::MoveTo {
            device_id: device_id.into(),
            position,
        })
    }

    /// Add a read command
    pub fn then_read(self, device_id: impl Into<String>) -> Self {
        self.add_command(PlanCommand::Read {
            device_id: device_id.into(),
        })
    }

    /// Add a trigger command
    pub fn then_trigger(self, device_id: impl Into<String>) -> Self {
        self.add_command(PlanCommand::Trigger {
            device_id: device_id.into(),
        })
    }

    /// Add a wait command
    pub fn then_wait(self, seconds: f64) -> Self {
        self.add_command(PlanCommand::Wait { seconds })
    }
}

impl Plan for ImperativePlan {
    fn plan_type(&self) -> &str {
        "imperative"
    }

    fn plan_name(&self) -> &str {
        "Imperative Commands"
    }

    fn plan_args(&self) -> HashMap<String, String> {
        let mut args = HashMap::new();
        args.insert("num_commands".to_string(), self.commands.len().to_string());
        if let Some(ref device) = self.primary_device {
            args.insert("primary_device".to_string(), device.clone());
        }
        args
    }

    fn movers(&self) -> Vec<String> {
        self.commands
            .iter()
            .filter_map(|cmd| match cmd {
                PlanCommand::MoveTo { device_id, .. } => Some(device_id.clone()),
                _ => None,
            })
            .collect()
    }

    fn detectors(&self) -> Vec<String> {
        self.commands
            .iter()
            .filter_map(|cmd| match cmd {
                PlanCommand::Read { device_id } => Some(device_id.clone()),
                PlanCommand::Trigger { device_id } => Some(device_id.clone()),
                _ => None,
            })
            .collect()
    }

    fn num_points(&self) -> usize {
        // Imperative plans are typically single-shot
        if self.emit_event {
            1
        } else {
            0
        }
    }

    fn next_command(&mut self) -> Option<PlanCommand> {
        // First, yield all commands
        if self.current_idx < self.commands.len() {
            let cmd = self.commands[self.current_idx].clone();
            self.current_idx += 1;
            return Some(cmd);
        }

        // Then emit event if configured and not yet emitted
        if self.emit_event && !self.event_emitted {
            self.event_emitted = true;

            // Build positions from any MoveTo commands
            let positions: HashMap<String, f64> = self
                .commands
                .iter()
                .filter_map(|cmd| match cmd {
                    PlanCommand::MoveTo {
                        device_id,
                        position,
                    } => Some((device_id.clone(), *position)),
                    _ => None,
                })
                .collect();

            return Some(PlanCommand::EmitEvent {
                stream: "primary".to_string(),
                data: HashMap::new(), // Data will be filled by RunEngine from reads
                positions,
            });
        }

        None
    }

    fn reset(&mut self) {
        self.current_idx = 0;
        self.event_emitted = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_imperative_move_to() {
        let mut plan = ImperativePlan::move_to("stage_x", 10.0);

        assert_eq!(plan.plan_type(), "imperative");
        assert_eq!(plan.movers(), vec!["stage_x".to_string()]);

        // Should yield one MoveTo command
        let cmd = plan.next_command();
        assert!(matches!(cmd, Some(PlanCommand::MoveTo { position, .. }) if position == 10.0));

        // No more commands (emit_event is false by default for move_to)
        assert!(plan.next_command().is_none());
    }

    #[test]
    fn test_imperative_read_with_event() {
        let mut plan = ImperativePlan::read("power_meter");

        assert_eq!(plan.detectors(), vec!["power_meter".to_string()]);
        assert_eq!(plan.num_points(), 1); // emit_event is true

        // Should yield Read command
        let cmd = plan.next_command();
        assert!(matches!(cmd, Some(PlanCommand::Read { device_id }) if device_id == "power_meter"));

        // Then EmitEvent
        let cmd = plan.next_command();
        assert!(matches!(cmd, Some(PlanCommand::EmitEvent { .. })));

        // No more commands
        assert!(plan.next_command().is_none());
    }

    #[test]
    fn test_imperative_chain() {
        let mut plan = ImperativePlan::move_to("stage_x", 5.0)
            .then_wait(0.1)
            .then_trigger("camera")
            .then_read("camera")
            .with_emit_event(true);

        // Count commands
        let mut cmd_count = 0;
        while plan.next_command().is_some() {
            cmd_count += 1;
        }

        // 4 commands + 1 EmitEvent
        assert_eq!(cmd_count, 5);
    }

    #[test]
    fn test_imperative_reset() {
        let mut plan = ImperativePlan::move_to("stage_x", 10.0).with_emit_event(true);

        // Run through once
        while plan.next_command().is_some() {}

        // Reset
        plan.reset();

        // Should be able to run again
        assert!(plan.next_command().is_some());
    }

    #[test]
    fn test_imperative_set_parameter() {
        let mut plan = ImperativePlan::set_parameter("laser", "wavelength", "800.0");

        let cmd = plan.next_command();
        assert!(matches!(
            cmd,
            Some(PlanCommand::Set { device_id, parameter, value })
            if device_id == "laser" && parameter == "wavelength" && value == "800.0"
        ));
    }
}
