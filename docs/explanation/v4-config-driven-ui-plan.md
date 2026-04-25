# v4 Config-Driven UI — Implementation Plan

**Status:** active · **ADR:** [018-v4-manifest-first-universal-driver-ui](../adr/018-v4-manifest-first-universal-driver-ui.md) · **Beads:** `bd-jcb4x.2` (Sub-epic G)

This is the implementation roadmap for Phase 2 of the bd-jcb4x v4 program —
specifically G2 (synthesis), G3 (layout slots), and G4 (panel retirement). It
takes the architectural decision captured in ADR-018 and turns it into a
concrete, multi-PR sequence with contract boundaries that multiple agents can
pick up without re-litigating scope.

## 1. Why a neutral IR

ADR-018 recorded three candidate crate boundaries for Phase 2 synthesis:

- **(a)** Add `hardware` as a dependency of `driver-universal`.
- **(b)** Put synthesis entirely inside `crates/ui`.
- **(c)** Build a neutral intermediate representation (IR) in
  `driver-universal` and translate it into `hardware::ControlPanelConfig` in
  `crates/ui`.

We are taking option (c). Rationale:

- **`driver-universal` stays narrow.** It currently depends on `common`,
  serde, toml, tokio, and a handful of parser crates — nothing `hardware`.
  Adding `hardware` (option a) would pull in the schema crate's entire
  dependency graph (including its Validate macro, its derived JsonSchema
  surface, and transitively the `egui`-adjacent bits) into every binary
  that links a manifest-driven device. That's a real compile-time cost and
  an unnecessary coupling.
- **Option (b) would strand the schema knowledge.** The `CommandUiHint` /
  `ParameterUiHint` types produced by G1 live in
  `driver-universal::config::validated`. Extracting a widget shape from
  them is fundamentally a driver-universal concern (knows about
  parameter dtypes, enum_values, ranges). Pushing that into `crates/ui`
  would force the UI crate to import all of `driver-universal`'s
  validated module just to read hints, which is a cross-layer violation.
- **Option (c) preserves two narrow contracts.** `driver-universal`
  exposes a `PanelIr` that is entirely a description of intent — no
  widgets, no egui, no `hardware::config::schema` types. `crates/ui` is
  the only consumer that knows how to render it, and converts it to
  `ControlPanelConfig` (the shape `ConfigDrivenPanel` already consumes).
  Each side is independently testable via golden fixtures.

The IR acts as a versioned API boundary: if we later need a gRPC-driven UI
config service, it can ship the IR across the wire and clients can render
it without reading TOML.

## 2. IR type definitions

The IR lives in `crates/driver-universal/src/panel_ir.rs` as a new module,
re-exported from the crate root.

```rust
//! Neutral panel intermediate representation.
//!
//! Extracted from the validated manifest's inline UI hints. Intentionally
//! carries no widget-framework types, no egui, and no hardware::config types.
//! Downstream renderers translate it into their own widget configs.

use serde::{Deserialize, Serialize};

/// A full synthesized panel description for one device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PanelIr {
    /// Device id this panel is for (stringified form of `DeviceId`).
    pub device_id: String,
    /// Header label shown at the top of the panel. Falls back to the
    /// device's friendly name when the manifest doesn't specify one.
    pub title: Option<String>,
    /// Ordered list of groups. Groups are the primary layout unit.
    pub groups: Vec<GroupIr>,
}

/// A named cluster of widgets that share a layout slot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupIr {
    /// Group label, if the manifest provided one via
    /// `[commands.*.ui].group` or `[parameters.*.ui].group`. Ungrouped
    /// widgets share an anonymous `None`-labeled group.
    pub label: Option<String>,
    /// Layout slot for this group. Default = `Main`.
    pub slot: LayoutSlot,
    /// Widgets in this group, in manifest declaration order.
    pub widgets: Vec<WidgetIr>,
}

/// A single widget within a group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WidgetIr {
    /// Where the widget reads/writes its value.
    pub source: ValueSource,
    /// Human-readable label.
    pub label: String,
    /// Optional tooltip / help text.
    pub description: Option<String>,
    /// Optional unit suffix for numeric widgets.
    pub unit: Option<String>,
    /// The widget shape.
    pub shape: WidgetShape,
}

/// What the widget reads/writes against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ValueSource {
    /// Parameter identified by its manifest key.
    Parameter(String),
    /// Command identified by its manifest key; a `Command` source is
    /// always rendered as a `WidgetShape::Button` by the translator.
    Command(String),
}

/// Widget rendering shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WidgetShape {
    Slider { min: f64, max: f64, step: Option<f64> },
    Toggle,
    EnumSelect { choices: Vec<String> },
    NumericInput { min: Option<f64>, max: Option<f64>, step: Option<f64> },
    TextInput,
    Button { confirm: Option<String> },
}

/// Named layout region within a `ConfigDrivenPanel`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutSlot {
    TopLeft,
    BottomRight,
    #[default]
    Main,
}
```

