# Deep Analysis: rust-daq as a Scientific Experiment Development Platform
**Date:** 2025-12-20
**Analyst:** Claude Code
**Purpose:** Comprehensive platform analysis for user-friendly, interactive, scientific experiment automation

## Executive Summary

rust-daq implements a **headless-first V5 architecture** for scientific instrument control and data acquisition. It demonstrates strong architectural foundations (capability-based HAL, Bluesky-inspired orchestration, document-oriented data model) but has significant gaps preventing it from being a **truly user-friendly platform** for laboratory researchers.

**Current State:** Excellent technical foundation (v0.5.0), limited accessibility
**Target State:** Interactive experiment development platform accessible to non-programmers

---

## 1. Architecture Analysis

### 1.1 Current Strengths

**V5 Headless-First Design:**
- Daemon (`rust-daq-daemon`) exposes hardware via gRPC
- GUI (`rust-daq-gui`) as pure gRPC client
- Clear separation: presentation layer fully decoupled from business logic
- Enables remote control, multiple clients, headless deployment

**Modular Crate Structure:**
- `common`: Foundation types, capabilities, errors
- `daq-hardware`: Capability-based HAL with real drivers (PVCAM, Thorlabs ELL14, Newport ESP300, MaiTai laser)
- `daq-experiment`: RunEngine with Bluesky-inspired Plan system
- `daq-scripting`: Rhai embedded scripting (Python planned)
- `daq-storage`: Multi-backend persistence (CSV, HDF5, Arrow)
- `daq-server`: gRPC server with streaming support
- `daq-egui`: Native GUI with real-time visualization
- `rust-daq`: Integration layer with organized prelude

**Data Pipeline ("Mullet Strategy"):**
- Reliable path: Backpressure + Ring buffer for guaranteed storage
- Lossy path: Broadcast channels for low-latency visualization
- Smart tradeoff between data integrity and responsiveness

**Capability-Based Hardware Abstraction:**
```rust
trait Movable { async fn move_abs(&self, position: f64) -> Result<()>; }
trait Readable { async fn read(&self) -> Result<Measurement>; }
trait FrameProducer { async fn arm(&self) -> Result<()>; }
trait Triggerable { async fn trigger(&self) -> Result<()>; }
```
- Flexible composition, easy mocking, extensible
- Contrast with inheritance hierarchies: more adaptable

**Reactive Parameter System:**
- `Parameter<T>` with hardware callbacks and validation
- Observable pattern for GUI/client synchronization
- Prevents "split brain" where hardware state diverges from application state

### 1.2 Architectural Gaps (User-Friendly Platform Perspective)

**1. Experiment Definition is Programmer-Centric**
- **Current:** Plans written in Rust or low-level Rhai scripts
- **Gap:** No visual experiment designer, no drag-and-drop workflow
- **Impact:** Requires programming knowledge to design experiments

**2. RunEngine/Plans Underutilized**
- **Current:** RunEngine exists but minimal Plan implementations
- **Gap:** No library of common scientific workflows (grid scan, time series, parameter sweep, optimization loops)
- **Impact:** Users reinvent the wheel for standard experiments

**3. Hardware Configuration is File-Based**
- **Current:** TOML config files (e.g., `config/demo.toml`)
- **Gap:** No GUI-based hardware setup wizard, no auto-discovery, no validation feedback
- **Impact:** Manual editing error-prone, intimidating for non-programmers

**4. Scripting is Low-Level**
- **Current:** Rhai scripts call device methods directly (`camera.arm()`, `stage.move_abs()`)
- **Gap:** No high-level "recipes" or templates, no block-based scripting
- **Impact:** Requires understanding hardware protocols

**5. Data Analysis is External**
- **Current:** Data saved to HDF5/CSV, analyzed separately (Python/MATLAB)
- **Gap:** No built-in analysis pipeline, no live computation
- **Impact:** Disconnected workflow, slow feedback loops

**6. GUI is Device-Centric, Not Experiment-Centric**
- **Current:** GUI organized by devices (Devices panel, Scripts panel)
- **Gap:** No "Experiment" panel, no experimental design workflow, no run history browser
- **Impact:** Users think in terms of devices, not experiments

---

## 2. User Experience Analysis

### 2.1 User Personas

