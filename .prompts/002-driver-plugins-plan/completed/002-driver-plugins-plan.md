<objective>
Create an implementation roadmap for a declarative, config-driven hardware driver plugin system.

Purpose: Guide phased implementation of TOML-based driver definitions with factory pattern, replacing hand-coded Rust drivers with a flexible, user-extensible system.

Input: Research findings from driver-plugins-research.md
Output: driver-plugins-plan.md with 4-6 implementation phases
</objective>

<context>
Research findings: @.prompts/001-driver-plugins-research/driver-plugins-research.md

Key findings to incorporate:
- **Hybrid architecture:** TOML protocol definitions + code-generated trait implementations
- **Dispatch method:** enum_dispatch for 10x performance over trait objects
- **State machines:** smlang for config-defined structure with proc-macro code generation
- **Validation:** serde_valid + schemars for integrated config validation
- **Config format:** TOML (already used in rust-daq, better type safety)

Existing codebase context:
- Current drivers: @crates/daq-hardware/src/drivers/ (ell14.rs, esp300.rs, maitai.rs, newport_1830c.rs)
- Capability traits: @crates/daq-hardware/src/capabilities.rs (Movable, Readable, FrameProducer, etc.)
- Current config: @config/config.v4.toml (Figment-based configuration)
- Registry: @crates/daq-hardware/src/registry.rs (DeviceRegistry pattern)
</context>

<planning_requirements>
**Must address:**
1. TOML schema specification for device protocol definitions
2. GenericSerialDriver struct that implements capability traits based on config
3. Factory pattern using enum_dispatch for driver instantiation
4. Migration path from existing ELL14 driver to config-based definition
5. Validation and error handling strategy
6. Test strategy including integration tests with mock serial ports

**Constraints:**
- Maintain backward compatibility during migration (existing drivers continue to work)
- Preserve async/await patterns used throughout daq-hardware
- No runtime performance regression (enum_dispatch addresses this)
- Support both RS-232 and RS-485 protocols
- Config files must be human-readable for instrument users

**Decisions to make (use research recommendations):**
- MVP scope: Start with ELL14, include ESP300 in Phase 2 for pattern validation
- Config location: `config/devices/*.toml` (consistent with existing config.v4.toml location)
- Scripting: Defer Rhai fallback to later phase (Phase 4+)
- Binary protocols: Defer Modbus to later phase (Phase 4+)

**Success criteria for the planned outcome:**
- User can define a new serial instrument by creating a TOML file
- Existing capability traits (Movable, Readable, etc.) work with config-defined drivers
- Adding a new instrument requires no Rust code changes
- Compile-time validation of config structure via schemars
- Clear error messages when config is malformed
</planning_requirements>

<output_structure>
Save to: `.prompts/002-driver-plugins-plan/driver-plugins-plan.md`

Structure the plan using this XML format:

```xml
<plan>
  <summary>
    {One paragraph overview of the approach, referencing research recommendations}
  </summary>

  <architecture>
    {Diagram or description of the target architecture}
    {Key structs, traits, and their relationships}
  </architecture>

  <phases>
    <phase number="1" name="{phase-name}">
      <objective>{What this phase accomplishes}</objective>
      <tasks>
        <task priority="high">{Specific actionable task with file paths}</task>
        <task priority="medium">{Another task}</task>
      </tasks>
      <deliverables>
        <deliverable>{What's produced - files, tests, etc.}</deliverable>
      </deliverables>
      <dependencies>{What must exist before this phase}</dependencies>
      <verification>{How to verify this phase is complete}</verification>
      <execution_notes>
        {Guidance for the implementing Claude}
        {Key patterns to follow, pitfalls to avoid}
      </execution_notes>
    </phase>
    <!-- Additional phases -->
  </phases>

  <toml_schema>
    {Complete TOML schema specification with examples}
    {Cover: connection, commands, responses, validation, state machine}
  </toml_schema>

  <migration_guide>
    {Step-by-step guide for migrating existing drivers}
    {Use ELL14 as the canonical example}
  </migration_guide>

  <metadata>
    <confidence level="{high|medium|low}">
      {Why this confidence level}
    </confidence>
    <dependencies>
      {External crate dependencies needed}
    </dependencies>
    <open_questions>
      {Uncertainties that may affect execution}
    </open_questions>
    <assumptions>
      {What was assumed in creating this plan}
    </assumptions>
    <risks>
      <risk severity="high">{Risk description}</risk>
      <mitigation>{How to address}</mitigation>
    </risks>
  </metadata>
</plan>
```
</output_structure>

<summary_requirements>
Create `.prompts/002-driver-plugins-plan/SUMMARY.md` with:

**One-liner:** [Substantive description of the implementation approach]
**Version:** v1
**Phase Overview:** [Brief description of each phase with objectives]
**Key Decisions Made:** [Decisions resolved from research open questions]
**Assumptions Needing Validation:** [What might need adjustment during implementation]
**Blockers:** [External impediments]
**Next Step:** Execute Phase 1 - Core Infrastructure
</summary_requirements>

<success_criteria>
- Plan addresses all requirements from research
- Phases are independently testable and executable by single prompts
- TOML schema is complete with examples for all config sections
- Migration guide is specific to ELL14 with clear steps
- Each phase includes verification criteria
- Metadata captures risks and mitigations
- SUMMARY.md provides clear phase overview
- Ready for implementation prompts to consume
</success_criteria>

<execution_guidance>
**For optimal output:**
- Use extended thinking for architectural decisions
- Reference specific files from the research when applicable
- Include code snippets for key struct/trait definitions in the plan
- Make phases small enough to execute in a single session
- Include specific file paths for all deliverables

**Parallel operations:**
- Read existing driver files in parallel to understand patterns
- Read capability traits and registry in parallel for interface design
</execution_guidance>
