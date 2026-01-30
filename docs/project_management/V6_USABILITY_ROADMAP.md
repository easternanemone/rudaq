# V6 Usability Roadmap

**Date**: December 2025
**Status**: Planning Document
**Purpose**: Document usability findings and improvement roadmap for potential V6 iteration

---

## Executive Summary

The rust-daq V5 architecture is **technically excellent** but has **operational barriers** that prevent laboratory adoption by non-programmer researchers. This document captures findings from a comprehensive usability assessment (December 2025) including independent verification by Gemini AI.

**Overall Verdict**: "Architecturally excellent but operationally immature" - Gemini

### Key Finding

✅ **The codebase is NOT overly complex.** The architectural complexity is domain-appropriate for scientific instrumentation. Multiple hardware protocols, flexible storage backends, real-time visualization, and remote operation are all justified requirements.

❌ **The problem is user-facing layers.** Installation UX, configuration workflows, and documentation have not kept pace with backend maturity.

### Critical Insight: User Segmentation

| User Type | Programming Proficiency | Current Experience | Target Score |
|-----------|------------------------|-------------------|--------------|
| **Physicists/Engineers (who code)** | High | 9/10 ✅ | 10/10 |
| **Pure Biologists/Chemists** | None/Low | 3/10 ❌ | 7/10 |

**Implication**: V6 improvements should prioritize **removing barriers for non-programmers** while preserving power-user capabilities.

---

## Assessment Methodology

### Sources

1. **Primary Analysis** (Claude Code)
   - Codebase exploration (15+ crates, 50+ documentation files)
   - Interface analysis (GUI, CLI, Rhai scripting)
   - Documentation review (user guides, examples, API reference)

2. **Independent Verification** (Gemini)
   - Laboratory software comparison (LabVIEW, MATLAB, PyMeasure)
   - Industry best practices
   - User experience assessment

Both assessments reached identical conclusions on key issues.

### Evaluation Criteria

- **Accessibility**: Can a non-programmer use this?
- **Learning Curve**: Time to first experiment
- **Documentation Quality**: Completeness, clarity, examples
- **Installation Friction**: Steps required to get started
- **Configuration Complexity**: Hardware setup difficulty
- **Error Handling**: How helpful are error messages?

---

## Current State Analysis

### What Works Well ✅

**1. Backend Architecture (V5)**
- Clean capability-based hardware abstraction
- Feature-gated modular design
- gRPC for remote operation
- Async/concurrent without data races
- **Assessment**: Production-ready, well-designed, performant

**2. Rhai Scripting Abstraction**
```rhai
for i in 0..10 {
    stage.move_abs(i * 1.0);
    camera.trigger();
    sleep(0.1);
}
```
- Human-readable syntax
- No compilation required (<50ms overhead)
- Safety limits (10,000 operation max)
- Domain-appropriate vocabulary
- **Assessment**: Accessible to anyone with MATLAB/Python experience

**3. Documentation (Technical)**
- Excellent API reference (100+ pages with examples)
- Progressive complexity in examples
- Clear architecture documentation
- **Assessment**: Outstanding for developers/power users

**4. Three Complementary Interfaces**
- GUI (egui) - Visual, no coding required
- CLI - Terminal commands for automation
- Rhai Scripts - Flexible experiment control
- **Assessment**: Appropriate variety for different workflows

### Critical Barriers ❌

**1. Installation Complexity** 🚨 CRITICAL
**Current Process**:
```bash
# User must have Rust installed
rustup install stable
git clone <repo>
cd rust-daq
cargo build -p bin  # 10+ minute compile
./target/release/rust-daq-daemon --help
```

**Problem**: 99% of lab users don't have Rust toolchain and shouldn't need it.

**Impact**: This alone eliminates most potential users.

**Gemini Quote**: "Requiring `cargo build` essentially restricts your user base to Rust developers."

---

**2. Hardware Configuration UX**
**Current Process**:
1. Edit TOML file manually
2. Find serial port name (OS-specific):
   - Linux: `ls /dev/tty*` → `/dev/ttyUSB0`
   - Windows: Device Manager → `COM3`
   - macOS: `ls /dev/tty.*` → `/dev/tty.usbserial-ABC123`
