<objective>
Implement Phase 1: Core Infrastructure for the declarative driver plugin system.

Purpose: Establish config schema structs, validation, and loading mechanisms as the foundation for config-driven hardware drivers.

Output: DeviceConfig schema structs, validation, config loader, and JSON schema generation.
</objective>

<context>
Research findings: @.prompts/001-driver-plugins-research/driver-plugins-research.md
Implementation plan: @.prompts/002-driver-plugins-plan/driver-plugins-plan.md

Key patterns to follow from research:
- Use `serde_valid` for integrated validation with serde
- Use `schemars` for JSON schema generation (IDE support)
- TOML format for config files (consistent with existing config.v4.toml)
- Figment for config loading (already used in the project)

Existing codebase context:
- Current config: @config/config.v4.toml (Figment-based)
- Config module: @crates/rust-daq/src/config_v4.rs (pattern to follow)
- Existing drivers: @crates/daq-hardware/src/drivers/ell14.rs (reference for protocol details)
</context>

<requirements>
**Functional Requirements:**

1. **DeviceConfig schema structs** covering:
   - `DeviceIdentity`: name, description, manufacturer, model, protocol, category, capabilities list
   - `ConnectionConfig`: serial settings (baud, parity, stop bits, flow control, timeout, terminators, bus config)
   - `CommandConfig`: template string, description, parameters map, optional response reference
   - `ResponseConfig`: pattern (regex or delimiter), fields with types, parsing mode
   - `ConversionConfig`: formula expressions for unit conversion
   - `ParameterConfig`: type, default, range, unit, description
   - `ValidationRule`: range validation, pattern matching
   - `TraitMapping`: maps capability trait methods to commands

2. **Validation with serde_valid:**
   - Range validation for numeric fields (baud_rate, timeout_ms)
   - Pattern validation for addresses, command templates
   - Custom validator for response patterns (valid regex check)
   - Custom validator for conversion formulas (valid evalexpr check)

3. **JSON Schema generation with schemars:**
   - Derive `JsonSchema` on all config structs
   - Export schema to `config/schemas/device.schema.json`
   - Include descriptions for IDE documentation

4. **Config loader module:**
   - `load_device_config(path: &Path) -> Result<DeviceConfig>`
   - `load_all_devices(dir: &Path) -> Result<Vec<DeviceConfig>>`
   - Validate after loading with clear error messages

**Quality Requirements:**
- All structs derive: Serialize, Deserialize, Debug, Clone, JsonSchema, Validate
- Error messages include field paths for debugging
- Comprehensive unit tests for parsing and validation
- Documentation comments on all public types

**Constraints:**
- No changes to existing driver code in this phase
- Keep daq-hardware as the home for this module (not common)
- Support both TOML and YAML parsing (TOML primary)
</requirements>

<implementation>
**File Structure:**
```
crates/daq-hardware/
├── Cargo.toml  # Add dependencies
└── src/
    └── config/
        ├── mod.rs           # Re-exports, DeviceConfig struct
        ├── schema.rs        # All schema structs
        ├── validation.rs    # Custom validators
        ├── loader.rs        # Config loading functions
        └── tests.rs         # Unit tests

config/
├── schemas/
│   └── device.schema.json   # Generated JSON schema
└── devices/
    └── .gitkeep             # Placeholder for device configs
```

**Key Implementation Details:**

1. **Command Template Format:**
   Use `${param}` syntax with optional format specifiers: `${position:08X}` for hex formatting.

   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
   pub struct CommandConfig {
       /// Command template with ${param} placeholders
       #[validate(custom(function = "validate_template"))]
       pub template: String,

       /// Human-readable description
       pub description: Option<String>,

       /// Parameter types for this command
       #[serde(default)]
       pub parameters: HashMap<String, ParameterType>,

       /// Reference to response definition for query commands
       pub response: Option<String>,
   }
   ```

2. **Response Pattern Parsing:**
   Support three modes: regex with named groups, delimiter-separated, fixed-position.

   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
   #[serde(tag = "mode", rename_all = "snake_case")]
   pub enum ResponsePattern {
       Regex {
           #[validate(custom(function = "validate_regex"))]
           pattern: String,
           fields: HashMap<String, FieldConfig>,
       },
       Delimiter {
           delimiter: String,
           fields: Vec<FieldConfig>,
       },
       Fixed {
           positions: Vec<FixedField>,
       },
   }
   ```