**Persona 1: Laboratory Researcher (Non-Programmer)**
- **Goal:** Automate repeated measurements (e.g., daily calibration, overnight data collection)
- **Needs:** Point-and-click configuration, simple scripting (if-then logic), email alerts
- **Current Pain:** Must learn Rust or Rhai, edit TOML files, debug gRPC errors

**Persona 2: Graduate Student (Basic Programming)**
- **Goal:** Design custom experiments (parameter sweeps, optimization, multi-device synchronization)
- **Needs:** Script templates, debugging tools, data preview, undo/redo
- **Current Pain:** No experiment templates, limited debugging (print statements), no in-app data viewer

**Persona 3: Instrumentation Developer (Expert)**
- **Goal:** Integrate new hardware, optimize performance, build reusable modules
- **Needs:** Driver SDK, performance profiling, testing framework
- **Current Strengths:** Good HAL abstraction, async runtime, capability traits
- **Current Pain:** Feature flag duplication, specialized binaries missing

### 2.2 Critical User Journey: "Run My First Automated Scan"

**Current Experience (v0.5.0):**
1. Install Rust toolchain (30 min)
2. Clone repo, run `cargo build` (10 min)
3. Read Getting Started panel (5 min)
4. Start daemon: `cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml`
5. Connect GUI to daemon
6. Write Rhai script from scratch or modify examples
7. Upload script via GUI (v0.6.0 feature, not yet implemented)
8. Manually trigger execution
9. Export data, analyze externally

**Ideal Experience:**
1. Download binary (2 min)
2. Launch GUI, click "New Experiment Wizard"
3. Select devices from auto-discovered list
4. Choose experiment type (Grid Scan template)
5. Fill in parameters visually (start/stop/steps with unit validation)
6. Preview execution graph
7. Click "Run", see live plots
8. Export with one click, or run built-in analysis

**Gap:** 8 steps → 7 steps, but qualitative difference in accessibility

---

## 3. Automation & Scripting Capabilities

### 3.1 Current State

**Rhai Scripting Engine:**
- Embedded language, Rust-like syntax
- Type-safe bindings to hardware APIs
- Examples: `demo_camera.rhai` (working), `demo_scan.rhai` (v0.6.0 placeholder)

**Script Execution Modes:**
1. **One-shot:** Direct execution with hardcoded mock devices (v0.5.0)
2. **Daemon:** gRPC-based execution with config-file devices (v0.6.0)

**Current Bindings (daq-scripting/src/bindings.rs):**
```rhai
camera.arm()
camera.trigger()
stage.move_abs(position)
sleep(duration_seconds)
print(message)
```

### 3.2 Automation Gaps

**1. No Conditional Logic Examples**
- **Missing:** If-then-else templates (e.g., "if power > threshold, stop scan")
- **Missing:** Error handling patterns (retry logic, fallback strategies)

**2. No Loop Abstractions**
- **Current:** Users write `for i in 0..10 { ... }` manually
- **Missing:** High-level "scan" command (e.g., `scan(stage, 0, 10, 11)`)

**3. No Event-Driven Automation**
- **Missing:** Triggers (e.g., "start acquisition when stage reaches position")
- **Missing:** Watchdogs (e.g., "alert if temperature > 50°C")

**4. No Experiment Scheduling**
- **Missing:** Cron-like scheduling, queue management
- **Missing:** Dependencies between experiments (run B after A succeeds)

**5. No Data-Driven Workflows**
- **Missing:** Real-time decision making (adaptive sampling based on live data)
- **Missing:** Feedback loops (optimize parameter based on measurement)

### 3.3 Recommendations

**Short-Term (v0.6.0):**
- Implement daemon device registration (complete `demo_scan.rhai`)
- Add high-level Rhai functions: `scan_1d()`, `scan_grid()`, `wait_for()`
- Create script template library in GUI

**Medium-Term (v0.7.0):**
- Block-based scripting UI (Blockly/Scratch-like)
- Experiment scheduler with queue management
- Real-time conditionals (if-then-else evaluated during execution)

**Long-Term (v0.8.0):**
- Visual experiment designer (drag nodes for devices, connect edges for data flow)
- Adaptive experiment engine (Bayesian optimization, active learning)
- Python scripting with Jupyter integration

---

