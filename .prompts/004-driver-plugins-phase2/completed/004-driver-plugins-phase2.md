<objective>
Implement Phase 2: GenericSerialDriver + ELL14 Migration (MVP)

Purpose: Create a working config-driven driver that can replace the hand-coded ELL14 driver, proving the declarative pattern works end-to-end.

Output: GenericSerialDriver, ELL14 TOML config, DriverFactory, and comparison tests.
</objective>

<context>
Research findings: @.prompts/001-driver-plugins-research/driver-plugins-research.md
Implementation plan: @.prompts/002-driver-plugins-plan/driver-plugins-plan.md
Phase 1 output: @crates/daq-hardware/src/config/ (schema, validation, loader)

Existing ELL14 driver to reference:
@crates/daq-hardware/src/drivers/ell14.rs

Key patterns from research:
- `enum_dispatch` for zero-overhead polymorphism (10x faster than trait objects)
- Command template interpolation with `${param}` syntax
- Regex-based response parsing with named capture groups
- `evalexpr` for unit conversion formulas

Phase 1 deliverables available:
- `DeviceConfig` and all schema structs
- `load_device_config()` function
- Validation for regex patterns and evalexpr formulas
- JSON schema at `config/schemas/device.schema.json`
</context>

<requirements>
**Functional Requirements:**

1. **ELL14 TOML Config** (`config/devices/ell14.toml`):
   - Complete protocol definition matching existing driver
   - All commands: ma, mr, gp, gs, ho, fw, bw, sj, gj, in, st
   - Response patterns for position, status, device_info, jog_step
   - Calibration conversion: degrees ↔ pulses
   - Error code mapping
   - Trait mapping for Movable

2. **GenericSerialDriver** (`crates/daq-hardware/src/drivers/generic_serial.rs`):
   - Construct from `DeviceConfig`
   - Serial port management (shared port support for RS-485 multidrop)
   - Command formatting with template interpolation
   - Response parsing (regex mode first, add others in Phase 3)
   - Unit conversion via evalexpr
   - Error handling with device-specific error codes
   - Async/await throughout (tokio)

3. **Capability Trait Implementations:**
   - `Movable` for GenericSerialDriver (move_abs, move_rel, position, stop, wait_settled)
   - `Parameterized` for device parameters
   - Trait method dispatch based on `trait_mapping` config

4. **ConfiguredDriver Enum** with enum_dispatch:
   ```rust
   #[enum_dispatch(Movable, Parameterized)]
   pub enum ConfiguredDriver {
       Ell14(GenericSerialDriver),
       Generic(GenericSerialDriver),
   }
   ```

5. **DriverFactory** (`crates/daq-hardware/src/factory.rs`):
   - `create_driver(config: &DeviceConfig, port: impl AsyncSerial) -> Result<ConfiguredDriver>`
   - `create_driver_from_file(path: &Path, port_path: &str) -> Result<ConfiguredDriver>`

6. **Migration Tests:**
   - Mock serial port tests comparing command output
   - Response parsing comparison tests
   - Integration tests on real hardware (maitai)

**Quality Requirements:**
- All public types documented
- Error messages include context (command name, expected vs actual)
- No performance regression vs hand-coded driver
- Comprehensive test coverage

**Constraints:**
- Keep existing Ell14Driver unchanged (deprecation in Phase 3+)
- Support RS-485 multidrop (shared port, address prefix)
- Async-compatible with tokio
</requirements>

<implementation>
**File Structure:**
```
crates/daq-hardware/
├── Cargo.toml                      # Add enum_dispatch
└── src/
    ├── drivers/
    │   ├── mod.rs                  # Add generic_serial export
    │   ├── ell14.rs                # Unchanged
    │   └── generic_serial.rs       # NEW: GenericSerialDriver
    ├── factory.rs                  # NEW: DriverFactory
    └── lib.rs                      # Add factory export

config/devices/
└── ell14.toml                      # NEW: ELL14 protocol config

crates/daq-hardware/tests/
└── ell14_migration.rs              # NEW: Comparison tests
```

