# Config-Driven Control Panel Rendering

This module implements config-driven rendering for device control panels, allowing
UI layouts to be defined in TOML configuration files instead of hardcoded logic.

## Architecture

### Components

1. **config_loader.rs** - Loads device TOML configs and caches them
2. **config_renderer.rs** - Renders UI based on `ControlPanelConfig`
3. **mod.rs** - Integration with InstrumentManagerPanel

### Configuration Flow

```
config/devices/*.toml
         ↓
   DeviceConfigCache::load_all()
         ↓
   DeviceConfig (with UiConfig)
         ↓
   render_device_control_panel()
         ↓
   config_renderer::render_config_panel()
```

## Usage

### Defining UI Config in TOML

Example from `config/devices/ell14.toml`:

```toml
[ui]
icon = "motor"
color = "#4CAF50"

[ui.control_panel]
layout = "vertical"
show_header = true

[[ui.control_panel.sections]]
type = "motion"
label = "Rotation"
show_jog = true
jog_steps = [0.1, 1.0, 5.0, 10.0, 45.0]
show_home = true
show_stop = true

[[ui.control_panel.sections]]
type = "preset_buttons"
label = "Quick Positions"
presets = [
    { label = "0°", value = 0.0 },
    { label = "45°", value = 45.0 },
    { label = "90°", value = 90.0 },
    { label = "180°", value = 180.0 },
]
```

### Supported Control Sections

| Section Type | Description | Config Fields |
|--------------|-------------|---------------|
| `motion` | Position display and jog controls | `show_jog`, `jog_steps`, `show_home`, `show_stop`, `precision`, `unit` |
| `preset_buttons` | Quick position/value buttons | `presets`, `vertical` |
| `custom_action` | Single action button | `label`, `command`, `params`, `style`, `confirm` |
| `camera` | Camera controls | `show_exposure`, `show_gain`, `show_binning`, `show_roi`, `show_histogram`, `show_stats` |
| `shutter` | Shutter control toggle | `toggle_style` |
| `wavelength` | Wavelength tuning | `show_slider`, `presets`, `show_color` |
| `parameter` | Generic parameter display/edit | `parameter`, `widget`, `read_only` |
| `status_display` | Read-only status info | `parameters`, `compact` |
| `sensor` | Sensor reading display | `precision`, `unit`, `show_trend`, `refresh_ms` |
| `separator` | Visual separator or spacer | `height`, `visible` |
| `custom` | Custom widget (plugin-based) | `widget`, `config` |

## Fallback Behavior

If no UI config is found for a device, the panel falls back to:
1. Hardcoded device-specific panels (MaiTaiControlPanel, etc.)
2. Capability-based generic rendering

This ensures backward compatibility with existing devices.

## Adding New Control Sections

1. Add variant to `ControlSection` enum in `daq-hardware/src/config/schema.rs`
2. Add corresponding config struct (e.g., `MotionSectionConfig`)
3. Add match arm in `config_renderer::render_section()`
4. Implement actual UI rendering logic

## Testing

Run tests:
```bash
cargo test -p daq-egui config_tests
```

## Future Improvements

- [ ] Implement full rendering for all section types (currently stub placeholders)
- [ ] Add command execution from config (CustomAction, PresetButtons)
- [ ] Support grid layout with configurable columns
- [ ] Add live config reloading (hot reload)
- [ ] Implement custom widget plugin system
- [ ] Add visual config editor in GUI