## 4. Hardware Integration Analysis

### 4.1 Current Driver Support

**Real Hardware Drivers:**
- **PVCAM (Photometrics cameras):** Prime BSI, GS2020 sensor, 2048x2048
- **Thorlabs ELL14:** Motorized rotator, RS-232, device-specific calibration
- **Newport ESP300:** Motion controller, TCP/IP
- **MaiTai Laser:** Tunable Ti:Sapphire, serial control
- **Newport 1830-C:** Power meter, serial

**Mock Drivers:**
- MockCamera, MockStage, MockPowerMeter
- Used for demos and testing

### 4.2 Hardware Integration Gaps

**1. No Hardware Auto-Discovery**
- **Current:** Manual TOML configuration with serial port paths
- **Missing:** USB/serial device enumeration, driver auto-selection
- **Impact:** User must know device serial ports, driver names

**2. No Hardware Wizard**
- **Missing:** GUI workflow for "Add New Device" with step-by-step instructions
- **Missing:** Connection testing ("Test Connection" button)
- **Missing:** Firmware version checking, compatibility warnings

**3. Limited Driver Ecosystem**
- **Current:** 5 real drivers + mocks
- **Missing:** Common lab equipment (oscilloscopes, spectrometers, function generators, temperature controllers)
- **Missing:** Vendor SDK integration guides

**4. No Driver Marketplace**
- **Missing:** Community-contributed drivers
- **Missing:** Driver search, versioning, dependency management

**5. Parameter Discovery is Code-Driven**
- **Current:** Parameters defined in driver code
- **Missing:** Device description files (like QCoDeS station YAML)
- **Impact:** Changing parameter metadata requires code changes

### 4.3 Recommendations

**Short-Term (v0.6.0):**
- Implement device registry UI (list, add, remove, test connection)
- Add "Test Connection" button in GUI
- Document driver development process

**Medium-Term (v0.7.0):**
- USB/serial auto-discovery with driver suggestions
- Hardware setup wizard with guided configuration
- External driver plugin system (dynamic loading)

**Long-Term (v0.8.0):**
- Driver marketplace with community contributions
- Device description format (JSON schema for parameters, capabilities)
- Hardware simulator for offline development

---

## 5. Experiment Design & Execution

### 5.1 Current State (RunEngine + Plans)

**Architecture:**
- **Plans:** Declarative experiment definitions (generators yielding commands)
- **RunEngine:** State machine executing plans with pause/resume
- **Documents:** Structured data stream (Start → Descriptor → Events → Stop)

**Example Plan:**
```rust
let plan = GridScan::new("stage_x", 0.0, 10.0, 11)
    .with_detector("power_meter")
    .build();
engine.queue(plan).await?;
```

**Current Plan Implementations:**
- (Minimal) GridScan exists but not fully integrated
- Most examples use imperative Rhai scripts, not declarative Plans

### 5.2 Experiment Design Gaps

**1. No Plan Library**
- **Missing:** Standard scientific workflows as ready-to-use Plans
- **Examples:** Time series, 2D/3D scans, parameter sweeps, optimization (Nelder-Mead, gradient descent), calibration routines, alignment procedures

**2. No Visual Experiment Builder**
- **Missing:** Drag-and-drop experiment graph editor
- **Missing:** Visual preview of execution flow (timeline, dependencies)

**3. No Experiment Templates**
- **Missing:** Domain-specific templates (spectroscopy, microscopy, laser characterization)
- **Missing:** User-defined template system

**4. No Metadata Management**
- **Current:** StartDoc has basic metadata (operator, sample ID)
- **Missing:** Enforced metadata schemas (e.g., require sample temperature, batch number)
- **Missing:** Metadata validation before experiment runs

**5. No Execution Control UI**
- **Current:** Start/pause via code
- **Missing:** GUI controls (Start, Pause, Resume, Stop, Skip to next point)
- **Missing:** Progress bars, time estimates, checkpoint indicators

**6. No Experiment History**
- **Missing:** Run history browser (list past experiments with metadata search)
- **Missing:** Re-run from history
- **Missing:** Compare results across runs

### 5.3 Recommendations

**Short-Term (v0.6.0):**
- Complete GridScan Plan integration
- Add TimeSeries Plan (sample at fixed intervals)
- Add Experiment History panel in GUI (list StartDocs from storage)

