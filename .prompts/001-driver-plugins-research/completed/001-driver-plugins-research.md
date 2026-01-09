<session_initialization>
Before beginning research, verify today's date:
!`date +%Y-%m-%d`

Use this date when searching for "current" or "latest" information.
</session_initialization>

<research_objective>
Research declarative, config-driven plugin architectures for hardware drivers in Rust.

Purpose: Inform the design of a factory-based driver plugin system where device behavior (commands, parsing, state machines, validation) is defined in TOML/YAML configuration files rather than Rust code.

Scope: Patterns, libraries, and approaches for:
- Declarative hardware driver definition
- Config-driven command/response parsing
- State machine DSLs expressible in config
- Factory patterns for runtime driver instantiation

Output: driver-plugins-research.md with structured findings
</research_objective>

<context>
This research is for the rust-daq project - a Rust-based data acquisition system for scientific instrumentation.

Current driver architecture:
@crates/daq-hardware/src/drivers/ell14.rs (Thorlabs ELL14 rotators)
@crates/daq-hardware/src/drivers/esp300.rs (Newport ESP300 motion controller)
@crates/daq-hardware/src/drivers/maitai.rs (Spectra-Physics MaiTai laser)
@crates/daq-hardware/src/drivers/newport_1830c.rs (Newport 1830-C power meter)
@crates/daq-driver-pvcam/ (Photometrics PVCAM camera)

Capability traits:
@crates/daq-hardware/src/capabilities.rs (Movable, Readable, FrameProducer, etc.)

The goal is to replace hand-coded Rust drivers with a declarative system where:
1. Device protocols are defined in TOML/YAML config files
2. A factory instantiates drivers from config at runtime
3. Users can add new instruments without writing Rust code
4. Complex behavior (state machines, validation, unit conversion) is expressible in config
</context>

<research_scope>
<include>
**Declarative Driver Patterns:**
- How other projects define device protocols declaratively
- Embedded/IoT driver definition approaches
- Scientific instrumentation frameworks (PyMeasure, Instrument Control, Bluesky)

**Config Schema Design:**
- TOML/YAML patterns for defining:
  - Serial command templates with placeholders
  - Response parsing with regex/patterns
  - Unit conversions and calibration
  - Validation rules and limits
  - State machines (e.g., device initialization sequences)
- Serde patterns for flexible config parsing
- Config validation (schemars, validator crate)

**Factory/Plugin Architecture:**
- Rust factory patterns for runtime object creation
- Plugin systems in Rust (libloading, abi_stable, dlopen)
- Trait object patterns for heterogeneous driver collections
- How to map config to trait implementations

**State Machine DSLs:**
- Expressing state machines in config (not code)
- Existing Rust crates: rust-fsm, smlang, statig
- How to drive state transitions from config definitions

**Similar Projects:**
- linuxcnc HAL (Hardware Abstraction Layer)
- OpenOCD (Open On-Chip Debugger) - config-driven
- Home Assistant integrations (YAML-defined devices)
- Bluesky/ophyd (Python scientific instrumentation)
- LabVIEW instrument drivers (VI-based but declarative)

**Rust Ecosystem:**
- Crates for serial protocol definition
- Crates for config-driven behavior
- Async patterns compatible with declarative definitions
</include>

<exclude>
- Specific implementation details (for planning phase)
- UI/visualization concerns
- Storage/logging considerations
- gRPC/networking aspects
</exclude>

<sources>
Official documentation (use WebFetch):
- https://docs.rs/serde/latest/serde/
- https://docs.rs/toml/latest/toml/
- https://docs.rs/config/latest/config/
- https://docs.rs/schemars/latest/schemars/
- https://docs.rs/rust-fsm/latest/rust_fsm/
- https://docs.rs/statig/latest/statig/
- https://docs.rs/smlang/latest/smlang/

Search queries for WebSearch:
- "declarative hardware driver rust {current_year}"
- "config-driven device protocol rust"
- "TOML YAML state machine DSL"
- "rust factory pattern trait objects"
- "scientific instrumentation rust framework"
- "embedded rust declarative driver"
- "PyMeasure instrument driver architecture"
- "Bluesky ophyd device definition"

