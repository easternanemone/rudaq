//! Translate a [`PanelIr`] into a [`ControlPanelConfig`] (G2b, bd-jcb4x.2.2b).
//!
//! Half of the option-(c) crate boundary: `driver-universal` produces the
//! IR with [`extract_panel_ir`](driver_universal::panel_extract::extract_panel_ir),
//! and this module turns it into the schema-typed widget config that
//! [`ConfigDrivenPanel`](super::config_renderer::ConfigDrivenPanel) renders.
//!
//! See [`docs/explanation/v4-config-driven-ui-plan.md`](../../../../../docs/explanation/v4-config-driven-ui-plan.md)
//! §4 for the full mapping table.
//!
//! The translator is **total**: any IR yields a config. Unrenderable
//! widgets (e.g. a `Toggle` shape with a `Command` source — buttons can't
//! toggle) are skipped and logged via `tracing::warn!`.

use driver_universal::panel_ir::{GroupIr, PanelIr, ValueSource, WidgetIr, WidgetShape};
use hardware::config::schema::{
    ButtonStyle, ControlPanelConfig, ControlSection, CustomActionSectionConfig, PanelLayout,
    ParameterSectionConfig, ParameterWidget, SeparatorConfig,
};

/// Visible spacer height (px) emitted before a labeled group's widgets.
/// Group label rendering itself comes in G3 with the layout-slot work; for
/// now we just keep visual breathing room between groups.
const GROUP_SEPARATOR_HEIGHT: u8 = 8;

/// Build a [`ControlPanelConfig`] from a panel IR.
///
/// Group labels become `Separator` sections preceding their widgets, so
/// the rendered panel has visible group headers. Widgets within a group
/// follow in declaration order.
//
// G2c wires this into `PanelRegistry::render_body`. Until then it is dead
// code; the `#[allow(dead_code)]` is removed in that PR.
#[allow(dead_code)]
pub(crate) fn synthesize_config(ir: &PanelIr) -> ControlPanelConfig {
    let mut sections: Vec<ControlSection> = Vec::new();

    for group in &ir.groups {
        emit_group(group, &mut sections);
    }

    ControlPanelConfig {
        layout: PanelLayout::Vertical,
        sections,
        // 0 = let the renderer pick a sensible default.
        width: 0,
        show_header: true,
        collapsible: false,
    }
}

#[allow(dead_code)]
fn emit_group(group: &GroupIr, sections: &mut Vec<ControlSection>) {
    // Visual spacer before each labeled group. The schema's
    // `SeparatorConfig` has no label field — group titles will land in G3
    // when named layout slots arrive.
    if let Some(label) = &group.label
        && !label.is_empty()
        && !sections.is_empty()
    {
        sections.push(ControlSection::Separator(SeparatorConfig {
            height: GROUP_SEPARATOR_HEIGHT,
            visible: true,
        }));
    }

    for widget in &group.widgets {
        if let Some(section) = widget_to_section(widget, group.label.as_deref()) {
            sections.push(section);
        } else {
            tracing::warn!(
                widget = ?widget.label,
                source = ?widget.source,
                shape = ?widget.shape,
                "synthesizer: skipping unrenderable widget shape/source pair"
            );
        }
    }
}