All types are `pub` at the crate root and re-exported as
`driver_universal::panel_ir::{PanelIr, …}`. They derive `Serialize` and
`Deserialize` so the IR can round-trip to JSON for debugging / wire use.

## 3. Extractor rules

The extractor lives in `driver-universal` and has signature:

```rust
pub fn extract_panel_ir(
    manifest: &ValidatedManifest,
    device_id: &str,
) -> PanelIr;
```

It is **total** — any validated manifest produces a `PanelIr`, possibly
with zero groups if no UI hints are present. Rules:

### 3.1 Group assignment

- If a command or parameter has `ui_hint.group = Some(name)`, it joins the
  group named `name`.
- If it has `ui_hint` but no `group`, it joins the anonymous group
  (`GroupIr.label = None`).
- If it has no `ui_hint` at all, it is **omitted from the IR**. This is how
  manifest authors opt in piecewise: add `[commands.X.ui]` only to the
  commands they want on the panel.
- Group layout slot defaults to `LayoutSlot::Main`. If any widget in the
  group declares a `ui_hint.slot`, that slot wins (last-writer-wins in
  manifest order — we warn in `manifest-check` if multiple widgets in the
  same group disagree).

### 3.2 Command → WidgetIr

Decision table for commands, consulted in order:

| `widget_hint` | command parameters | → `WidgetShape` |
|---|---|---|
| `Some(Button)` | any | `Button { confirm: None }` |
| `None` | 0 parameters | `Button { confirm: None }` |
| `Some(other)` | — | warn, fall through to Button |

The label falls back to `command.description.unwrap_or(command_name)`.

### 3.3 Parameter → WidgetIr

Decision table for parameters:

| `widget_hint` | dtype | enum_values | range | → `WidgetShape` |
|---|---|---|---|---|
| `Some(Slider)` | numeric | — | Some | `Slider { min, max, step }` |
| `Some(Slider)` | numeric | — | None | warn, fall to NumericInput |
| `Some(Toggle)` | bool | — | — | `Toggle` |
| `Some(EnumSelect)` | string | non-empty | — | `EnumSelect { choices }` |
| `Some(NumericInput)` | numeric | — | — | `NumericInput { min, max, step }` |
| `Some(TextInput)` | string | — | — | `TextInput` |
| `None` | bool | — | — | `Toggle` |
| `None` | numeric | — | Some | `Slider { min, max, step }` |
| `None` | numeric | — | None | `NumericInput` |
| `None` | string | non-empty | — | `EnumSelect { choices }` |
| `None` | string | empty | — | `TextInput` |

Label falls back to `parameter.name`. Unit falls back to `parameter.unit`
if present in the manifest.

### 3.4 Ordering

Groups are emitted in the order their first widget was declared in the
manifest. Widgets within a group are in manifest declaration order. This
gives authors direct control over visual order without a separate
`order` field.

## 4. Translator mapping table

The translator lives in `crates/ui/src/panels/instrument_manager/synthesis.rs`
with signature:

```rust
pub(crate) fn synthesize_config(
    ir: &driver_universal::panel_ir::PanelIr,
) -> hardware::config::schema::ControlPanelConfig;
```

### 4.1 Top-level

- `PanelIr.title` → `ControlPanelConfig.show_header = true` + device
  header uses the title. If `None`, hide the custom title and let the
  default device name show.
- `PanelIr.groups` → `ControlPanelConfig.sections`, one `ControlSection`
  per widget (groups are not preserved as a nesting level — they only
  affect layout slot and label). Group label becomes a `Separator`
  section emitted before the group's widgets when `GroupIr.label` is
  `Some(_)`.
- `ControlPanelConfig.layout` = `PanelLayout::Vertical` for now. (G3
  introduces a horizontal/grid mode driven by slot mix.)
- `ControlPanelConfig.collapsible = false`, `show_header = true`.

### 4.2 Per-widget

| `WidgetShape` + `ValueSource` | → `ControlSection` |
|---|---|
| `Button { confirm }` + `Command(cmd)` | `CustomAction(CustomActionSectionConfig { label, command, params: {}, style: Default, confirm })` |
| `Slider { min, max, step }` + `Parameter(p)` | `Parameter(ParameterSectionConfig { label, parameter, widget: Slider, read_only: false, … })` |
| `Toggle` + `Parameter(p)` | `Parameter(ParameterSectionConfig { label, parameter, widget: Toggle, … })` |
| `EnumSelect { choices }` + `Parameter(p)` | `Parameter(ParameterSectionConfig { label, parameter, widget: Dropdown, … })` |
| `NumericInput { … }` + `Parameter(p)` | `Parameter(ParameterSectionConfig { label, parameter, widget: Spinner, … })` |
| `TextInput` + `Parameter(p)` | `Parameter(ParameterSectionConfig { label, parameter, widget: TextInput, … })` |