GitHub repos to explore:
- rust embedded ecosystem
- scientific Rust projects
- config-driven applications
</sources>
</research_scope>

<verification_checklist>
**Config Schema Options:**
- [ ] Document at least 3 approaches to defining device protocols in config
- [ ] Verify TOML vs YAML trade-offs for this use case
- [ ] Find examples of command template syntax with variable substitution
- [ ] Find examples of response parsing patterns in config

**State Machine Expression:**
- [ ] Document at least 2 ways to express state machines in config
- [ ] Verify which Rust FSM crates support declarative definitions
- [ ] Check if any crates support config-driven state machine generation

**Factory/Plugin Patterns:**
- [ ] Document trait object factory patterns in Rust
- [ ] Verify dynamic dispatch overhead for our use case
- [ ] Check plugin loading options (static vs dynamic)

**Similar Projects:**
- [ ] Analyze PyMeasure's instrument driver architecture
- [ ] Check Home Assistant's YAML device definition approach
- [ ] Look at Bluesky/ophyd's device abstraction

**Rust Ecosystem:**
- [ ] Identify relevant crates for each concern
- [ ] Check crate maintenance status and adoption
- [ ] Verify async compatibility
</verification_checklist>

<research_quality_assurance>
Before completing research, perform these checks:

<completeness_check>
- [ ] All config schema options documented with examples
- [ ] State machine approaches evaluated for config expressibility
- [ ] Factory patterns compared with trade-offs
- [ ] At least 3 similar projects analyzed for patterns
</completeness_check>

<source_verification>
- [ ] Primary claims backed by official docs or authoritative sources
- [ ] Crate recommendations verified with recent downloads/maintenance
- [ ] Code examples tested or from official documentation
- [ ] Distinguish verified facts from assumptions
</source_verification>

<blind_spots_review>
Ask yourself: "What might I have missed?"
- [ ] Are there scientific instrumentation Rust projects I didn't find?
- [ ] Did I check embedded Rust ecosystem for relevant patterns?
- [ ] Did I consider serialization formats beyond TOML/YAML?
- [ ] Did I look at how async/await interacts with declarative definitions?
</blind_spots_review>
</research_quality_assurance>

<output_requirements>
Write findings incrementally to driver-plugins-research.md as you discover them:

1. Create the file with this initial structure:
   ```xml
   <research>
     <summary>[Will complete at end]</summary>
     <findings></findings>
     <recommendations></recommendations>
     <code_examples></code_examples>
     <metadata></metadata>
   </research>
   ```

2. As you research each aspect, immediately append findings:
   - Find config pattern → Write finding
   - Discover relevant crate → Write finding
   - Find code example → Append to code_examples

3. After all research complete:
   - Write summary (synthesize all findings)
   - Write recommendations (based on findings)
   - Write metadata (confidence, dependencies, etc.)

Save to: `.prompts/001-driver-plugins-research/driver-plugins-research.md`
</output_requirements>

<summary_requirements>
Create `.prompts/001-driver-plugins-research/SUMMARY.md` with:

**One-liner:** [Substantive description of key recommendation]
**Version:** v1
**Key Findings:** [3-5 actionable takeaways]
**Decisions Needed:** [What requires user input]
**Blockers:** [External impediments]
**Next Step:** Create driver-plugins-plan.md
</summary_requirements>

<success_criteria>
- All verification checklist items completed
- At least 3 config schema approaches documented with examples
- At least 2 state machine expression approaches evaluated
- Factory/plugin patterns compared with Rust-specific considerations
- Similar projects analyzed for applicable patterns
- Recommendations prioritized by feasibility and value
- SUMMARY.md created with substantive one-liner
- Quality report distinguishes verified from assumed claims
- Ready for planning phase to consume
</success_criteria>

<efficiency>
For maximum efficiency, invoke all independent tool operations simultaneously:
- Parallel WebSearch queries for different topics
- Parallel WebFetch for independent documentation pages
- Parallel file reads when gathering context

Use extended thinking for:
- Synthesizing findings across different sources
- Evaluating trade-offs between approaches
- Formulating recommendations
</efficiency>