3. Look up baud rate in instrument manual (19200? 9600? 115200?)
4. Understand serial concepts: flow control, parity bits, stop bits
5. Restart daemon to reload config

**Problem**: Requires knowledge of serial communication protocols. Error-prone.

**Comparison**: LabVIEW's "Measurement & Automation Explorer" (MAX) provides point-and-click device management with auto-detection.

---

**3. Documentation Gaps**

**Missing (Critical)**:
- "Getting Started for Non-Programmers"
- Hardware setup instructions (how to connect devices)
- Troubleshooting guide (common errors and solutions)
- Data export workflow (where data goes, how to import to MATLAB/Python)

**Existing Documentation Issues**:
- README assumes Rust knowledge ("cargo build -p bin")
- Guides are developer-focused (architecture diagrams, not workflows)
- No glossary (baud rate, serial port, gRPC undefined)
- Examples excellent but no "absolute beginner" tutorial

---

**4. No Visual Workflow Builder**
**Current**: Must write Rhai scripts (even for simple scans)

**Problem**: Requires basic programming literacy (variables, loops, functions)

**User Need**: Template-based or GUI-based experiment design for common patterns (linear scan, time series, grid scan)

---

## Comparison to Laboratory Software

### LabVIEW

**LabVIEW Strengths**:
- Visual programming (drag-and-drop)
- Measurement & Automation Explorer (hardware config GUI)
- "Draw a button and wire it" intuitiveness
- Immediate visual feedback

**rust-daq Advantages**:
- Avoids "spaghetti code" visual diagrams
- Type safety prevents runtime errors
- Text-based version control
- Better concurrency model

**Verdict**: rust-daq is superior for complex logic, but LabVIEW wins on approachability.

---

### Python (PyMeasure/InstrumentKit)

**Python Strengths**:
- Massive ecosystem of drivers
- Familiar to scientific community
- Powerful for data analysis (numpy, pandas, scipy)
- No compilation

**rust-daq Advantages**:
- Better concurrency and type safety
- Rhai cleaner than Python for simple loops
- Hardware abstraction via capabilities
- Performance (no GIL issues)

**Verdict**: rust-daq offers better reliability, Python offers broader ecosystem.

---

### MATLAB Instrument Control Toolbox

**MATLAB Strengths**:
- Built-in data analysis/visualization
- Simple API (`fopen(serial('COM3'))`)
- Familiar to scientists

**rust-daq Advantages**:
- Open source (no license costs)
- Modern async architecture
- Better error handling
- More powerful scripting

**Verdict**: rust-daq is more powerful, MATLAB is more accessible.

---

## V6 Improvement Roadmap

### Guiding Principles

1. **Preserve Backend Excellence**: V5 architecture is solid, don't regress
2. **Lower Barriers**: Focus on installation, configuration, documentation
3. **Quick Wins First**: Prioritize high-impact, low-effort improvements
4. **Power Users Protected**: Don't sacrifice advanced features for simplicity

### Priority Tiers

#### IMMEDIATE (1-2 weeks)

**1. Pre-compiled Binaries** 🚨 CRITICAL
- **Impact**: Eliminates #1 adoption barrier
- **Effort**: Low (GitHub Actions automation)
- **Deliverable**:
  - `rust-daq-daemon` (Windows, macOS, Linux)
  - `rust-daq-gui` (Windows, macOS, Linux)
  - GitHub Releases page with download links
- **Implementation**:
  ```yaml
  # .github/workflows/release.yml
  name: Release Binaries
  on:
    push:
      tags:
        - 'v*'
  jobs:
    build:
      strategy:
        matrix:
          os: [ubuntu-latest, macos-latest, windows-latest]
      # ... cross-compile and upload
  ```

**2. "Mock Mode" Getting Started Guide** 🎯 QUICK WIN
- **Impact**: Users can try software in 5 minutes without hardware
- **Effort**: Very Low (documentation only, no code changes)
- **Deliverable**: `docs/user_guides/getting_started_mock.md`
- **Content**:
  1. Download binary
  2. Run GUI
  3. Load mock devices (`MockStage`, `MockCamera`)
  4. Execute simple scan script
  5. View data output