Any other source/shape combination logs a warning and is **skipped** (the
translator is total and infallible; it never panics on bad input).

### 4.3 Slot handling (G3)

Layout slots do not map directly to `ControlPanelConfig` fields because
the existing schema doesn't have them. G3 adds a new `LayoutSlot` field
to `ControlSection` variants (via a helper wrapper in `crates/ui`). The
renderer in `ConfigDrivenPanel` groups sections by slot and lays them
out in a 2x2 grid (top_left / top_right / bottom_left / bottom_right)
or as a single vertical column in `Main`.

## 5. Integration sequence

Each bullet is one PR. PRs are strictly ordered — each builds on the
previous. A PR should not be opened until its predecessor has merged.

### G2a — IR types + extractor (driver-universal only)

- Adds `crates/driver-universal/src/panel_ir.rs` with the types from §2.
- Adds `crates/driver-universal/src/panel_extract.rs` with
  `extract_panel_ir`.
- Re-exports `panel_ir` from the crate root.
- Unit tests: fixture manifest (synthetic, not one of the 14 production
  ones) that exercises each decision-table row in §3.2 and §3.3. At
  least one test per table row.
- No UI crate changes. No behavioral impact on the daemon.
- Acceptance: `cargo nextest run -p driver-universal` passes; the IR
  serializes to JSON cleanly (smoke test).

### G2b — Translator + golden fixtures (crates/ui only)

- Adds `crates/ui/src/panels/instrument_manager/synthesis.rs` with
  `synthesize_config`.
- Unit tests: golden fixtures of `PanelIr` JSON → expected
  `ControlPanelConfig` JSON (round-trip via serde so the test is
  resilient to field additions). At least one fixture per row in §4.2.
- No runtime changes — the translator is dead code.
- Acceptance: `cargo nextest run -p ui --lib` passes; both native and
  wasm32 compile checks stay green.

### G2c — Wire into PanelRegistry

- In `crates/ui/src/app/panel_registry.rs` (or wherever D3's registry
  lives), extend the render-body decision path:
  1. If `grpc_config` has a `Present(cfg)`, use `cfg`.
  2. **New**: Otherwise, if the device has a manifest with inline UI
     hints, call `extract_panel_ir` → `synthesize_config` and use that.
  3. Otherwise fall back to the existing hand-written panel match arm.
- No panel deletions yet — the synthesized path is additive.
- Add a feature flag `universal_synthesis` (default: enabled) so we can
  disable it at compile time if something goes wrong in dogfooding.
- Acceptance: launch daemon with `tutorial_device_example.toml`, verify
  GUI shows a panel synthesized from the manifest.

### G3 — Layout slots

- Adds `LayoutSlot` to `ConfigDrivenPanel`'s internal section
  representation (not to the public `ControlPanelConfig` schema — we
  keep the schema stable).
- Renders main-slot sections in a column, top_left/bottom_right in a
  2x2 grid header region above main.
- Tests: snapshot test of ConfigDrivenPanel for a known IR at each slot.

### G4a — Migrate rotator (smallest scope, proof of concept)

- Add `[commands.X.ui]` / `[parameters.X.ui]` to `config/devices/ell14.toml`.
- Add a runtime constant `USE_SYNTHESIZED_ROTATOR_PANEL = true`.
- When true, the ConfigDriven path handles rotator devices; when false,
  the existing hand-written `RotatorControlPanel` does.
- Acceptance: both paths produce equivalent UIs on leabs-dev; default
  flag stays true for a week.

### G4b — Migrate remaining panels

- Stage (`esp300.toml`, `esp301_example.toml`)
- Power meter generic (`newport_1830c.toml`, `thorlabs_pm400.toml`)
- MaiTai (`maitai.toml`)
- Generic (last because it's the fallback for un-annotated devices — it
  becomes a no-op panel when the synthesizer returns zero widgets, which
  is the correct behavior).

### G4c — Delete retired panels

- Remove `PanelWidget::{Stage, Rotator, PowerMeter, Generic, MaiTai}`
  variants from `crates/ui/src/app/types.rs`.
- Remove their `PanelController::ensure_*` helpers.
- Remove their routing cases from `PanelRegistry::render_body`.
- Delete the panel source files themselves
  (`crates/ui/src/panels/rotator_control.rs`, etc.).
