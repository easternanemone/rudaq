//! Hardware command dispatch and condition evaluation.
//!
//! [`CommandDispatcher`] owns references to the `DeviceRegistry` and
//! feedback channel. It provides the pure hardware interaction layer
//! (move, read, trigger, set, evaluate condition) without access to
//! engine state (abort flags, pause, document stream). This makes the
//! hardware interaction logic testable without a full `RunEngine`.

use tokio::sync::mpsc;
use tracing::{debug, warn};

use hardware::registry::DeviceRegistry;

use crate::feedback::FeedbackEvent;
use crate::plans::EvalCondition;

/// Stateless dispatcher for hardware commands and condition evaluation.
///
/// Created by `RunEngine` at the start of plan execution and passed to
/// `process_command` for the hardware-interaction portions. The orchestration
/// loop (abort checking, pause handling, event emission) stays on `RunEngine`.
pub(crate) struct CommandDispatcher<'a> {
    pub(crate) registry: &'a DeviceRegistry,
    pub(crate) feedback_tx: &'a mpsc::Sender<FeedbackEvent>,
}

impl<'a> CommandDispatcher<'a> {
    /// Execute a move command.
    pub(crate) async fn execute_move(&self, device_id: &str, position: f64) -> anyhow::Result<()> {
        debug!(device = %device_id, position = %position, "Moving");

        let device = self.registry.get_movable(device_id);
        if let Some(device) = device {
            device.move_abs(position).await?;
        } else {
            warn!(device = %device_id, "Device not found or not movable, skipping move");
        }

        Ok(())
    }

    /// Execute a read command.
    pub(crate) async fn execute_read(&self, device_id: &str) -> anyhow::Result<f64> {
        debug!(device = %device_id, "Reading");

        let device = self.registry.get_readable(device_id);
        if let Some(device) = device {
            let value = device.read().await?;
            Ok(value)
        } else {
            warn!(device = %device_id, "Device not found or not readable, returning 0.0");
            Ok(0.0)
        }
    }

    /// Execute a trigger command.
    pub(crate) async fn execute_trigger(&self, device_id: &str) -> anyhow::Result<()> {
        debug!(device = %device_id, "Triggering");

        let device = self.registry.get_triggerable(device_id);
        if let Some(device) = device {
            device.trigger().await?;
        } else {
            debug!(device = %device_id, "Device not triggerable, skipping");
        }

        Ok(())
    }

    /// Execute a set parameter command.
    pub(crate) async fn execute_set_parameter(
        &self,
        device_id: &str,
        parameter: &str,
        value: &str,
    ) -> anyhow::Result<()> {
        debug!(device = %device_id, param = %parameter, value = %value, "Setting parameter");

        // LEGACY: Try Settable trait first for drivers that don't implement Parameterized.
        // Remove after all drivers implement Parameterized. See
        // docs/reference/deprecation-plan.md Section 3.3.
        let settable = self.registry.get_settable(device_id);
        if let Some(settable) = settable {
            let json_value: serde_json::Value = serde_json::from_str(value)
                .or_else(|_| {
                    Ok::<_, serde_json::Error>(serde_json::Value::String(value.to_string()))
                })
                .map_err(|e| anyhow::anyhow!("Invalid value format: {}", e))?;

            settable.set_value(parameter, json_value).await?;
            return Ok(());
        }

        // New path - use Parameterized trait and Parameter<T> system
        let json_value: serde_json::Value = serde_json::from_str(value)
            .or_else(|_| Ok::<_, serde_json::Error>(serde_json::Value::String(value.to_string())))
            .map_err(|e| anyhow::anyhow!("Invalid value format: {}", e))?;

        if let Some(parameterized) = self.registry.get_parameterized(device_id) {
            let params = parameterized.parameters();
            if let Some(param) = params.get(parameter) {
                param.set_json(json_value)?;
                return Ok(());
            }
            anyhow::bail!(
                "Parameter '{}' not found on device '{}'",
                parameter,
                device_id
            );
        }

        anyhow::bail!(
            "Device '{}' not found or does not support parameter setting",
            device_id
        );
    }

    /// Evaluate an `EvalCondition` by reading from the device registry.
    ///
    /// Returns `true` if the condition is satisfied, `false` otherwise.
    /// On read errors the condition evaluates to `false` and a warning is logged.
    pub(crate) async fn evaluate_condition(&self, condition: &EvalCondition) -> bool {
        match condition {
            EvalCondition::Threshold {
                device_id,
                field: _,
                threshold,
                above,
            } => {
                let Some(readable) = self.registry.get_readable(device_id) else {
                    warn!(%device_id, "evaluate_condition: device not readable");
                    return false;
                };
                match readable.read().await {
                    Ok(value) => {
                        let result = if *above {
                            value > *threshold
                        } else {
                            value < *threshold
                        };
                        if result {
                            if let Err(e) =
                                self.feedback_tx.try_send(FeedbackEvent::ThresholdCrossed {
                                    device_id: device_id.clone(),
                                    field: "value".to_string(),
                                    value,
                                    threshold: *threshold,
                                })
                            {
                                warn!("Feedback event dropped (channel full): {e}");
                            }
                        }
                        result
                    }
                    Err(e) => {
                        warn!(%device_id, error = %e, "evaluate_condition: read failed");
                        false
                    }
                }
            }
            EvalCondition::Comparison {
                left_device_id,
                left_field: _,
                right_device_id,
                right_field: _,
                operator,
            } => {
                let left = self.registry.get_readable(left_device_id);
                let right = self.registry.get_readable(right_device_id);

                let (Some(left_r), Some(right_r)) = (left, right) else {
                    warn!(
                        %left_device_id,
                        %right_device_id,
                        "evaluate_condition: one or both devices not readable"
                    );
                    return false;
                };

                let (left_val, right_val) = match (left_r.read().await, right_r.read().await) {
                    (Ok(l), Ok(r)) => (l, r),
                    (Err(e), _) | (_, Err(e)) => {
                        warn!(error = %e, "evaluate_condition: read failed");
                        return false;
                    }
                };

                operator.evaluate(left_val, right_val)
            }
        }
    }
}