- **Key**: Use screenshots, assume zero programming knowledge

**3. Troubleshooting FAQ**
- **Impact**: Reduces support burden
- **Effort**: Low (documentation)
- **Deliverable**: `docs/user_guides/troubleshooting.md`
- **Sections**:
  - Installation issues (Rust not found, compilation errors)
  - Serial port permissions (Linux: dialout group, Windows: driver issues)
  - Hardware not found (port names, baud rates)
  - Common script errors (syntax, runtime)
  - OS-specific troubleshooting

---

#### HIGH PRIORITY (1-2 months)

**4. GUI Configuration Editor Panel**
- **Impact**: Matches LabVIEW's MAX tool UX
- **Effort**: Medium (new GUI panel)
- **Deliverable**: "Configuration" panel in `ui`
- **Features**:
  - Dropdown of available serial ports (`serialport::available_ports()`)
  - Form-based device setup:
    - Select driver from list
    - Select port from dropdown
    - Configure parameters (baud, timeout, etc.)
  - Validation before saving
  - Generate/edit `hardware.toml` automatically
- **Leverage Existing**: `hardware` has `validate_driver_config` logic
- **File**: `crates/ui/src/panels/configuration.rs`

**5. Hardware Setup Guides (per instrument)**
- **Impact**: Step-by-step reduces setup friction
- **Effort**: Medium (photos/diagrams required)
- **Deliverable**: `docs/user_guides/hardware_setup/*.md`
- **Guides Needed**:
  - `newport_esp300.md` - Motion controller setup
  - `elliptec_ell14.md` - Rotation stage connection
  - `newport_1830c.md` - Power meter configuration
  - `pvcam_prime_bsi.md` - Camera installation
  - `maitai.md` - Laser interface
- **Each Guide Includes**:
  - Photos of device and cables
  - Connection diagram
  - Driver installation (if needed)
  - Port identification (OS-specific)
  - Configuration example
  - Troubleshooting

**6. Data Export Documentation**
- **Impact**: Critical missing workflow piece
- **Effort**: Low (documentation + examples)
- **Deliverable**: `docs/user_guides/data_export.md`
- **Content**:
  - Where data is saved (default paths)
  - File formats (CSV structure, HDF5 schema, Arrow layout)
  - How to import to MATLAB (`readtable`, `h5read`)
  - How to import to Python (pandas, h5py)
  - How to import to Origin
  - Example analysis scripts

---

#### MEDIUM PRIORITY (3-6 months)

**7. Sequence Builder (GUI Feature)**
- **Impact**: Covers 80% of use cases without code
- **Effort**: Medium
- **Gemini Recommendation**: Better than full drag-and-drop editor
- **Deliverable**: "Sequence Builder" panel in `ui`
- **UI Design**:
  ```
  [Add Step ▼]
    ├─ Move Stage
    ├─ Wait
    ├─ Acquire Frame
    ├─ Set Parameter
    └─ Custom Rhai

  Sequence Steps:
  1. Move Stage → Position: 10.0 mm
  2. Wait → Duration: 0.5 s
  3. Acquire Frame
  4. Move Stage → Position: 20.0 mm
  [▶ Run Sequence] [💾 Save as Script]
  ```
- **Backend**: Generate Rhai script from sequence definition
- **Benefit**: No programming required for 80% of experiments

**8. Template Scripts Collection**
- **Impact**: Fill-in-the-blank reduces coding barrier
- **Effort**: Low
- **Deliverable**: `crates/examples/templates/*.rhai`
- **Templates**:
  - `linear_scan_template.rhai`
  - `time_series_template.rhai`
  - `focus_optimization_template.rhai`
  - `polarization_scan_template.rhai`