**Medium-Term (v0.7.0):**
- Plan library: 2D/3D scans, parameter sweeps, alignment, calibration
- Execution control panel in GUI (start/pause/resume/stop)
- Metadata editor with validation

**Long-Term (v0.8.0):**
- Visual experiment graph editor
- Template system with domain-specific libraries
- Experiment comparison tool (overlay plots, diff metadata)

---

## 6. Data Management & Visualization

### 6.1 Current State

**Storage Backends:**
- **CSV:** Simple, portable, limited performance
- **HDF5:** High-performance binary, requires native libraries
- **Arrow:** Columnar in-memory format, fast queries

**Data Pipeline:**
- RingBuffer for reliable in-memory buffering
- Background writers (HDF5Writer, CSVWriter)
- Lossy broadcast for live visualization

**GUI Visualization:**
- Real-time device state updates via gRPC streaming
- egui_plot for live plots
- Rerun integration for camera frames

### 6.2 Data Management Gaps

**1. No Built-In Data Browser**
- **Current:** Data saved to files, opened externally
- **Missing:** In-app data browser (list experiments, view metadata)
- **Missing:** Quick preview (plot first 100 points, show thumbnail)

**2. No Live Analysis**
- **Missing:** Real-time computations (mean, std dev, peak finding)
- **Missing:** Live fitting (update curve fit as data arrives)
- **Missing:** Derived quantities (e.g., reflectance = I_reflected / I_incident)

**3. No Data Export Wizard**
- **Current:** Files saved automatically during acquisition
- **Missing:** Export subset of data, format conversion, axis selection

**4. No Annotation System**
- **Missing:** Mark interesting regions during acquisition
- **Missing:** Add notes to runs ("laser unstable during this period")

**5. No Long-Term Data Management**
- **Missing:** Database backend (search by metadata, date range)
- **Missing:** Data lifecycle policies (archive old runs, compress)

**6. Limited Visualization Options**
- **Current:** Line plots, camera frames
- **Missing:** Heatmaps, waterfall plots, 3D surface plots
- **Missing:** Multi-panel layouts (compare multiple detectors)

### 6.3 Recommendations

**Short-Term (v0.6.0):**
- Add Data Browser panel in GUI (list HDF5/CSV files in directory)
- Implement quick preview (load StartDoc + first EventDoc)
- Add basic statistics display (mean, min, max, count)

**Medium-Term (v0.7.0):**
- Live analysis framework (compute derived quantities during acquisition)
- Annotation system (markers on plots, text notes)
- Heatmap and waterfall plot widgets

**Long-Term (v0.8.0):**
- Database backend (SQLite or MongoDB for metadata)
- Advanced visualizations (3D plots, multi-panel layouts)
- Export wizard with format conversion

---

## 7. Usability & Onboarding

### 7.1 Current State (v0.5.0)

**Getting Started Panel:**
- Default panel on GUI launch
- Step-by-step demo mode instructions
- Cross-platform command examples (Unix + Windows)
- Honest v0.5.0 vs v0.6.0 feature separation

**Demo Infrastructure:**
- `config/demo.toml`: Pre-configured mock devices
- `examples/demo_camera.rhai`: Working one-shot camera demo
- `examples/demo_scan.rhai`: Placeholder for v0.6.0

**Documentation:**
- Architecture docs (ARCHITECTURE.md, GUI guide)
- Instrument-specific guides (PVCAM, ELL14, MaiTai)
- Project management docs (bd issue tracking)

### 7.2 Usability Gaps

**1. Installation Friction**
- **Current:** Requires Rust toolchain (rustup, cargo)
- **Missing:** Pre-built binaries (GitHub Actions exists but not yet triggered)
- **Missing:** Installer (one-click setup for Windows/macOS)

**2. Configuration Complexity**
- **Current:** Manual TOML editing
- **Missing:** Configuration wizard in GUI
- **Missing:** Validation with helpful error messages

**3. Error Messages are Developer-Focused**
- **Current:** Rust error types (DaqError::Hardware, DaqError::Io)
- **Missing:** User-friendly error explanations ("Camera not found" vs "Error opening /dev/video0: ENOENT")

