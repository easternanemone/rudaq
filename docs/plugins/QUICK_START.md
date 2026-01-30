# Plugin Quick Start Guide

Get a plugin running in 5 minutes.

## Choose Your Path

**Option A: Config-Only Plugin (30 minutes)**
→ For simple ASCII/SCPI instruments

**Option B: Native Rust Plugin (2-4 hours)**
→ For complex hardware with state machines

**Option C: Rhai Script Plugin (1 hour)**
→ For experiment workflows and data processing

---

## Option A: Config-Only Plugin

### Step 1: Create Plugin Directory

```bash
mkdir -p ~/.rust-daq/plugins/my-device
cd ~/.rust-daq/plugins/my-device
```

### Step 2: Create plugin.toml

```toml
[plugin]
id = "my-device"
name = "My Device"
version = "1.0.0"
type = "config"

[[devices]]
type_id = "my_device"
display_name = "My Device"
config_file = "device.toml"
```

### Step 3: Create device.toml

```toml
[device]
name = "My Device"
manufacturer = "Acme"
model = "1000"
capabilities = ["Readable"]

[connection]
type = "serial"
baud_rate = 9600
terminator_tx = "\r\n"
terminator_rx = "\r\n"

[commands.read]
template = "READ?"
response = "value"

[responses.value]
pattern = "^(?P<value>[+-]?\\d+\\.?\\d*)$"

[responses.value.fields.value]
type = "float"

[trait_mapping.Readable.read]
command = "read"
output_field = "value"
```

### Step 4: Use in Experiment

```toml
[[devices]]
name = "sensor"
type = "my-device.my_device"
port = "/dev/ttyUSB0"
```

**Done!** Your device is now available.

---

## Option B: Native Rust Plugin

### Step 1: Create Plugin Project

```bash
mkdir -p ~/.rust-daq/plugins/my-native/src
cd ~/.rust-daq/plugins/my-native
```

### Step 2: Create Cargo.toml

```toml
[package]
name = "my-native-plugin"
version = "1.0.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]
name = "my_native"

[dependencies]
abi_stable = "0.11"
plugin-api = { path = "/path/to/rust-daq/crates/plugin-api" }
```

### Step 3: Create src/lib.rs

```rust
use abi_stable::export_root_module;
use abi_stable::sabi_extern_fn;
use abi_stable::std_types::{RHashMap, ROption, RResult, RString, RVec};
use plugin_api::prelude::*;

#[export_root_module]
fn get_root_module() -> PluginMod_Ref {
    PluginMod {
        abi_version,
        get_metadata,
        list_module_types,
        create_module,
    }.leak_into_prefix()
}

#[sabi_extern_fn]
fn abi_version() -> AbiVersion { AbiVersion::CURRENT }

#[sabi_extern_fn]
fn get_metadata() -> PluginMetadata {
    PluginMetadata::new("my-native", "My Native Plugin", "1.0.0")
}

#[sabi_extern_fn]
fn list_module_types() -> RVec<FfiModuleTypeInfo> {
    let mut types = RVec::new();
    types.push(FfiModuleTypeInfo {
        type_id: RString::from("my_module"),
        display_name: RString::from("My Module"),
        description: RString::from("Does something cool"),
        version: RString::from("1.0.0"),
        parameters: RVec::new(),
        event_types: RVec::new(),
        data_types: RVec::new(),
        required_roles: RVec::new(),
        optional_roles: RVec::new(),
    });
    types
}

#[sabi_extern_fn]
fn create_module(type_id: RString) -> RResult<ModuleFfiBox, RString> {
    match type_id.as_str() {
        "my_module" => RResult::ROk(
            ModuleFfi_TO::from_value(MyModule::new(), abi_stable::type_layout::TLPrefix)
        ),
        _ => RResult::RErr(RString::from("Unknown type")),
    }
}

// Your module implementation
pub struct MyModule { /* ... */ }

impl MyModule {
    fn new() -> Self { Self {} }
}

impl ModuleFfi for MyModule {
    fn type_info(&self) -> FfiModuleTypeInfo { /* ... */ }
    fn type_id(&self) -> RString { RString::from("my_module") }
    fn state(&self) -> FfiModuleState { FfiModuleState::Created }
    fn configure(&mut self, _: FfiModuleConfig) -> FfiModuleResult<RVec<RString>> {
        RResult::ROk(RVec::new())
    }
    fn get_config(&self) -> FfiModuleConfig { RHashMap::new() }
    fn stage(&mut self, _: &FfiModuleContext) -> FfiModuleResult<()> { RResult::ROk(()) }
    fn unstage(&mut self, _: &FfiModuleContext) -> FfiModuleResult<()> { RResult::ROk(()) }
    fn start(&mut self, _: FfiModuleContext) -> FfiModuleResult<()> { RResult::ROk(()) }
    fn pause(&mut self) -> FfiModuleResult<()> { RResult::ROk(()) }
    fn resume(&mut self) -> FfiModuleResult<()> { RResult::ROk(()) }
    fn stop(&mut self) -> FfiModuleResult<()> { RResult::ROk(()) }
    fn poll_event(&mut self) -> ROption<FfiModuleEvent> { ROption::RNone }
    fn poll_data(&mut self) -> ROption<FfiModuleDataPoint> { ROption::RNone }
}

unsafe impl Send for MyModule {}
unsafe impl Sync for MyModule {}
```