- **Structure**:
  ```rhai
  ///////////////////////////////////////
  // CONFIGURATION (Edit these values)
  ///////////////////////////////////////
  let START_POS = 0.0;      // mm
  let END_POS = 100.0;      // mm
  let STEP_SIZE = 1.0;      // mm
  let DWELL_TIME = 0.1;     // seconds

  ///////////////////////////////////////
  // MAIN SCRIPT (Don't edit below)
  ///////////////////////////////////////
  // ... implementation
  ```

**9. Glossary of Terms**
- **Impact**: Bridges knowledge gap
- **Effort**: Low
- **Deliverable**: `docs/user_guides/glossary.md`
- **Terms to Define**:
  - **Serial Port**: Physical connection for device communication
  - **Baud Rate**: Speed of serial communication (bits per second)
  - **COM Port**: Windows name for serial ports (COM1, COM2, etc.)
  - **TTY**: Unix/Linux name for serial ports (/dev/ttyUSB0, etc.)
  - **Flow Control**: Hardware handshaking for serial communication
  - **gRPC**: Remote procedure call protocol for network communication
  - **Async**: Concurrent execution without blocking
  - **Mock Device**: Software simulation of hardware for testing
  - **Capability**: What a device can do (Move, Read, Trigger, etc.)
  - **Parameter**: Configurable device setting (position, exposure, etc.)

**10. Simplified README**
- **Impact**: Better first impression
- **Effort**: Low (rewrite)
- **Changes**:
  - **Before**:
    ```markdown
    ## Prerequisites
    - Rust: Stable toolchain (1.75+)
    - System Libraries (Optional): HDF5, Arrow
    ```
  - **After**:
    ```markdown
    ## Quick Start (No Programming Required)
    1. Download `rust-daq-gui` for your OS
    2. Run the application
    3. Follow the "Mock Mode Tutorial" (5 minutes)

    **For Developers**: See [Building from Source](docs/developer/building.md)
    ```

---

#### LONG-TERM (6+ months)

**11. Installation Packages**
- Windows: `.msi` installer (WiX Toolset)
- macOS: `.dmg` with notarization
- Linux: `.deb`, `.rpm`, or AppImage
- Features:
  - Bundle daemon + GUI + default config
  - Desktop shortcuts
  - File associations (.rhai scripts)
  - Automatic PATH setup

**12. Improved Error Messages**
- Translate Rhai runtime errors to user-friendly descriptions
- "Did you mean?" suggestions
- Actionable next steps
- Example:
  ```
  BEFORE: "Runtime error: Function 'tigger' not found"
  AFTER:  "Error: Function 'tigger' not found
           Did you mean 'trigger'?
           Available camera functions: arm(), trigger(), get_resolution()"
  ```

**13. Video Tutorials**
- "Your First Scan in 5 Minutes" (mock devices)
- "Connecting Your Newport Power Meter"
- "Writing Your First Script"
- "Analyzing Your Data in MATLAB"
- Publish on YouTube, link from docs

**14. Web-Based GUI** (Optional)
- Browser interface (React/Vue)
- Connect to server via gRPC-Web
- Zero installation
- **Caveat**: May not be worth effort if desktop GUI sufficient

---

## What NOT to Do

Based on Gemini's recommendations:

