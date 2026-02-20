# Config-Driven Device Control Panels

Device TOML configs can include a `[ui.control_panel]` section that declaratively
defines how the GUI renders controls for that device. When present, this takes
priority over hardcoded panels — no Rust code changes needed for new devices.

## Quick Start

Add a `[ui.control_panel]` section to any `config/devices/*.toml` file:

```toml
[ui.control_panel]
layout = "vertical"       # "vertical" (default) or "horizontal"
show_header = true        # Show device name header
collapsible = false       # Allow collapsing the panel

[[ui.control_panel.sections]]
type = "motion"
label = "Position"
show_jog = true
jog_steps = [0.1, 1.0, 10.0]
precision = 3
unit = "mm"
```

## Dispatch Priority

The GUI routes devices to control panels in this order:

1. **Config-driven** — `[ui.control_panel]` exists in the device's TOML config
2. **Hardcoded** — Specialized panels for known hardware (MaiTai, Comedi, etc.)
3. **Generic** — Auto-composed from device capabilities (movable, readable, etc.)

## Section Types

### `motion` — Position control with jog buttons

```toml
[[ui.control_panel.sections]]
type = "motion"
label = "X Axis"          # Section header
show_jog = true           # Show jog increment buttons (default: true)
jog_steps = [0.1, 1.0, 10.0]  # Jog step sizes
show_home = true          # Show Home button (default: false)
show_stop = true          # Show Stop button (default: true)
precision = 3             # Decimal places for position display
unit = "mm"               # Unit label
```

gRPC: `get_device_state()` for position, `move_absolute()`, `move_relative()`, `move_absolute(..., 0.0)` for Home, `execute_device_command("stop")` for Stop

### `sensor` — Read-only measurement with optional gauge and trend

```toml
[[ui.control_panel.sections]]
type = "sensor"
label = "Power"
precision = 2
unit = "mW"
show_trend = true         # Show rolling line chart (default: false)
refresh_ms = 1000         # Auto-refresh interval in ms (default: 0 = manual only)
```

gRPC: `read_value()` on init + at `refresh_ms` interval

### `shutter` — Open/close toggle

```toml
[[ui.control_panel.sections]]
type = "shutter"
label = "Beam Shutter"
toggle_style = true       # true = single toggle, false = separate Open/Close buttons
```

gRPC: `get_shutter()`, `set_shutter()`

### `wavelength` — Tunable laser wavelength control

```toml
[[ui.control_panel.sections]]
type = "wavelength"
label = "Wavelength"
show_slider = true        # Show slider control (default: true)
presets = [700.0, 800.0, 900.0]  # Quick-set preset buttons
show_color = true         # Show wavelength-to-color indicator (default: true)
```

gRPC: `get_wavelength()`, `set_wavelength()`

### `preset_buttons` — Quick-set position buttons

```toml
[[ui.control_panel.sections]]
type = "preset_buttons"
label = "Quick Positions"
presets = [
    { label = "0°", value = 0.0 },
    { label = "90°", value = 90.0 },
    { label = "180°", value = 180.0 },
]
vertical = false          # Button layout direction (default: false = horizontal)
```

gRPC: `move_absolute()` for each preset

### `parameter` — Read/write device parameter

```toml
[[ui.control_panel.sections]]
type = "parameter"
label = "Gain"
parameter = "gain"        # Parameter key for gRPC get/set
read_only = false
widget = "slider"         # "auto", "text_input", "slider", "spinner", "toggle", "dropdown"
```

gRPC: `get_parameter()`, `set_parameter()`

### `status_display` — Read-only parameter grid

```toml
[[ui.control_panel.sections]]
type = "status_display"
label = "Status"
parameters = ["emission_enabled", "interlock_ok"]
compact = true            # Compact inline layout (default: false)
```

gRPC: `get_parameter()` per parameter name

### `custom_action` — Command button

```toml
[[ui.control_panel.sections]]
type = "custom_action"
label = "Reset Interlock"
command = "reset_interlock"
params = {}               # Optional JSON parameters
style = "danger"          # "default", "primary", "success", "danger"
confirm = "Are you sure you want to reset the interlock?"  # Confirmation message (omit to skip)
```

gRPC: `execute_device_command()`

### `separator` — Visual divider

```toml
[[ui.control_panel.sections]]
type = "separator"
visible = true
```

### `camera` — Camera exposure/gain (placeholder)

```toml
[[ui.control_panel.sections]]
type = "camera"
label = "Camera"
```

### `custom` — Plugin-rendered widget (placeholder)

```toml
[[ui.control_panel.sections]]
type = "custom"
label = "Custom Widget"
widget = "my_widget_name"
```

## Example Configs

See these files for complete examples:

| File | Device | Sections |
|------|--------|----------|
| `ell14.toml` | Thorlabs ELL14 rotator | motion + presets |
| `maitai.toml` | MaiTai Ti:Sapphire laser | wavelength + shutter + sensor + status |
| `esp300.toml` | Newport ESP300 stage | motion |
| `thorlabs_pm400.toml` | Thorlabs PM400 power meter | sensor + parameter + custom_action |
| `ipg_laser.toml` | IPG fiber laser | sensor + custom_actions + parameter + status |

## Architecture

The config-driven panel system lives in `crates/ui/src/panels/instrument_manager/`:

- `config_loader.rs` — `DeviceConfigCache` loads and caches all `config/devices/*.toml` files
- `config_renderer.rs` — `ConfigDrivenPanel` renders sections from `ControlPanelConfig`
- `dispatch.rs` — `determine_panel_type_with_config()` routes devices to panels
- `config_tests.rs` — Integration tests for config loading

The schema types are defined in `crates/hardware/src/config/schema.rs` — all fields have
`#[serde(default)]` so a minimal `type = "motion"` produces a functional panel.
