//! Extract a [`PanelIr`] from a validated manifest (G2a, bd-jcb4x.2.2a).
//!
//! The extractor is total: any [`DeviceManifest`] yields a [`PanelIr`],
//! possibly with zero groups when no inline UI hints are present. Decision
//! tables are documented in
//! [`docs/explanation/v4-config-driven-ui-plan.md`](../../../docs/explanation/v4-config-driven-ui-plan.md).
//!
//! Group order: the order each group's first widget appeared in the
//! manifest's iteration order. Within a group, widget order matches
//! iteration order. (`HashMap` iteration is non-deterministic, so callers
//! that need a stable order should sort the manifest's source TOML before
//! parsing — out of scope for the extractor.)

use crate::config::validated::{
    CommandConfig, CommandUiHint, DeviceManifest, ManifestParameterMeta, ParameterUiHint,
    WidgetHint,
};
use crate::panel_ir::{GroupIr, LayoutSlot, PanelIr, ValueSource, WidgetIr, WidgetShape};

/// Build a [`PanelIr`] from a validated manifest.
///
/// `device_id` is the stringified runtime device id; the IR carries it
/// purely as metadata for downstream consumers.
pub fn extract_panel_ir(manifest: &DeviceManifest, device_id: &str) -> PanelIr {
    let mut groups = GroupOrder::default();

    // Parameters first so that — when a parameter and a command share a
    // group name — the parameter widgets appear above the command buttons
    // in the synthesized panel. Manifest authors who care about exact order
    // can disambiguate by group naming.
    for param in &manifest.parameter_metadata {
        if let Some(hint) = &param.ui_hint {
            let widget = build_parameter_widget(param, hint);
            groups.push(hint.group.as_deref(), widget);
        }
    }

    let mut command_entries: Vec<(&String, &CommandConfig)> = manifest.commands.iter().collect();
    // Stable order across HashMap iteration for deterministic tests.
    command_entries.sort_by(|a, b| a.0.cmp(b.0));
    for (name, cmd) in command_entries {
        if let Some(hint) = &cmd.ui_hint {
            let widget = build_command_widget(name, cmd, hint);
            groups.push(hint.group.as_deref(), widget);
        }
    }

    PanelIr {
        device_id: device_id.to_string(),
        title: manifest
            .device
            .description
            .clone()
            .or_else(|| Some(manifest.device.name.clone())),
        groups: groups.into_vec(),
    }
}

/// Decision table for parameters → [`WidgetIr`]. See plan §3.3.
fn build_parameter_widget(meta: &ManifestParameterMeta, hint: &ParameterUiHint) -> WidgetIr {
    let label = hint.label.clone().unwrap_or_else(|| meta.name.clone());

    let shape = match (hint.widget_hint, meta.dtype.as_str()) {
        (Some(WidgetHint::Slider), _) => match (meta.min_value, meta.max_value) {
            (Some(min), Some(max)) => WidgetShape::Slider {
                min,
                max,
                step: hint.step,
            },
            _ => WidgetShape::NumericInput {
                min: meta.min_value,
                max: meta.max_value,
                step: hint.step,
            },
        },
        (Some(WidgetHint::Toggle), _) => WidgetShape::Toggle,
        (Some(WidgetHint::EnumSelect), _) => WidgetShape::EnumSelect {
            choices: hint.enum_values.clone(),
        },
        (Some(WidgetHint::NumericInput), _) => WidgetShape::NumericInput {
            min: meta.min_value,
            max: meta.max_value,
            step: hint.step,
        },
        (Some(WidgetHint::TextInput), _) => WidgetShape::TextInput,
        (Some(WidgetHint::Button), _) => {
            // Parameters can't be buttons; fall back to a sensible default.
            default_parameter_shape(meta, hint)
        }
        (None, _) => default_parameter_shape(meta, hint),
    };

    // ParameterUiHint has no `description` field of its own (G1 schema), so
    // the synthesized widget description always falls back to the
    // parameter's top-level description. If a future G3 schema adds
    // `[parameters.X.ui] description = "..."`, hook it in here as the
    // first arm of the chain.
    WidgetIr {
        source: ValueSource::Parameter(meta.name.clone()),
        label,
        description: meta.description.clone(),
        unit: meta.unit.clone(),
        shape,
    }
}