❌ **DON'T build full drag-and-drop visual programming**
- Massive engineering sink
- LabVIEW-style visual programming has maintenance nightmares
- Sequence Builder (item #7) covers 80% of needs with 20% of effort

❌ **DON'T pursue hardware auto-discovery aggressively**
- Risk: Sending random bytes to expensive/sensitive equipment
- False positives: Other devices on same port
- GUI configuration editor provides same UX benefit safely

❌ **DON'T add more scripting engines (Python, Lua) yet**
- Focus on making Rhai excellent first
- Multi-engine support adds complexity
- Wait for user adoption to validate need

❌ **DON'T sacrifice backend quality for UX shortcuts**
- V5 architecture is production-ready
- Maintain type safety, concurrency model, capability abstraction
- All improvements should be additive (GUI panels, docs, binaries)

---

## Implementation Priority Matrix

| Priority | Effort | Impact | Items |
|----------|--------|--------|-------|
| **DO FIRST** | Low | High | Pre-compiled binaries, Mock guide, FAQ |
| **HIGH VALUE** | Medium | High | GUI config editor, Hardware guides, Data docs |
| **GOOD ROI** | Low-Medium | Medium | Template scripts, Glossary, README update |
| **POWER USER** | Medium | Medium | Sequence builder |
| **PROFESSIONAL** | High | Medium | Installers, Video tutorials |
| **OPTIONAL** | Very High | Low-Medium | Web GUI |

---

## Success Metrics

### 3-Month Goals
- [ ] Pre-built binaries available on GitHub Releases
- [ ] Mock mode tutorial published
- [ ] Troubleshooting FAQ published
- [ ] GUI configuration editor functional
- [ ] 3+ hardware setup guides complete

### 6-Month Goals
- [ ] Sequence builder shipped in GUI
- [ ] Template script collection (5+ templates)
- [ ] Data export fully documented
- [ ] Installation packages (Windows + macOS)

### User Adoption KPIs
- Downloads of pre-built binaries
- GitHub stars/forks
- Community contributions (scripts, drivers, documentation)
- External labs using the system (testimonials/case studies)

---

## Technical Architecture Decisions (V5 → V6)

### What to Preserve

1. **Headless-First Architecture**
   - Daemon (backend) + Multiple frontends (GUI, CLI, scripts)
   - gRPC for remote operation
   - Clean separation of concerns

2. **Capability-Based Hardware Abstraction**
   - Devices defined by what they can do (Movable, Readable, etc.)
   - No concrete type hierarchies
   - Easy mocking for testing

3. **Rhai Scripting Engine**
   - Human-readable syntax
   - No compilation cycle
   - Safety limits
   - Domain-appropriate vocabulary

4. **Feature Flags**
   - Modular compilation
   - Users only compile what they need
   - Keep binaries lean

5. **Reactive Parameters**
   - Observable state changes
   - Broadcast to GUI/network clients
   - Prevents "split brain" synchronization issues

### What to Add (Non-Breaking)

1. **Configuration GUI Panel**
   - Generates TOML, doesn't replace it
   - Power users can still hand-edit
   - Validates before saving

2. **Sequence Builder**
   - Generates Rhai scripts
   - Users can view/edit generated code
   - Advanced users bypass GUI and write directly

3. **Binary Distribution**
   - GitHub Releases
   - No change to source code
   - CI/CD automation

4. **Documentation**
   - Additive, no removals
   - Link existing technical docs from new user guides
   - Maintain developer documentation

### What to Avoid

1. **No GUI-only Features**
   - Everything must be scriptable/automatable
   - GUI is a convenience layer over gRPC API
   - CLI must remain full-featured

2. **No Breaking Changes to APIs**
   - Rhai scripting API is stable
   - Parameter interface is stable
   - Capability traits are stable

3. **No Reduced Functionality**
   - Don't sacrifice power for simplicity
   - Add abstraction layers, don't remove capabilities

---

## Comparison to V1-V4 Iterations

### V1-V4: Learning Phase
- **V1**: Monolithic, tightly coupled
- **V2**: Attempted modularization (incomplete)
- **V3**: gRPC-first (premature optimization)
- **V4**: Over-abstracted (too many layers)

**Lesson**: Slow iterations taught what NOT to do.

### V5: Production Architecture ✅
- Headless-first (daemon + clients)
- Capability-based HAL
- Feature-gated modular design
- Document-oriented data model (Bluesky-inspired)
- Rhai scripting abstraction

**Status**: Technically sound, ready for usability improvements.

### V6: Usability Focus (Proposed)
- **No architectural changes**
- Focus on:
  - Installation UX (binaries, installers)
  - Configuration UX (GUI editor)
  - Documentation (user guides, tutorials)
  - Workflow tools (sequence builder, templates)

**Philosophy**: V5 backend + V6 frontend = Complete system.

---

## Resource Requirements

### Immediate (1-2 weeks)
- **Effort**: 20-30 hours
- **Personnel**: 1 developer
- **Skills**: CI/CD (GitHub Actions), Technical writing
- **Deliverables**: Binaries, Mock tutorial, FAQ

### High Priority (1-2 months)
- **Effort**: 80-120 hours
- **Personnel**: 1 developer + 1 technical writer (or split roles)
- **Skills**: Rust GUI (egui), TOML serialization, Hardware knowledge
- **Deliverables**: Config editor, Hardware guides, Data docs

### Medium Priority (3-6 months)
- **Effort**: 160-240 hours
- **Personnel**: 2 developers (can parallelize)
- **Skills**: GUI development, Script generation, Documentation
- **Deliverables**: Sequence builder, Templates, Glossary

### Long-Term (6+ months)
- **Effort**: 240+ hours
- **Personnel**: 2-3 developers (installers require platform expertise)
- **Skills**: Windows (WiX), macOS (codesigning), Linux (packaging)
- **Deliverables**: Professional installers, Videos, Web GUI (optional)

---

## Risks and Mitigations

### Risk: Feature Creep
**Mitigation**: Strict adherence to priority matrix. Quick wins first, defer optional items.

### Risk: Breaking Power User Workflows
**Mitigation**: All improvements additive. CLI and direct TOML editing remain available.

### Risk: Documentation Becomes Outdated
**Mitigation**: Automate screenshot generation. Include version numbers in guides.

### Risk: Platform-Specific Installation Issues
**Mitigation**: Test binaries on fresh VMs. Provide fallback instructions (build from source).

### Risk: Sequence Builder Too Simplistic
**Mitigation**: Design as "training wheels." Advanced users can export Rhai and customize.

---

## Open Questions for V6

1. **Binary Distribution**: GitHub Releases sufficient, or need proper package repositories (Homebrew, Chocolatey, apt)?

2. **GUI Framework**: Continue with egui (immediate mode) or consider declarative frameworks (iced, Dioxus)?

3. **Hardware Auto-Discovery**: Worth revisiting with opt-in "safe mode" for known devices?

4. **Python Bindings**: Demand for PyO3 bindings to control rust-daq from Python scripts?

5. **Cloud Integration**: Future need for cloud storage backends (S3, Google Cloud Storage)?

6. **Multi-User**: Concurrent access from multiple GUIs to same daemon session?

---

## Appendix: Key Quotes

### Gemini's Assessment

> "The system is **architecturally excellent but operationally immature** for non-programmers. It follows a 'Headless-First' design which is powerful for automation but currently lacks the 'Batteries Included' experience of commercial tools."

> "Requiring `cargo build` essentially restricts your user base to Rust developers."

> "The complexity is **appropriate** for the backend (gRPC, capabilities, actors) as it ensures stability—a crash in a driver shouldn't crash the UI. However, the **user-facing complexity is too high** because internal details (TOML, CLI args, compilation) leak out to the user."

> "The Rhai example you provided is **sufficiently simple**. This is accessible to anyone who has used MATLAB or basic Python."

> "**Do not build a full drag-and-drop editor yet.** It is a massive engineering sink. Better Alternative: A 'Sequence Builder' in the GUI covers 80% of needs without the complexity of a full visual programming language."

---

## Conclusion

The rust-daq project has successfully built a **production-ready, high-performance backend** through the V1-V4 learning iterations. V5 represents a stable, well-architected foundation.

**The path forward (V6) is clear**: Improve user-facing layers (installation, configuration, documentation) while preserving backend excellence. The improvements are **additive and low-risk** - no architectural changes required.

**Quick wins exist**: Pre-compiled binaries and mock mode tutorial require minimal effort but remove major adoption barriers.

**High-value improvements**: GUI configuration editor and hardware setup guides provide LabVIEW-like user experience without sacrificing Rust's technical advantages.

**The vision**: A system that serves both populations:
- **Power users** (physicists/engineers): Full Rust/Rhai capabilities, scriptable, automatable
- **Casual users** (biologists/chemists): GUI-driven, template-based, minimal programming

This is achievable without compromising the V5 architecture.

---

**Document Version**: 1.0
**Last Updated**: December 2025
**Authors**: Claude Code + Gemini (collaborative assessment)
**Status**: Planning Document (not yet approved for implementation)
