//! Neutral panel intermediate representation (G2a, bd-jcb4x.2.2a).
//!
//! Extracted from a validated manifest's inline UI hints. Carries no
//! widget-framework types, no egui, and no `hardware::config::schema` types
//! so `driver-universal` can stay narrow. Downstream renderers translate
//! it into their own widget config (G2b's `crates/ui` translator turns it
//! into `ControlPanelConfig`).
//!
//! See [`docs/explanation/v4-config-driven-ui-plan.md`](../../../docs/explanation/v4-config-driven-ui-plan.md)
//! for the full design rationale and the extractor's decision tables.

use serde::{Deserialize, Serialize};

/// A full synthesized panel description for one device.
///
/// Always produced from a validated manifest. May have zero groups when no
/// commands or parameters carry inline UI hints — that's a successful
/// "this device opted out of the synthesizer" outcome, not an error.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PanelIr {
    /// Device id this panel is for. Stringified `DeviceId` (the IR has no
    /// dependency on the runtime `DeviceId` newtype, on purpose).
    pub device_id: String,

    /// Header label shown at the top of the panel. `None` lets the
    /// renderer fall back to the device's friendly name.
    pub title: Option<String>,

    /// Ordered list of groups. Groups are the primary layout unit;
    /// widgets within the same group share a layout slot and an optional
    /// label. Order is the order each group's first widget appeared in
    /// the manifest.
    pub groups: Vec<GroupIr>,
}

/// A named cluster of widgets sharing a layout slot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupIr {
    /// Group label, when the manifest provided one via `ui.group` on a
    /// command or parameter. Ungrouped widgets share an anonymous group
    /// with `label = None`.
    pub label: Option<String>,

    /// Layout slot for this group. Defaults to `Main`. G3 introduces the
    /// `slot` field on `CommandUiHint` / `ParameterUiHint`; until then
    /// every group is `Main`.
    pub slot: LayoutSlot,

    /// Widgets in this group, in manifest declaration order.
    pub widgets: Vec<WidgetIr>,
}

/// A single widget within a group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WidgetIr {
    /// Where the widget reads/writes its value.
    pub source: ValueSource,

    /// Human-readable label. Always populated — the extractor falls back
    /// to the command/parameter name if no label is declared.
    pub label: String,

    /// Optional tooltip / help text.
    pub description: Option<String>,

    /// Optional unit suffix for numeric widgets (e.g. "mW", "deg").
    pub unit: Option<String>,

    /// The widget rendering shape.
    pub shape: WidgetShape,
}

/// What the widget reads/writes against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum ValueSource {
    /// Parameter identified by its manifest key. The widget reads/writes
    /// the parameter via the device's standard `GetParameter`/`SetParameter`
    /// path (or via explicit `read_command`/`write_command` overrides).
    Parameter(String),

    /// Command identified by its manifest key. A `Command` source is
    /// always rendered as a `WidgetShape::Button` by the translator.
    Command(String),
}

/// Widget rendering shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WidgetShape {
    /// Bounded numeric slider. Requires a min/max range; `step` is
    /// optional (renderer picks a sensible default if `None`).
    Slider {
        min: f64,
        max: f64,
        step: Option<f64>,
    },

    /// Boolean toggle / checkbox.
    Toggle,

    /// Dropdown over a fixed set of string choices.
    EnumSelect { choices: Vec<String> },

    /// Numeric text input. Range and step all optional — the renderer
    /// degrades to an unbounded numeric box when nothing is provided.
    NumericInput {
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    },

    /// Plain text line edit.
    TextInput,

    /// Action button. `confirm = Some(prompt)` makes the renderer ask the
    /// user before dispatching the command.
    Button { confirm: Option<String> },
}

/// Named layout region within a `ConfigDrivenPanel`.
///
/// G2a treats every group as `Main` because the inline UI schema does not
/// yet carry a `slot` field. G3 adds `slot` to `CommandUiHint` /
/// `ParameterUiHint` and updates the extractor accordingly.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutSlot {
    /// Top-left grid region.
    TopLeft,
    /// Bottom-right grid region.
    BottomRight,
    /// Main central region. Default for any group whose widgets do not
    /// declare a slot.
    #[default]
    Main,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ir_roundtrips_through_json() {
        let ir = PanelIr {
            device_id: "test_device".to_string(),
            title: Some("Test".to_string()),
            groups: vec![GroupIr {
                label: Some("Power".to_string()),
                slot: LayoutSlot::Main,
                widgets: vec![WidgetIr {
                    source: ValueSource::Parameter("power_pct".to_string()),
                    label: "Power".to_string(),
                    description: None,
                    unit: Some("%".to_string()),
                    shape: WidgetShape::Slider {
                        min: 0.0,
                        max: 100.0,
                        step: Some(1.0),
                    },
                }],
            }],
        };

        let json = serde_json::to_string(&ir).expect("serialize");
        let back: PanelIr = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(ir, back);
    }

    #[test]
    fn layout_slot_defaults_to_main() {
        assert_eq!(LayoutSlot::default(), LayoutSlot::Main);
    }
}