/// Default widget shape when `widget_hint` is `None` or invalid. See plan §3.3.
fn default_parameter_shape(meta: &ManifestParameterMeta, hint: &ParameterUiHint) -> WidgetShape {
    match meta.dtype.as_str() {
        "bool" => WidgetShape::Toggle,
        "string" if !hint.enum_values.is_empty() => WidgetShape::EnumSelect {
            choices: hint.enum_values.clone(),
        },
        "string" => WidgetShape::TextInput,
        _numeric => match (meta.min_value, meta.max_value) {
            (Some(min), Some(max)) => WidgetShape::Slider {
                min,
                max,
                step: hint.step,
            },
            _ => WidgetShape::NumericInput {
                min: meta.min_value,
                max: meta.max_value,
                step: hint.step,
            },
        },
    }
}

/// Decision table for commands → [`WidgetIr`]. See plan §3.2.
fn build_command_widget(name: &str, cmd: &CommandConfig, hint: &CommandUiHint) -> WidgetIr {
    // Commands always render as buttons in v4. `widget_hint = Some(other)` is
    // accepted but folds back to a button — the alternative would be to
    // refuse the manifest at validation time, which is too aggressive for
    // a forward-looking enum.
    let confirm = None; // Reserved for a future `confirm` field on CommandUiHint.

    WidgetIr {
        source: ValueSource::Command(name.to_string()),
        label: hint.label.clone().unwrap_or_else(|| name.to_string()),
        description: hint.description.clone().or_else(|| cmd.description.clone()),
        unit: None,
        shape: WidgetShape::Button { confirm },
    }
}

/// Insertion-order group accumulator.
#[derive(Default)]
struct GroupOrder {
    /// (label_key, slot, widgets) — `label_key` is `None` for the
    /// anonymous group, `Some(name)` for named groups. Anonymous group
    /// shows up at most once.
    entries: Vec<(Option<String>, LayoutSlot, Vec<WidgetIr>)>,
}

impl GroupOrder {
    fn push(&mut self, label: Option<&str>, widget: WidgetIr) {
        let key = label.map(|s| s.to_string());
        if let Some(slot) = self.entries.iter_mut().find(|(k, _, _)| k == &key) {
            slot.2.push(widget);
        } else {
            self.entries.push((key, LayoutSlot::Main, vec![widget]));
        }
    }