**4. No Interactive Tutorials**
- **Current:** Static documentation, example scripts
- **Missing:** In-app guided tutorials (tooltips, highlighted regions)
- **Missing:** Video walkthroughs

**5. No Community Platform**
- **Missing:** Forum for questions, example sharing
- **Missing:** User-contributed experiment templates
- **Missing:** FAQ, troubleshooting guides

### 7.3 Recommendations

**Short-Term (v0.6.0):**
- Trigger first GitHub Release (tag v0.5.0) to build binaries
- Add configuration validator (check serial ports exist, drivers available)
- Improve error messages (add user-friendly context)

**Medium-Term (v0.7.0):**
- Interactive tutorial mode (step-through first experiment)
- Configuration wizard (detect devices, test connections)
- FAQ panel in GUI

**Long-Term (v0.8.0):**
- Installer packages (`.msi` for Windows, `.dmg` for macOS, `.deb` for Linux)
- Community forum integration (share experiments, ask questions)
- In-app video tutorials

---

## 8. Platform Comparison

### 8.1 Similar Platforms

**Bluesky (Python):**
- Inspiration for RunEngine/Plans
- **Strengths:** Mature ecosystem, extensive Plan library, IPython integration
- **Gaps vs rust-daq:** No type safety, slower (GIL), harder to deploy

**QCoDeS (Python):**
- Parameter-based measurement framework
- **Strengths:** Large driver library, Jupyter integration, dataset management
- **Gaps vs rust-daq:** No GUI, no real-time visualization, slower

**ScopeFoundry (Python):**
- Qt-based GUI framework for microscopy
- **Strengths:** Modular architecture, hardware plugins, 2D/3D visualization
- **Gaps vs rust-daq:** Single-threaded, no async, limited remote control

**ARTIQ (Rust/Python):**
- Control system for quantum physics experiments
- **Strengths:** Deterministic timing (FPGA), remote control, dataset browser
- **Gaps vs rust-daq:** Specialized for quantum, steeper learning curve

**LabView (Commercial):**
- Visual programming for instrumentation
- **Strengths:** Huge driver library, visual design, industry standard
- **Gaps:** Expensive, proprietary, Windows-centric, not modern (no async, no type safety)

### 8.2 rust-daq Competitive Position

**Current Advantages:**
- Modern architecture (async/await, type safety, zero-cost abstractions)
- Headless-first (remote control, scriptable, embeddable)
- High performance (Rust speed, efficient data pipeline)
- Strong foundations (capability-based HAL, reactive parameters)