**Key Implementation Details:**

1. **Command Template Engine:**
   ```rust
   impl GenericSerialDriver {
       /// Format command from template
       fn format_command(&self, cmd_name: &str, params: &HashMap<String, Value>) -> Result<String> {
           let cmd_config = self.config.commands.get(cmd_name)
               .ok_or_else(|| Error::UnknownCommand(cmd_name.to_string()))?;

           let mut result = cmd_config.template.clone();

           // Replace ${address} with device address
           result = result.replace("${address}", &self.address);

           // Replace ${param} and ${param:format} placeholders
           for (name, value) in params {
               let placeholder = format!("${{{}}}", name);
               let formatted = self.format_value(value, &cmd_config.parameters.get(name))?;
               result = result.replace(&placeholder, &formatted);

               // Handle format specifiers like ${position:08X}
               let format_placeholder = regex::Regex::new(&format!(r"\$\{{{name}:([^}}]+)\}}"))?;
               // ... apply format
           }

           Ok(result)
       }
   }
   ```

2. **Response Parsing:**
   ```rust
   fn parse_response(&self, response_name: &str, data: &str) -> Result<HashMap<String, Value>> {
       let resp_config = self.config.responses.get(response_name)
           .ok_or_else(|| Error::UnknownResponse(response_name.to_string()))?;

       let re = regex::Regex::new(&resp_config.pattern)?;
       let caps = re.captures(data)
           .ok_or_else(|| Error::ResponseMismatch {
               expected: resp_config.pattern.clone(),
               actual: data.to_string()
           })?;

       let mut fields = HashMap::new();
       for (name, field_config) in &resp_config.fields {
           if let Some(m) = caps.name(name) {
               let value = self.parse_field(m.as_str(), field_config)?;
               fields.insert(name.clone(), value);
           }
       }

       Ok(fields)
   }
   ```

3. **Unit Conversion:**
   ```rust
   fn apply_conversion(&self, conv_name: &str, input_name: &str, input_value: f64) -> Result<f64> {
       let conv = self.config.conversions.get(conv_name)
           .ok_or_else(|| Error::UnknownConversion(conv_name.to_string()))?;

       // Build context with parameters and input
       let mut context = evalexpr::HashMapContext::new();
       context.set_value(input_name.into(), evalexpr::Value::Float(input_value))?;

       // Add device parameters to context
       for (name, param) in &self.parameters {
           context.set_value(name.clone(), param.to_evalexpr_value())?;
       }

       evalexpr::eval_float_with_context(&conv.formula, &context)
           .map_err(|e| Error::ConversionFailed { formula: conv.formula.clone(), error: e.to_string() })
   }
   ```

4. **Movable Trait Implementation:**
   ```rust
   #[async_trait]
   impl Movable for GenericSerialDriver {
       async fn move_abs(&self, position: f64) -> Result<()> {
           let mapping = self.get_trait_mapping("Movable", "move_abs")?;

           // Apply input conversion (degrees -> pulses)
           let converted = if let Some(conv_name) = &mapping.input_conversion {
               self.apply_conversion(conv_name, &mapping.from_param.unwrap_or("position".into()), position)?
           } else {
               position
           };

           // Build params and send command
           let mut params = HashMap::new();
           params.insert(mapping.input_param.clone(), Value::Int(converted.round() as i64));

           self.send_command(&mapping.command, &params).await
       }

       async fn position(&self) -> Result<f64> {
           let mapping = self.get_trait_mapping("Movable", "position")?;

           let response = self.send_query(&mapping.command).await?;
           let fields = self.parse_response(&mapping.command, &response)?;

           let raw_value = fields.get(&mapping.output_field.unwrap_or("position".into()))
               .ok_or(Error::MissingField)?
               .as_f64()?;

           // Apply output conversion (pulses -> degrees)
           if let Some(conv_name) = &mapping.output_conversion {
               self.apply_conversion(conv_name, "pulses", raw_value)
           } else {
               Ok(raw_value)
           }
       }

       // ... stop, move_rel, wait_settled
   }
   ```

