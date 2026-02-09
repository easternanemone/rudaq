# driver-universal

Declarative TOML-based driver system for rust-daq (schema v3). Add support for serial, TCP, and SCPI instruments **without writing any Rust code**.

## When to Use This

| Approach | Use When |
|----------|----------|
| **driver-universal (this crate)** | ASCII/SCPI serial or TCP devices with standard command-response patterns |
| **Native driver crate** | Complex protocols, binary data, multi-step handshakes, or high-performance requirements |

## How It Works

Device manifests go through a parse-don't-validate pipeline:

```
TOML file → RawManifest (serde) → DeviceManifest (validated) → DeviceComponents (runtime)
```

Place `.toml` configs in `config/devices/` — they are loaded at daemon startup via `load_all_factories()`.

## Schema v3 Structure

A minimal device config:

```toml
schema_version = 3

[device]
name = "My Instrument"
capabilities = ["Readable"]

[connection]
type = "serial"       # serial, tcp, or udp
baud_rate = 9600
timeout_ms = 1000

[commands.read_value]
template = "READ?"
expects_response = true

[responses.read_value]
transform = ["trim", "to_float"]

[capabilities.readable]
read = { command = "read_value" }
```

### Available Sections

| Section | Purpose |
|---------|---------|
| `[device]` | Name, capabilities, category, manufacturer |
| `[connection]` | Transport type, baud rate, terminators, timeout |
| `[commands.*]` | Command templates with MiniJinja parameters |
| `[responses.*]` | Response parsing (transform pipelines, regex, format strings) |
| `[capabilities.*]` | Map capability trait methods to commands |
| `[parameters.*]` | Device parameters with types, ranges, defaults |
| `[conversions.*]` | Unit conversion formulas (evalexpr syntax) |
| `[[init_sequence]]` | Commands to run on connect |
| `[error_codes.*]` | Error code mapping with recovery actions |

## Command Templates

Commands use [MiniJinja](https://docs.rs/minijinja) template syntax:

```toml
[commands.move_abs]
template = "{{ addr }}ma{{ (position * pulses_per_deg) | round }}"
parameters = { addr = "string", position = "float" }
```

Custom filters: `round`, `hex`, `pad_left`, `pad_right`, `abs`.

## Response Parsing

Four tiers, from simplest to most powerful:

**1. SCPI auto-parse** — numeric responses parsed automatically:
```toml
[commands.read]
template = "MEAS?"
response_type = "float"
```

**2. Transform pipelines** — chainable string operations:
```toml
[responses.reading]
transform = ["trim", "split_whitespace_first", "to_float"]
```

Available transforms: `trim`, `to_float`, `to_int`, `to_bool`, `strip_prefix("...")`, `strip_suffix("...")`, `split_whitespace_first`, `split_whitespace_last`, `regex_extract("pattern")`, `scale(factor)`, `offset(value)`, `clamp(min,max)`.

**3. Format strings** — positional field extraction:
```toml
[responses.status]
format = "{addr:1}GS{code:hex2}"
```

**4. Regex** — named capture groups:
```toml
[responses.position]
regex = "^(?P<value>[+-]?\\d+\\.?\\d*)\\s*(?P<units>\\w+)$"
```

## Capability Mapping

Map trait methods to commands:

```toml
[capabilities.movable]
move_abs = { command = "move_abs" }
move_rel = { command = "move_rel" }
position = { command = "get_position" }
home = { command = "home" }
stop = { command = "stop" }

[capabilities.readable]
read = { command = "read_value" }
```

## Examples

- [`config/devices/ell14.toml`](../../config/devices/ell14.toml) — Thorlabs ELL14 rotator (RS-485, hex encoding, complex parsing)
- [`config/devices/newport_1830c.toml`](../../config/devices/newport_1830c.toml) — Newport power meter (simple ASCII)
- [`config/devices/esp300.toml`](../../config/devices/esp300.toml) — Newport ESP300 motion controller (multi-axis)
- [`config/devices/minimal_device_template.toml`](../../config/devices/minimal_device_template.toml) — Copy-paste starting point

## Further Reading

- [Hardware Drivers Guide](../../docs/guides/hardware-drivers.md) — full guide including declarative driver patterns
- [NEWCOMER_GUIDE.md](../../docs/architecture/NEWCOMER_GUIDE.md) — system architecture overview
