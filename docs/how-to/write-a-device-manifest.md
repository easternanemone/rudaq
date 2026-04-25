# Write a Device Manifest (v4)

This guide walks you through authoring a `driver-universal` device manifest
end-to-end. It covers every major v4 feature: transport, commands, response
format strings, variants, transforms, rich parameter metadata, `evalexpr`
conversions, inline UI declarations, and the CLI tools that validate and
scaffold manifests.

The running example is `config/devices/tutorial_device_example.toml` — a
fictitious ACME DPS-3010 programmable bench DC power supply. Read this guide
beside that file; each section below names the corresponding table.

For the broader conceptual background (tiers, validation pipeline, v2 → v3
migration), see [`device-config.md`](device-config.md) — this page assumes
you are working in v4 and focuses on the features that are new or reshaped.

## Prerequisites

- `schema_version = 3` at the top of every manifest. The v4 revision shares
  the v3 schema number; the jump is in *patterns and tooling*, not in the
  declared version integer.
- `driver-universal` binaries compiled locally:

  ```bash
  cargo build -p driver-universal --bin manifest-check
  cargo build -p driver-universal --bin migrate-v3
  ```

- Your manifest lives in `config/devices/<name>.toml`. The daemon scans that
  directory at boot; anything valid there becomes an addressable device.

## 1. Device identity

```toml
[device]
name = "ACME DPS-3010 (tutorial)"
manufacturer = "ACME Instruments"
model = "DPS-3010"
description = "Fictitious 30 V / 10 A programmable bench DC power supply."
category = "source"
capabilities = ["Readable", "Settable", "Commandable"]
```

The `capabilities` array must match the capability tables you populate later
(`[capabilities.readable]`, `[capabilities.settable]`, etc.). `manifest-check`
warns if they drift apart.

## 2. Transport

```toml
[connection]
type = "serial"
baud_rate = 9600
data_bits = 8
parity = "none"
stop_bits = 1
flow_control = "none"
timeout_ms = 1000
terminator = "\n"
```

Supported `type` values are `serial`, `tcp`, and `ipg_tcp` (a TCP variant
that skips the echoed command on query). Use `terminator_tx` / `terminator_rx`
when TX and RX differ — a plain `terminator` applies to both.

Use `[[init_sequence]]` to put the device into a known state after connect:

```toml
[[init_sequence]]
command = "clear_errors"
delay_ms = 50
```

Each entry names a `[commands.*]` entry; it must either declare a response or
set `expects_response = false`.

## 3. Response parsing

v4 gives you three complementary tools for decoding replies. Reach for them
in this order:

### 3.1 Format strings — `format = "..."`

Use for structured replies where you know the layout. Specifiers:

| Specifier     | Meaning                                    |
|---------------|--------------------------------------------|
| `{name}`      | Greedy string capture until the next literal |
| `{name:N}`    | Exactly `N` characters (string)             |
| `{name:int}`  | Decimal integer                             |
| `{name:float}`| Floating point number                       |
| `{name:hex2}`, `{name:hex4}`, `{name:hex8}` | Fixed-width hex → integer |
| `{_:N}`       | Skip N characters (discard)                 |

```toml
[responses.voltage_reading]
format = "VOLT: {value:float} V"
```

This parses `"VOLT: 12.345 V"` into `{ value: 12.345 }`. The literals outside
`{}` must match exactly; unit suffixes and prefixes are part of the template,
not an afterthought.

### 3.2 Variants — `variants = [...]`

Use when firmware differences or optional fields produce more than one reply
shape. First match wins.

```toml
[responses.status_line]
variants = [
    "{state},{code:int},{temp_c:float}",  # firmware ≥ 2.1
    "{state},{code:int}",                 # older firmware
]
```

Variants replace nearly every regex-alternation case in practice. If you find
yourself writing a regex, see if two variants handle it instead.

### 3.3 Transform pipelines — `transform = [...]`

Use for simple sequential operations, or to normalize a value after extraction.
Operations run left-to-right. Available ops:

| Shorthand                      | Effect                                              |
|--------------------------------|-----------------------------------------------------|
| `trim`                         | Strip leading/trailing whitespace                   |
| `remove_prefix('X')`           | Drop a fixed prefix if present                      |
| `remove_suffix('X')`           | Drop a fixed suffix if present                      |
| `to_float`                     | Parse as `f64`                                      |
| `to_int`                       | Parse as `i64`                                      |
| `scale(factor)`                | Multiply numeric value                              |
| `offset(value)`                | Add to numeric value                                |
| `split_comma(index)`           | Comma-split, take element `index`                   |
| `map('from', 'to')`            | Replace one specific string with another            |
| `match_one_of(['a','b','c'])`  | Pass-through if in set, else error                  |
| `format('tmpl', 'field')`      | Parse against a format template, yield one field    |
| `regex_extract('re', group)`   | Regex escape hatch (prefer `format` / `variants`)   |

Example from the tutorial:

```toml
[responses.output_state]
transform = [
    "trim",
    "map('ON', 'true')",
    "map('OFF', 'false')",
]
```

`format(...)` is the preferred replacement for `regex_extract`. It's faster,
emulator-invertible, and easier to read. Keep `regex_extract` only when a
single response is genuinely shared across commands whose replies differ
only in prefix (see `config/devices/maitai.toml` for a real example).

### 3.4 Choosing among the three

| If the reply is… | Use |
|-------------------|-----|
| Always the same structured shape | `format` |
| One of a small fixed set of shapes | `variants` |
| A value that needs normalizing after extraction | `transform` |
| "CMD: value" where the prefix varies per command | `transform` with `format(...)` shorthand |

## 4. Commands

```toml
[commands.read_voltage]
template = "MEAS:VOLT?"
description = "Measure the actual output voltage."
response = "voltage_reading"

[commands.set_voltage]
template = "SOUR:VOLT {{ volts }}"
description = "Set the target output voltage."
expects_response = false
parameters = { volts = "float" }
```

- `template` is a MiniJinja string. `{{ placeholder }}` substitutes a named
  parameter. Custom filters `hex(n)`, `pad(n)`, and `int` are available.
- `parameters` declares each placeholder with a type (`float`, `int`, `int32`,
  `string`). `manifest-check` errors if a template uses a `{{ x }}` that has
  no matching `parameters.x` entry.
- `response` names the `[responses.*]` entry that parses the reply. Omit it
  together with `expects_response = false` for writes.

## 5. Rich parameter metadata

`[parameters.X]` may be a bare default or a table. The table form feeds the
GUI's default widgets and the DB schema. Only numeric defaults are stored
for formula evaluation; strings and bools round-trip through the TOML for
driver-level use.

```toml
[parameters.voltage_setpoint]
type = "float"
default = 0.0
range = [0.0, 30.0]
unit = "V"
description = "Target output voltage"
read_only = false
```

## 6. Conversions — `evalexpr` formulas

Conversions let you translate between a user-facing unit and the wire-level
unit the device wants. Each conversion is a pure `evalexpr` formula; the free
identifier(s) come from the capability method's `input_param` / `output_field`.

```toml
[conversions.millivolts_to_volts]
formula = "mv / 1000.0"

[conversions.volts_to_millivolts]
formula = "round(v * 1000.0)"
```

Keep formulas pure: no side effects, no I/O, no time. Validation happens at
load time — `manifest-check` builds the operator tree and rejects syntactic
errors before the daemon ever starts.

## 7. Capabilities

Capabilities wire commands into the typed capability traits (`Readable`,
`Settable`, `Movable`, …).

```toml
[capabilities.readable]
read = { command = "read_voltage", output_field = "value" }

[capabilities.settable]
set = {
    command = "set_voltage",
    from_param = "voltage_mv",
    input_param = "volts",
    input_conversion = "millivolts_to_volts",
}
```

- `from_param` is the caller's argument name (shows up in gRPC / scripts).
- `input_param` is the MiniJinja placeholder in the command template.
- `input_conversion` / `output_conversion` name a `[conversions.X]` formula.
- `output_field` names which extracted field the capability returns.