- Remove the `USE_SYNTHESIZED_*` runtime flags.
- Acceptance: UI crate loses ~1500 lines of panel glue, all 14 device
  manifests continue to work.

## 6. Testing strategy

Three levels of tests, one at each boundary:

1. **Manifest → IR** (in `driver-universal`). Fixture-based. Each
   §3 decision-table row gets at least one test. Fixture manifests are
   synthetic TOML strings inlined in the test module, not real device
   files.

2. **IR → ControlPanelConfig** (in `crates/ui`). Golden JSON fixtures.
   Each §4.2 mapping row gets at least one test. Use `insta` or
   `serde_json::to_value` equality for the assertions.

3. **End-to-end** (in `integration-tests`). Launch the daemon with
   `config/devices/tutorial_device_example.toml`, connect via gRPC,
   verify the device's `ControlPanelConfig` is non-empty and has
   expected section count. No GUI rendering in integration tests.

For G4a, also add a **dogfood smoke** — a human-visible checklist in
the PR description that someone runs the daemon on leabs-dev and
verifies the synthesized rotator panel visually matches the
hand-written one before merging.

## 7. Migration safety

Each G4 migration PR keeps the hand-written panel alive behind a
`const USE_SYNTHESIZED_X: bool = true` flag. To roll back a migration,
flip the constant to `false` and rebuild — the hand-written panel
returns. We remove the constant and the hand-written code only in G4c,
after all five migrations have been live for ≥7 days without
regressions.

If an unforeseen constraint emerges (e.g. the rotator panel has a
jog-speed keybinding the synthesizer can't replicate), the migration
for that device stops at G4a-style state until the IR grows a new
widget shape to accommodate it.

## 8. Open questions

These are questions we expect to encounter during implementation.
Answers will be recorded in the PR that settles them, not preemptively
in this doc.

- **Conversion formulas in the IR?** If a parameter's manifest declares
  `[conversions.degrees_to_pulses]`, does the IR carry that metadata so
  the synthesized slider displays in degrees while writing pulses?
  *Working assumption:* yes, as an optional `display_conversion:
  Option<String>` on `WidgetIr`.
- **`CustomAction` with command parameters?** Inline UI declares the
  button. The button's command may take parameters (e.g. `move_to(angle)`).
  The synthesized button currently passes `params: {}`. How do we
  handle commands that need user input?
  *Working assumption:* commands with required parameters are not
  eligible for `WidgetShape::Button` — they render as a disabled
  "needs-args" placeholder with a warning in `manifest-check`.
- **Runtime toggle granularity?** Is `USE_SYNTHESIZED_X` per-device-
  type (one flag for all rotators) or per-device-id (one flag for
  "ell14_a")?
  *Working assumption:* per-type, because per-id explodes the config
  surface.
- **Does `manifest-check` warn on missing UI hints?** If a manifest has
  parameters/commands but no `[.ui]` tables, the synthesized panel is
  empty. Should the CLI flag that as a warning so undergrads don't
  ship devices with blank panels?
  *Working assumption:* yes, `manifest-check --ui-check` flags it;
  default CLI invocation stays quiet.

## 9. Beads decomposition

This plan maps to beads issues as follows:

| Bead | Scope |
|---|---|
| `bd-jcb4x.2.2a` | G2a — IR types + extractor |
| `bd-jcb4x.2.2b` | G2b — Translator + golden fixtures |
| `bd-jcb4x.2.2c` | G2c — Wire into PanelRegistry |
| `bd-jcb4x.2.3`  | G3 — Layout slots (existing) |
| `bd-jcb4x.2.4a` | G4a — Migrate rotator |
| `bd-jcb4x.2.4b` | G4b — Migrate remaining panels |
| `bd-jcb4x.2.4c` | G4c — Delete retired panels |

G2 (`bd-jcb4x.2.2`) becomes an epic that holds G2a/G2b/G2c. G4
(`bd-jcb4x.2.4`) becomes an epic that holds G4a/G4b/G4c.

## 10. Non-goals

Explicitly out of scope for this plan:

- **Redesigning native SDK panels** (PVCAM, Andor, Dover, Comedi).
  These stay hand-written per ADR-018.
- **Wire-protocol synthesis.** We do not stream `PanelIr` from the
  daemon to the client — synthesis happens client-side from the
  manifest. A future ADR may revisit this if we need dynamic panels.
- **TOML schema breaking changes.** `[commands.X.ui]` is the v4
  schema G1 froze; this plan does not modify it.
- **egui layout primitives.** G3 uses the existing `ConfigDrivenPanel`
  layout code; we do not introduce a new layout engine.