5. **enum_dispatch Setup:**
   ```rust
   // In crates/daq-hardware/src/drivers/mod.rs
   use enum_dispatch::enum_dispatch;
   use crate::capabilities::{Movable, Parameterized};

   #[enum_dispatch(Movable, Parameterized)]
   pub enum ConfiguredDriver {
       Ell14(GenericSerialDriver),
       Generic(GenericSerialDriver),
   }
   ```

**Dependencies to Add:**
```toml
# crates/daq-hardware/Cargo.toml
[dependencies]
enum_dispatch = "0.3"
# evalexpr already added in Phase 1
```

**Avoid:**
- Don't modify existing Ell14Driver code
- Don't implement state machines yet (Phase 4)
- Don't add Rhai scripting yet (Phase 5)
- Don't create configs for other devices yet (Phase 3)
</implementation>

<output>
Create/modify files:

**New files:**
- `crates/daq-hardware/src/drivers/generic_serial.rs` - GenericSerialDriver implementation
- `crates/daq-hardware/src/factory.rs` - DriverFactory
- `config/devices/ell14.toml` - Complete ELL14 protocol definition
- `crates/daq-hardware/tests/ell14_migration.rs` - Comparison tests

**Modify:**
- `crates/daq-hardware/Cargo.toml` - Add enum_dispatch
- `crates/daq-hardware/src/drivers/mod.rs` - Add generic_serial, ConfiguredDriver enum
- `crates/daq-hardware/src/lib.rs` - Add factory export
</output>

<verification>
Before declaring complete:

1. **Build verification:**
   ```bash
   cargo build -p daq-hardware
   cargo clippy -p daq-hardware -- -D warnings
   ```

2. **Unit tests:**
   ```bash
   cargo test -p daq-hardware generic_serial -- --nocapture
   cargo test -p daq-hardware factory -- --nocapture
   ```

3. **Migration tests (mock):**
   ```bash
   cargo test -p daq-hardware ell14_migration -- --nocapture
   ```

4. **Command output comparison:**
   - `move_abs(45.0)` produces `2ma00004650` (for address "2", 45° = 17,488 pulses = 0x4450)
   - `get_position` produces `2gp`
   - Response `2PO00004650` parses to position ≈ 45.0°

5. **Hardware test (if available):**
   ```bash
   # On maitai remote
   cargo test --features hardware_tests test_generic_ell14 -- --nocapture
   ```

6. **Edge cases:**
   - Negative positions (signed hex encoding)
   - Maximum position (360°)
   - Error response handling
   - Timeout handling
</verification>

<summary_requirements>
Create `.prompts/004-driver-plugins-phase2/SUMMARY.md` with:

**One-liner:** [Description of what was implemented]
**Version:** v1
**Files Created:**
- List each file with brief description
**Files Modified:**
- List modifications
**Tests Added:**
- Test coverage summary
**ELL14 Commands Implemented:**
- List all commands in ell14.toml
**Trait Methods Working:**
- List Movable methods verified
**Issues Encountered:**
- Problems and resolutions
**Next Step:** Execute Phase 3 - ESP300 + Additional Capabilities
</summary_requirements>

<success_criteria>
- [ ] ell14.toml parses and validates successfully
- [ ] GenericSerialDriver constructs from config
- [ ] Command formatting matches existing driver output
- [ ] Response parsing extracts correct values
- [ ] Unit conversion (degrees ↔ pulses) is accurate
- [ ] Movable trait works (move_abs, position, stop)
- [ ] enum_dispatch compiles with async_trait
- [ ] Migration tests pass (mock serial)
- [ ] cargo build/clippy/test pass
- [ ] SUMMARY.md created
</success_criteria>

<efficiency>
**Parallel operations:**
- Read ell14.rs and config schema in parallel
- Run multiple test categories in parallel

**Extended thinking for:**
- Hex encoding/decoding for signed integers
- Template format specifier parsing
- Error recovery patterns
</efficiency>
