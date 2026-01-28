<objective>
Implement Phase 3: ESP300 + Additional Capabilities (Pattern Validation)

Purpose: Validate that the declarative driver pattern generalizes across different protocol styles by adding ESP300, Newport1830C, and MaiTai configs, plus implementing additional capability traits.

Output: Three new TOML configs, extended trait implementations, and validation tests proving the pattern works for diverse devices.
</objective>

<context>
Previous phases:
- Phase 1: @crates/daq-hardware/src/config/ (schema, validation, loader)
- Phase 2: @crates/daq-hardware/src/drivers/generic_serial.rs (GenericSerialDriver)
- Phase 2: @crates/daq-hardware/src/factory.rs (ConfiguredDriver, DriverFactory)
- Phase 2: @config/devices/ell14.toml (ELL14 protocol reference)

Existing hand-coded drivers to match:
@crates/daq-hardware/src/drivers/esp300.rs
@crates/daq-hardware/src/drivers/newport_1830c.rs
@crates/daq-hardware/src/drivers/maitai.rs

Capability traits to implement:
@crates/common/src/capabilities.rs (Readable, WavelengthTunable, ShutterControl)

Key differences from ELL14:
- ESP300: SCPI-like commands (`{axis}PA{position}`), different response format, multi-axis
- Newport1830C: Query-only (power meter), simple numeric responses
- MaiTai: Complex state machine (warmup, mode switching), wavelength tuning, shutter
</context>

<requirements>
**Config Files to Create:**

1. **ESP300** (`config/devices/esp300.toml`):
   - SCPI-style commands: `{axis}PA{position}`, `{axis}PR{distance}`, `{axis}TP?`
   - Multi-axis support (1-3 axes)
   - Velocity/acceleration commands
   - Query responses with numeric parsing
   - Trait mapping for Movable

2. **Newport1830C** (`config/devices/newport_1830c.toml`):
   - Simple query commands for power reading
   - Numeric response parsing (scientific notation)
   - Unit selection (W, dBm)
   - Wavelength correction
   - Trait mapping for Readable

3. **MaiTai** (`config/devices/maitai.toml`):
   - Wavelength tuning: `WAVELENGTH {nm}`
   - Shutter control: `SHUTTER OPEN`, `SHUTTER CLOSE`
   - Status queries: `?WAVELENGTH`, `?SHUTTER`, `?STATUS`
   - Power queries
   - Trait mapping for WavelengthTunable, ShutterControl, Readable

**Trait Implementations to Add:**

1. **Readable** trait for GenericSerialDriver:
   ```rust
   async fn read(&self) -> Result<f64>;
   async fn read_with_unit(&self) -> Result<(f64, String)>;
   ```

2. **WavelengthTunable** trait:
   ```rust
   async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()>;
   async fn wavelength(&self) -> Result<f64>;
   async fn wavelength_range(&self) -> Result<(f64, f64)>;
   ```

3. **ShutterControl** trait:
   ```rust
   async fn open_shutter(&self) -> Result<()>;
   async fn close_shutter(&self) -> Result<()>;
   async fn shutter_state(&self) -> Result<bool>;
   ```

**ConfiguredDriver Enum Updates:**
```rust
#[enum_dispatch(Movable, Readable, WavelengthTunable, ShutterControl)]
pub enum ConfiguredDriver {
    Ell14(GenericSerialDriver),
    Esp300(GenericSerialDriver),
    Newport1830C(GenericSerialDriver),
    MaiTai(GenericSerialDriver),
    Generic(GenericSerialDriver),
}
```

**Quality Requirements:**
- All configs validate with existing schema
- Tests compare output with existing hand-coded drivers
- No regressions in ELL14 functionality
</requirements>

<implementation>
**File Structure:**
```
config/devices/
├── ell14.toml          # Existing
├── esp300.toml         # NEW
├── newport_1830c.toml  # NEW
└── maitai.toml         # NEW

crates/daq-hardware/src/
├── drivers/
│   └── generic_serial.rs  # MODIFY: Add Readable, WavelengthTunable, ShutterControl
└── factory.rs             # MODIFY: Add new variants to ConfiguredDriver

crates/daq-hardware/tests/
├── ell14_migration.rs     # Existing
├── esp300_migration.rs    # NEW
├── newport_1830c_migration.rs  # NEW
└── maitai_migration.rs    # NEW
```

**ESP300 Protocol Details:**
```toml
# Command format: {axis}PA{position} (absolute move)
# Response: typically just confirmation or error

[commands.move_absolute]
template = "${axis}PA${position}"
parameters = { axis = "int", position = "float" }

[commands.get_position]
template = "${axis}TP?"
response = "position"
parameters = { axis = "int" }

[responses.position]
pattern = "^(?P<value>[+-]?\\d+\\.?\\d*)$"
fields = { value = { type = "float" } }
```

**Newport1830C Protocol Details:**
```toml
# Simple power meter - query and get numeric response

[commands.read_power]
template = "D?"
response = "power"

[responses.power]
pattern = "^(?P<value>[+-]?\\d+\\.?\\d*[Ee]?[+-]?\\d*)$"
fields = { value = { type = "float" } }

[trait_mapping.Readable.read]
command = "read_power"
output_field = "value"
```

**MaiTai Protocol Details:**
```toml
# Wavelength tunable laser with shutter

[commands.set_wavelength]
template = "WAVELENGTH ${wavelength_nm}"
parameters = { wavelength_nm = "float" }

[commands.get_wavelength]
template = "?WAVELENGTH"
response = "wavelength"

[commands.open_shutter]
template = "SHUTTER OPEN"

[commands.close_shutter]
template = "SHUTTER CLOSE"

[commands.get_shutter]
template = "?SHUTTER"
response = "shutter_state"

[trait_mapping.WavelengthTunable.set_wavelength]
command = "set_wavelength"
input_param = "wavelength_nm"
from_param = "wavelength"

[trait_mapping.ShutterControl.open_shutter]
command = "open_shutter"
```