    fn into_vec(self) -> Vec<GroupIr> {
        self.entries
            .into_iter()
            .map(|(label, slot, widgets)| GroupIr {
                label,
                slot,
                widgets,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse::parse_manifest;
    use crate::config::raw::RawManifest;

    fn manifest(src: &str) -> DeviceManifest {
        let raw: RawManifest = toml::from_str(src).expect("manifest TOML should parse");
        parse_manifest(raw).expect("manifest should validate")
    }

    fn must_have_widget<'a>(
        ir: &'a PanelIr,
        group_label: Option<&str>,
        widget_label: &str,
    ) -> &'a WidgetIr {
        let group = ir
            .groups
            .iter()
            .find(|g| g.label.as_deref() == group_label)
            .unwrap_or_else(|| panic!("group {:?} missing", group_label));
        group
            .widgets
            .iter()
            .find(|w| w.label == widget_label)
            .unwrap_or_else(|| panic!("widget {widget_label:?} missing in group {:?}", group_label))
    }

    #[test]
    fn empty_manifest_yields_no_groups() {
        let ir = extract_panel_ir(&manifest(MINIMAL_MANIFEST), "test/dev");
        assert!(ir.groups.is_empty());
        assert_eq!(ir.device_id, "test/dev");
    }

    #[test]
    fn parameter_with_range_defaults_to_slider() {
        let ir = extract_panel_ir(&manifest(SLIDER_PARAMETER_MANIFEST), "dev");
        let w = must_have_widget(&ir, Some("Power"), "Power");
        match &w.shape {
            WidgetShape::Slider { min, max, .. } => {
                assert!((min - 0.0).abs() < 1e-9);
                assert!((max - 100.0).abs() < 1e-9);
            }
            other => panic!("expected Slider, got {other:?}"),
        }
        assert_eq!(w.unit.as_deref(), Some("%"));
    }

    #[test]
    fn parameter_without_range_defaults_to_numeric_input() {
        let ir = extract_panel_ir(&manifest(NUMERIC_NORANGE_MANIFEST), "dev");
        let w = must_have_widget(&ir, None, "freq_hz");
        assert!(matches!(w.shape, WidgetShape::NumericInput { .. }));
    }

    #[test]
    fn bool_parameter_defaults_to_toggle() {
        let ir = extract_panel_ir(&manifest(BOOL_PARAMETER_MANIFEST), "dev");
        let w = must_have_widget(&ir, None, "emission");
        assert!(matches!(w.shape, WidgetShape::Toggle));
    }

    #[test]
    fn string_with_enum_values_picks_dropdown() {
        let ir = extract_panel_ir(&manifest(ENUM_PARAMETER_MANIFEST), "dev");
        let w = must_have_widget(&ir, None, "mode");
        match &w.shape {
            WidgetShape::EnumSelect { choices } => {
                assert_eq!(
                    choices,
                    &vec!["off".to_string(), "low".to_string(), "high".to_string()]
                );
            }
            other => panic!("expected EnumSelect, got {other:?}"),
        }
    }

    #[test]
    fn string_without_enum_values_falls_to_text_input() {
        let ir = extract_panel_ir(&manifest(STRING_PARAMETER_MANIFEST), "dev");
        let w = must_have_widget(&ir, None, "label");
        assert!(matches!(w.shape, WidgetShape::TextInput));
    }

    #[test]
    fn command_with_ui_renders_as_button() {
        let ir = extract_panel_ir(&manifest(COMMAND_BUTTON_MANIFEST), "dev");
        let w = must_have_widget(&ir, Some("Actions"), "Reset");
        assert!(matches!(w.shape, WidgetShape::Button { confirm: None }));
        assert_eq!(w.source, ValueSource::Command("reset".to_string()));
    }

    #[test]
    fn explicit_widget_hint_overrides_default() {
        let ir = extract_panel_ir(&manifest(EXPLICIT_TEXT_INPUT_MANIFEST), "dev");
        let w = must_have_widget(&ir, None, "device_label");
        assert!(matches!(w.shape, WidgetShape::TextInput));
    }

    #[test]
    fn parameter_without_ui_hint_is_omitted() {
        let ir = extract_panel_ir(&manifest(MIXED_HINT_MANIFEST), "dev");
        // Two parameters declared, only one has [.ui]; the IR only contains the annotated one.
        let total_widgets: usize = ir.groups.iter().map(|g| g.widgets.len()).sum();
        assert_eq!(total_widgets, 1);
    }

    // --- Fixture manifests ---
    //
    // These are synthetic — they exercise extractor decisions without
    // dragging in the schema details of any real device. Every fixture
    // includes the bare minimum manifest to satisfy parser invariants:
    // a [device] block, a [connection] (mock), and any commands a
    // capability needs.

    const MINIMAL_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "minimal"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025
"#;

    const SLIDER_PARAMETER_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "slider_dev"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025

[parameters.power_pct]
type = "float"
default = 50.0
range = [0.0, 100.0]
unit = "%"

[parameters.power_pct.ui]
label = "Power"
group = "Power"
"#;

    const NUMERIC_NORANGE_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "numeric_dev"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025

[parameters.freq_hz]
type = "float"
default = 1000.0

[parameters.freq_hz.ui]
"#;

    const BOOL_PARAMETER_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "bool_dev"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025

[parameters.emission]
type = "bool"
default = false

[parameters.emission.ui]
"#;

    const ENUM_PARAMETER_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "enum_dev"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025

[parameters.mode]
type = "string"
default = "off"

[parameters.mode.ui]
enum_values = ["off", "low", "high"]
"#;

    const STRING_PARAMETER_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "string_dev"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025

[parameters.label]
type = "string"
default = ""

[parameters.label.ui]
"#;

    const COMMAND_BUTTON_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "cmd_dev"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025

[commands.reset]
template = "*RST"
expects_response = false

[commands.reset.ui]
label = "Reset"
group = "Actions"
"#;

    const EXPLICIT_TEXT_INPUT_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "explicit_dev"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025

[parameters.device_label]
type = "string"
default = ""

[parameters.device_label.ui]
widget = "text_input"
"#;

    const MIXED_HINT_MANIFEST: &str = r#"
schema_version = 3

[device]
name = "mixed_dev"

[connection]
type = "tcp"
host = "127.0.0.1"
port = 5025

[parameters.with_ui]
type = "float"
default = 1.0
range = [0.0, 10.0]

[parameters.with_ui.ui]
label = "Annotated"

[parameters.no_ui]
type = "float"
default = 2.0
"#;
}