## 8. UI — config-driven control panels

```toml
[ui]
icon = "bolt"
color = "#2196F3"
panel_kind = "generic"

[ui.control_panel]
layout = "vertical"
show_header = true

[[ui.control_panel.sections]]
type = "sensor"
label = "Voltage"
precision = 3
unit = "V"
refresh_ms = 500

[[ui.control_panel.sections]]
type = "parameter"
label = "Voltage setpoint"
parameter = "voltage_setpoint"
widget = "spinner"
read_command = "read_voltage"
write_command = "set_voltage"

[[ui.control_panel.sections]]
type = "custom_action"
label = "Output ON"
command = "output_on"
style = "success"
```

Available section types: `motion`, `sensor`, `shutter`, `wavelength`,
`preset_buttons`, `parameter`, `custom_action`, `separator`, `status_display`.
The full catalogue lives in
[`config/devices/CONFIG_README.md`](../../config/devices/CONFIG_README.md).

Dispatch priority is: config-driven → hardcoded panel → generic
auto-composition from capabilities. Adding a `[ui.control_panel]` is usually
enough for the GUI to render the device without any Rust changes.

## 9. CLI tools

### `manifest-check`

The authoritative validator. Run it after every edit:

```bash
cargo run -p driver-universal --bin manifest-check -- config/devices/tutorial_device_example.toml
# OK  config/devices/tutorial_device_example.toml — 11 commands, 5 responses, 2 parameters
```

`manifest-check` reports missing command references, unknown response names,
undeclared MiniJinja placeholders, malformed format strings, invalid
`evalexpr` formulas, and capability-to-command mismatches. A green line here
is the contract between you and the loader.

### `migrate-v3`

One-shot converter for v3-deprecated patterns (top-level response `regex`
and `regex_extract(...)` transforms) into v4 format:

```bash
cargo run -p driver-universal --bin migrate-v3 --release -- path/to/manifest.toml --in-place
```

The migrator leaves a `# TODO` comment wherever it could not auto-reduce a
pattern. Resolve those before landing.

### `manifest-wizard`

An interactive scaffolder is planned as part of task F3c. When it lands,
prefer it over hand-writing the top of the file — it knows the full schema
and produces a `manifest-check`-green starting point. Until then, copy the
tutorial file and edit.

## 10. Workflow

1. Copy `tutorial_device_example.toml` to `config/devices/<your-device>.toml`.
2. Fill in `[device]`, `[connection]`, and an `[[init_sequence]]` if needed.
3. Add one command + one response at a time, running `manifest-check` after
   each change. Small steps keep diagnostics local.
4. Add capabilities once commands and responses are stable.
5. Add `[ui.control_panel]` last — it's the easiest thing to iterate on
   because it doesn't affect whether the driver loads.
6. Run the driver-universal test suite to catch anything the static checker
   misses:

   ```bash
   cargo nextest run -p driver-universal
   ```

7. Commit with a `feat(manifests):` prefix that names the device.

## References

- [`config/devices/tutorial_device_example.toml`](../../config/devices/tutorial_device_example.toml) — the worked example this guide walks through
- [`config/devices/CONFIG_README.md`](../../config/devices/CONFIG_README.md) — full `[ui.control_panel]` section catalogue
- [`docs/how-to/device-config.md`](device-config.md) — conceptual background, v2 → v3 migration, mock testing
- [`docs/how-to/hardware-drivers.md`](hardware-drivers.md) — bigger picture of the driver layer
- [`llm-wiki/crates/driver-universal.md`](../../llm-wiki/crates/driver-universal.md) — dense crate reference
- Working examples to read after the tutorial:
  `config/devices/ipg_laser.toml` (format+transform), `config/devices/ell14.toml`
  (variants with hex fields), `config/devices/thorlabs_sc10.toml` (map-based
  enum decoding), `config/devices/maitai.toml` (regex_extract escape hatch
  with justification).