**Trait Implementation Pattern:**
```rust
#[async_trait]
impl Readable for GenericSerialDriver {
    async fn read(&self) -> Result<f64> {
        self.execute_trait_method("Readable", "read", None).await?
            .ok_or_else(|| anyhow!("read() returned no value"))
    }
}

#[async_trait]
impl WavelengthTunable for GenericSerialDriver {
    async fn set_wavelength(&self, wavelength_nm: f64) -> Result<()> {
        self.execute_trait_method("WavelengthTunable", "set_wavelength", Some(wavelength_nm)).await?;
        Ok(())
    }

    async fn wavelength(&self) -> Result<f64> {
        self.execute_trait_method("WavelengthTunable", "wavelength", None).await?
            .ok_or_else(|| anyhow!("wavelength() returned no value"))
    }

    async fn wavelength_range(&self) -> Result<(f64, f64)> {
        // Read from config parameters
        let min = self.get_parameter("wavelength_min")?;
        let max = self.get_parameter("wavelength_max")?;
        Ok((min, max))
    }
}

#[async_trait]
impl ShutterControl for GenericSerialDriver {
    async fn open_shutter(&self) -> Result<()> {
        self.execute_trait_method("ShutterControl", "open_shutter", None).await?;
        Ok(())
    }

    async fn close_shutter(&self) -> Result<()> {
        self.execute_trait_method("ShutterControl", "close_shutter", None).await?;
        Ok(())
    }

    async fn shutter_state(&self) -> Result<bool> {
        let value = self.execute_trait_method("ShutterControl", "shutter_state", None).await?
            .ok_or_else(|| anyhow!("shutter_state() returned no value"))?;
        Ok(value > 0.5)  // Convert numeric to bool
    }
}
```

**Avoid:**
- Don't modify existing hand-coded drivers
- Don't implement state machines yet (Phase 4)
- Don't add Rhai scripting yet (Phase 5)
</implementation>

<output>
Create/modify files:

**New files:**
- `config/devices/esp300.toml` - ESP300 motion controller protocol
- `config/devices/newport_1830c.toml` - Power meter protocol
- `config/devices/maitai.toml` - Laser protocol
- `crates/daq-hardware/tests/esp300_migration.rs` - ESP300 tests
- `crates/daq-hardware/tests/newport_1830c_migration.rs` - Power meter tests
- `crates/daq-hardware/tests/maitai_migration.rs` - Laser tests

**Modify:**
- `crates/daq-hardware/src/drivers/generic_serial.rs` - Add Readable, WavelengthTunable, ShutterControl impls
- `crates/daq-hardware/src/factory.rs` - Add Esp300, Newport1830C, MaiTai variants
- `crates/daq-hardware/src/config/schema.rs` - Add any missing trait mapping types if needed
</output>

<verification>
Before declaring complete:

1. **Build verification:**
   ```bash
   cargo build -p daq-hardware
   cargo clippy -p daq-hardware -- -D warnings
   ```

2. **Config validation:**
   ```bash
   cargo test -p daq-hardware config::tests -- --nocapture
   ```
   - All three new configs parse and validate

3. **Unit tests:**
   ```bash
   cargo test -p daq-hardware generic_serial -- --nocapture
   cargo test -p daq-hardware esp300 -- --nocapture
   cargo test -p daq-hardware newport -- --nocapture
   cargo test -p daq-hardware maitai -- --nocapture
   ```

4. **Migration tests:**
   - ESP300: `1PA100.0` command format correct
   - Newport1830C: Power reading parses scientific notation
   - MaiTai: Wavelength and shutter commands correct

5. **ELL14 regression:**
   ```bash
   cargo test -p daq-hardware ell14 -- --nocapture
   ```
   - All existing ELL14 tests still pass

6. **Trait dispatch:**
   - ConfiguredDriver dispatches to correct trait impl based on protocol
</verification>

<summary_requirements>
Create `.prompts/005-driver-plugins-phase3/SUMMARY.md` with:

**One-liner:** [Description of pattern validation results]
**Version:** v1
**Configs Created:**
- List each config with command count
**Traits Implemented:**
- List new trait implementations
**Tests Added:**
- Test coverage per device
**Pattern Validation:**
- Did the pattern work for all three devices?
- Any schema extensions needed?
- Any edge cases discovered?
**Next Step:** Execute Phase 4 - State Machines + Init Sequences
</summary_requirements>

<success_criteria>
- [ ] esp300.toml parses and validates
- [ ] newport_1830c.toml parses and validates
- [ ] maitai.toml parses and validates
- [ ] Readable trait works for Newport1830C
- [ ] WavelengthTunable trait works for MaiTai
- [ ] ShutterControl trait works for MaiTai
- [ ] Movable trait works for ESP300
- [ ] ConfiguredDriver dispatches all traits correctly
- [ ] All ELL14 tests still pass (no regression)
- [ ] cargo build/clippy/test pass
- [ ] SUMMARY.md created with pattern validation notes
</success_criteria>

<efficiency>
**Parallel operations:**
- Read all three existing drivers in parallel to understand protocols
- Create all three config files can be parallelized
- Run test suites in parallel

**Extended thinking for:**
- Protocol differences and how schema handles them
- Trait method to config mapping design
- Edge cases in response parsing
</efficiency>