**Current Disadvantages:**
- Limited driver library (5 real drivers vs hundreds in LabView/QCoDeS)
- No visual experiment designer (vs LabView)
- Limited Plan library (vs Bluesky's 50+ Plans)
- No established community (vs mature Python ecosystems)
- Installation friction (Rust toolchain vs Python/LabView installers)

**Opportunity Space:**
- **"LabView for the 21st Century":** Visual design + modern architecture
- **"Type-Safe Bluesky":** RunEngine/Plans with compile-time guarantees
- **"Embedded-Ready ScopeFoundry":** Deploy to Raspberry Pi, real-time OS
- **"Open-Source ARTIQ for General Labs":** Deterministic control without FPGA lock-in

---

## 9. Identified Gaps (Prioritized)

### 9.1 Critical Gaps (Blocking User Adoption)

**C1. No Pre-Built Binaries**
- Users cannot try without installing Rust toolchain
- **Fix:** Trigger GitHub Release workflow

**C2. Daemon Device Registration Not Implemented (v0.6.0)**
- Multi-device workflows (demo_scan.rhai) don't work
- **Fix:** Implement device globals in ScriptEngine from config

**C3. No Experiment Execution UI**
- Users cannot run experiments from GUI (must use CLI scripts)
- **Fix:** Add "Run Script" button, execution status panel

**C4. No Data Browser**
- Users cannot see results without leaving application
- **Fix:** Add Data panel listing runs with quick preview

### 9.2 High-Priority Gaps (Limiting Usability)

**H1. No Hardware Setup Wizard**
- Manual TOML editing error-prone
- **Fix:** GUI-based device configuration with validation

**H2. No Plan Library**
- Users reinvent common experiments
- **Fix:** Implement GridScan, TimeSeries, ParameterSweep Plans

**H3. No Script Templates**
- Users start from scratch every time
- **Fix:** Add template dropdown in Scripts panel

**H4. No Live Analysis**
- Users must export, analyze separately (slow feedback)
- **Fix:** Add real-time statistics, basic fitting

**H5. Limited Visualization**
- Only line plots, no heatmaps/3D
- **Fix:** Add heatmap widget for 2D scans

### 9.3 Medium-Priority Gaps (Improving Experience)

**M1. No Interactive Tutorials**
- Learning curve steep for non-programmers
- **Fix:** Add guided tutorial mode

**M2. No Experiment History**
- Cannot compare runs, re-run experiments
- **Fix:** History panel with metadata search

**M3. No Error Recovery**
- Experiments fail completely on any error
- **Fix:** Add retry logic, partial results saving

**M4. No Block-Based Scripting**
- Rhai scripts intimidating for beginners
- **Fix:** Blockly-style visual scripting

**M5. No Community Platform**
- Users isolated, cannot share knowledge
- **Fix:** Set up Discourse forum, example repository

### 9.4 Low-Priority Gaps (Nice-to-Have)

**L1. No Python Scripting**
- Existing Python users cannot leverage skills
- **Fix:** Add PyO3-based Python bindings

**L2. No Jupyter Integration**
- Cannot use notebooks for experiments
- **Fix:** Add Jupyter kernel

**L3. No Advanced Visualizations**
- Limited to basic plots
- **Fix:** 3D plots, waterfall, animations

**L4. No Database Backend**
- Metadata search limited
- **Fix:** SQLite for experiment metadata

**L5. No Mobile Monitoring**
- Cannot check experiments remotely
- **Fix:** Web UI or mobile app

---

## 10. Recommendations Summary

### 10.1 Short-Term Roadmap (v0.6.0 - Q1 2025)

**Theme:** Complete Foundational Features
1. ✅ Pre-built binaries (trigger GitHub Release)
2. ✅ Daemon device registration (complete demo_scan.rhai)
3. ✅ Script execution UI (Run button, status panel)
4. ✅ Data Browser panel (list runs, quick preview)
5. ✅ Hardware configuration UI (add/remove devices, test connection)
6. ✅ Basic Plan library (GridScan, TimeSeries)

**Impact:** Usable by graduate students with basic programming

### 10.2 Medium-Term Roadmap (v0.7.0 - Q2 2025)

**Theme:** Accessibility for Non-Programmers
1. ✅ Script templates library (scan types, calibration, alignment)
2. ✅ Block-based scripting UI (Blockly-like)
3. ✅ Live analysis framework (real-time statistics, fitting)
4. ✅ Heatmap and waterfall visualizations
5. ✅ Experiment history and comparison
6. ✅ Interactive tutorial system

**Impact:** Usable by laboratory researchers without programming background

### 10.3 Long-Term Roadmap (v0.8.0 - Q3-Q4 2025)

**Theme:** Advanced Platform Features
1. ✅ Visual experiment designer (node-graph editor)
2. ✅ Python scripting support
3. ✅ Jupyter integration
4. ✅ Database backend for metadata
5. ✅ Driver marketplace
6. ✅ Adaptive experiment engine (Bayesian optimization)

**Impact:** Competitive with commercial platforms (LabView), surpassing in architecture

---

## 11. Conclusion

rust-daq has **world-class technical foundations** but remains **programmer-centric**. Transforming it into a **user-friendly scientific platform** requires:

1. **Closing the Execution Gap:** Finish v0.6.0 features (daemon device registration, script UI)
2. **Democratizing Experiment Design:** Plan library, templates, visual tools
3. **Integrating Analysis:** Live computation, data browser, visualization
4. **Lowering Barriers:** Pre-built binaries, wizards, tutorials
5. **Building Community:** Forums, examples, documentation

**Strategic Priority:** Focus v0.6.0 on **completion** (make existing features work end-to-end) rather than **expansion** (add new capabilities). This builds credibility and usability momentum.

**Target Outcome:** By v0.7.0, a laboratory researcher should be able to:
- Download binary → configure devices via GUI → design experiment from template → run and visualize → export results **without writing code or editing files**.

This positions rust-daq as the **modern open-source alternative to LabView** for scientific automation.