#[allow(dead_code)]
fn widget_to_section(widget: &WidgetIr, group_label: Option<&str>) -> Option<ControlSection> {
    // Prefix labeled groups onto the per-widget label so the user sees
    // group context even though SeparatorConfig has no title field. G3
    // replaces this with proper slot-based layout.
    let label = match group_label {
        Some(g) if !g.is_empty() => format!("{} — {}", g, widget.label),
        _ => widget.label.clone(),
    };

    match (&widget.source, &widget.shape) {
        // Commands always render as buttons.
        (ValueSource::Command(cmd), WidgetShape::Button { confirm }) => {
            Some(ControlSection::CustomAction(CustomActionSectionConfig {
                label,
                command: cmd.clone(),
                params: Default::default(),
                style: ButtonStyle::Default,
                confirm: confirm.clone(),
            }))
        }

        // Numeric / toggle / dropdown / text shapes against a Parameter
        // source map onto the existing ParameterSectionConfig.
        (ValueSource::Parameter(name), shape) => {
            let widget_kind = match shape {
                WidgetShape::Slider { .. } => ParameterWidget::Slider,
                WidgetShape::Toggle => ParameterWidget::Toggle,
                WidgetShape::EnumSelect { .. } => ParameterWidget::Dropdown,
                WidgetShape::NumericInput { .. } => ParameterWidget::Spinner,
                WidgetShape::TextInput => ParameterWidget::TextInput,
                // Buttons can't bind to a parameter; reject below.
                WidgetShape::Button { .. } => return None,
            };
            Some(ControlSection::Parameter(ParameterSectionConfig {
                label,
                parameter: name.clone(),
                widget: widget_kind,
                read_only: false,
                read_command: None,
                write_command: None,
                write_param: None,
            }))
        }

        // Anything else (e.g. command + slider) is a manifest-author bug
        // we surface via the `tracing::warn!` in `emit_group`.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use driver_universal::panel_ir::{GroupIr, LayoutSlot, ValueSource, WidgetIr, WidgetShape};

    fn ir_one_widget(source: ValueSource, shape: WidgetShape) -> PanelIr {
        PanelIr {
            device_id: "dev".into(),
            title: Some("Test".into()),
            groups: vec![GroupIr {
                label: None,
                slot: LayoutSlot::Main,
                widgets: vec![WidgetIr {
                    source,
                    label: "L".into(),
                    description: None,
                    unit: None,
                    shape,
                }],
            }],
        }
    }

    #[test]
    fn slider_param_maps_to_parameter_section_slider() {
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Parameter("p".into()),
            WidgetShape::Slider {
                min: 0.0,
                max: 1.0,
                step: None,
            },
        ));
        match &cfg.sections[0] {
            ControlSection::Parameter(p) => {
                assert_eq!(p.parameter, "p");
                assert!(matches!(p.widget, ParameterWidget::Slider));
            }
            other => panic!("expected Parameter section, got {other:?}"),
        }
    }

    #[test]
    fn toggle_param_maps_to_parameter_section_toggle() {
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Parameter("on".into()),
            WidgetShape::Toggle,
        ));
        let ControlSection::Parameter(p) = &cfg.sections[0] else {
            panic!("not a parameter section")
        };
        assert!(matches!(p.widget, ParameterWidget::Toggle));
    }

    #[test]
    fn enum_select_maps_to_dropdown() {
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Parameter("mode".into()),
            WidgetShape::EnumSelect {
                choices: vec!["a".into(), "b".into()],
            },
        ));
        let ControlSection::Parameter(p) = &cfg.sections[0] else {
            panic!("not a parameter section")
        };
        assert!(matches!(p.widget, ParameterWidget::Dropdown));
    }

    #[test]
    fn numeric_input_maps_to_spinner() {
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Parameter("x".into()),
            WidgetShape::NumericInput {
                min: None,
                max: None,
                step: None,
            },
        ));
        let ControlSection::Parameter(p) = &cfg.sections[0] else {
            panic!("not a parameter section")
        };
        assert!(matches!(p.widget, ParameterWidget::Spinner));
    }

    #[test]
    fn text_input_maps_to_text_input() {
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Parameter("name".into()),
            WidgetShape::TextInput,
        ));
        let ControlSection::Parameter(p) = &cfg.sections[0] else {
            panic!("not a parameter section")
        };
        assert!(matches!(p.widget, ParameterWidget::TextInput));
    }

    #[test]
    fn command_button_maps_to_custom_action() {
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Command("reset".into()),
            WidgetShape::Button { confirm: None },
        ));
        match &cfg.sections[0] {
            ControlSection::CustomAction(a) => {
                assert_eq!(a.command, "reset");
                assert_eq!(a.confirm, None);
            }
            other => panic!("expected CustomAction, got {other:?}"),
        }
    }

    #[test]
    fn command_button_carries_confirm_prompt() {
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Command("zap".into()),
            WidgetShape::Button {
                confirm: Some("Sure?".into()),
            },
        ));
        let ControlSection::CustomAction(a) = &cfg.sections[0] else {
            panic!("not a custom action")
        };
        assert_eq!(a.confirm.as_deref(), Some("Sure?"));
    }

    #[test]
    fn second_labeled_group_emits_separator_between_groups() {
        // First group has no leading separator (would be redundant at the
        // top of the panel). Subsequent labeled groups get a spacer.
        let ir = PanelIr {
            device_id: "dev".into(),
            title: None,
            groups: vec![
                GroupIr {
                    label: Some("Power".into()),
                    slot: LayoutSlot::Main,
                    widgets: vec![WidgetIr {
                        source: ValueSource::Parameter("p".into()),
                        label: "P".into(),
                        description: None,
                        unit: None,
                        shape: WidgetShape::Toggle,
                    }],
                },
                GroupIr {
                    label: Some("Diagnostics".into()),
                    slot: LayoutSlot::Main,
                    widgets: vec![WidgetIr {
                        source: ValueSource::Parameter("d".into()),
                        label: "D".into(),
                        description: None,
                        unit: None,
                        shape: WidgetShape::Toggle,
                    }],
                },
            ],
        };
        let cfg = synthesize_config(&ir);
        // [Param("Power — P"), Separator, Param("Diagnostics — D")]
        assert_eq!(cfg.sections.len(), 3);
        assert!(matches!(cfg.sections[0], ControlSection::Parameter(_)));
        assert!(matches!(cfg.sections[1], ControlSection::Separator(_)));
        assert!(matches!(cfg.sections[2], ControlSection::Parameter(_)));
    }

    #[test]
    fn first_labeled_group_skips_leading_separator() {
        let ir = PanelIr {
            device_id: "dev".into(),
            title: None,
            groups: vec![GroupIr {
                label: Some("Power".into()),
                slot: LayoutSlot::Main,
                widgets: vec![WidgetIr {
                    source: ValueSource::Parameter("p".into()),
                    label: "P".into(),
                    description: None,
                    unit: None,
                    shape: WidgetShape::Toggle,
                }],
            }],
        };
        let cfg = synthesize_config(&ir);
        // Just the parameter — no leading separator, no extra junk.
        assert_eq!(cfg.sections.len(), 1);
        let ControlSection::Parameter(p) = &cfg.sections[0] else {
            panic!("expected Parameter section")
        };
        // Group label is prefixed onto the per-widget label.
        assert_eq!(p.label, "Power — P");
    }

    #[test]
    fn empty_ir_yields_empty_sections() {
        let ir = PanelIr {
            device_id: "dev".into(),
            title: None,
            groups: vec![],
        };
        let cfg = synthesize_config(&ir);
        assert!(cfg.sections.is_empty());
        assert!(cfg.show_header);
    }

    #[test]
    fn parameter_with_button_shape_is_skipped() {
        // Manifest author error: parameter source + button shape. The
        // translator skips it instead of panicking.
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Parameter("p".into()),
            WidgetShape::Button { confirm: None },
        ));
        assert!(cfg.sections.is_empty());
    }

    #[test]
    fn command_with_non_button_shape_is_skipped() {
        let cfg = synthesize_config(&ir_one_widget(
            ValueSource::Command("c".into()),
            WidgetShape::Toggle,
        ));
        assert!(cfg.sections.is_empty());
    }

    #[test]
    fn empty_group_label_does_not_emit_separator() {
        let ir = PanelIr {
            device_id: "dev".into(),
            title: None,
            groups: vec![GroupIr {
                label: Some(String::new()),
                slot: LayoutSlot::Main,
                widgets: vec![WidgetIr {
                    source: ValueSource::Parameter("p".into()),
                    label: "P".into(),
                    description: None,
                    unit: None,
                    shape: WidgetShape::Toggle,
                }],
            }],
        };
        let cfg = synthesize_config(&ir);
        // Only the Parameter section, no Separator for empty label.
        assert_eq!(cfg.sections.len(), 1);
        assert!(matches!(cfg.sections[0], ControlSection::Parameter(_)));
    }
}