3. **Type System for Fields:**
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
   #[serde(rename_all = "snake_case")]
   pub enum FieldType {
       String,
       Int,
       Int32,
       Int64,
       Uint,
       Uint32,
       Uint64,
       Float,
       Float64,
       Bool,
       HexI32 { signed: bool },
       HexU8,
       HexU16,
       HexU32,
   }
   ```

4. **Validation Helpers:**
   ```rust
   // In validation.rs
   pub fn validate_regex(pattern: &str) -> Result<(), ValidationError> {
       regex::Regex::new(pattern)
           .map(|_| ())
           .map_err(|e| ValidationError::Custom(format!("Invalid regex: {}", e)))
   }

   pub fn validate_evalexpr(formula: &str) -> Result<(), ValidationError> {
       // Check formula syntax with evalexpr
       evalexpr::build_operator_tree(formula)
           .map(|_| ())
           .map_err(|e| ValidationError::Custom(format!("Invalid formula: {}", e)))
   }
   ```

**Avoid:**
- Don't implement GenericSerialDriver in this phase (Phase 2)
- Don't modify existing ell14.rs driver
- Don't add enum_dispatch yet (Phase 2)
- Don't create actual device config files yet (Phase 2)

**Dependencies to Add (Cargo.toml):**
```toml
[dependencies]
serde_valid = "0.24"
schemars = "0.8"
evalexpr = "11"
# regex already present
# serde, toml, figment already present
```
</implementation>

<output>
Create/modify files:

**New files:**
- `crates/daq-hardware/src/config/mod.rs` - Module root with DeviceConfig and re-exports
- `crates/daq-hardware/src/config/schema.rs` - All schema struct definitions
- `crates/daq-hardware/src/config/validation.rs` - Custom validators
- `crates/daq-hardware/src/config/loader.rs` - Config loading functions
- `config/schemas/device.schema.json` - Generated JSON schema
- `config/devices/.gitkeep` - Placeholder directory

**Modify:**
- `crates/daq-hardware/Cargo.toml` - Add dependencies
- `crates/daq-hardware/src/lib.rs` - Add `pub mod config;`

**Test files:**
- `crates/daq-hardware/src/config/tests.rs` - Unit tests (inline or separate)
</output>

<verification>
Before declaring complete:

1. **Build verification:**
   ```bash
   cargo build -p daq-hardware
   cargo clippy -p daq-hardware -- -D warnings
   ```

2. **Test verification:**
   ```bash
   cargo test -p daq-hardware config:: -- --nocapture
   ```

3. **Schema generation:**
   - Verify `config/schemas/device.schema.json` is valid JSON
   - Verify schema includes descriptions and type info

4. **Manual verification:**
   - Create a test TOML file with valid config → parses successfully
   - Create a test TOML file with invalid regex → produces clear error
   - Create a test TOML file with out-of-range baud rate → produces clear error

5. **Edge cases:**
   - Empty config file → appropriate error
   - Missing required fields → error with field path
   - Invalid TOML syntax → parse error
   - Valid YAML file → parses successfully (if YAML support enabled)
</verification>

<summary_requirements>
Create `.prompts/003-driver-plugins-phase1/SUMMARY.md` with:

**One-liner:** [Description of what was implemented]
**Version:** v1
**Files Created:**
- List each file with brief description
**Files Modified:**
- List each modified file with what changed
**Tests Added:**
- List test coverage
**Decisions Made:**
- Any implementation decisions not in plan
**Issues Encountered:**
- Any problems and how they were resolved
**Next Step:** Execute Phase 2 - GenericSerialDriver + ELL14 Migration
</summary_requirements>

<success_criteria>
- [ ] All schema structs defined with proper serde/schemars derives
- [ ] Validation works for regex patterns, evalexpr formulas, numeric ranges
- [ ] Config loader loads TOML files and validates them
- [ ] JSON schema generated and valid
- [ ] `cargo build -p daq-hardware` succeeds
- [ ] `cargo clippy -p daq-hardware` passes with no warnings
- [ ] Unit tests pass for parsing valid configs
- [ ] Unit tests pass for rejecting invalid configs with clear errors
- [ ] SUMMARY.md created with files list
</success_criteria>

<efficiency>
**Parallel operations:**
- Read existing config_v4.rs and ell14.rs in parallel for patterns
- Read Cargo.toml files in parallel if checking dependencies

**Extended thinking for:**
- Schema design decisions (field types, validation rules)
- Error message formatting for user-friendly output
</efficiency>