### Step 4: Build and Install

```bash
cargo build --release
cp target/release/libmy_native.dylib ~/.rust-daq/plugins/my-native/

# Create plugin.toml
cat > plugin.toml << 'EOF'
[plugin]
id = "my-native"
name = "My Native Plugin"
version = "1.0.0"
type = "native"
library = "libmy_native"
EOF
```

### Step 5: Use in Experiment

```toml
[[modules]]
name = "my_mod"
type = "my-native.my_module"
```

---

## Option C: Rhai Script Plugin

### Step 1: Create Plugin Directory

```bash
mkdir -p ~/.rust-daq/plugins/my-script
cd ~/.rust-daq/plugins/my-script
```

### Step 2: Create plugin.toml

```toml
[plugin]
id = "my-script"
name = "My Script Module"
version = "1.0.0"
type = "script"
engine = "rhai"

[[modules]]
type_id = "my_logger"
display_name = "My Logger"
script_file = "my_logger.rhai"

[[modules.parameters]]
id = "interval_ms"
type = "int"
default = "1000"
```

### Step 3: Create my_logger.rhai

```javascript
// State
let interval_ms = 1000;
let is_running = false;
let count = 0;

fn configure(params) {
    if "interval_ms" in params {
        interval_ms = parse_int(params["interval_ms"]);
    }
    return [];
}

fn stage(context) {
    print("[MyLogger] Staging");
    count = 0;
}

fn start(context) {
    print("[MyLogger] Starting");
    is_running = true;
}

fn stop() {
    print("[MyLogger] Stopping");
    is_running = false;
}

fn unstage(context) {
    print(`[MyLogger] Took ${count} readings`);
}

fn poll_data() {
    if !is_running { return (); }

    count += 1;
    return #{
        data_type: "reading",
        timestamp_ns: count * interval_ms * 1_000_000,
        values: #{ count: count },
        metadata: #{}
    };
}

fn poll_event() { return (); }
```

### Step 4: Use in Experiment

```toml
[[modules]]
name = "logger"
type = "my-script.my_logger"

[modules.parameters]
interval_ms = "500"
```

---

## What's Next?

- See `examples/plugins/` for complete working examples
- Read `docs/plugins/README.md` for full documentation
- Check `crates/plugin-api/` for the API reference

## Troubleshooting

**Plugin not discovered:**
```bash
ls ~/.rust-daq/plugins/*/plugin.toml
```

**Native plugin ABI error:**
- Rebuild against the current plugin-api version
- Check that AbiVersion::CURRENT matches

**Script syntax error:**
- Validate Rhai syntax at https://rhai.rs/playground/
- Check for missing semicolons or braces
