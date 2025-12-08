# Changelog

All notable changes to Rust DAQ will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.5.0] - 2025-12-06

### V5 Transition Complete

The V5 architectural transition is now complete. This release finalizes the cleanup of legacy code and stabilizes the new architecture.

### Changed
- **Removed**: `src/hardware/adapter.rs` (legacy V1/V2 adapter trait)
- **Deprecated**: `DataPoint` struct in `src/core.rs` (use `Measurement` enum instead)
- **Deprecated**: `ScriptHost` in `src/scripting/engine.rs` (use `RhaiEngine` instead)
- **Refactored**: Examples and tests to use `RhaiEngine`
- **Documentation**: Comprehensive updates to reflect V5 completion

### Fixed
- Resolved split-brain architecture issues in parameter system
- Unified hardware driver state management
- Fixed module compilation issues in gRPC services

## [Unreleased] - V5 Architectural Transition

### Changed - V5 Architectural Transition (2025-11-20)

#### 🚨 BREAKING CHANGES - Complete V5 Migration

**All V1-V4 legacy code removed** (~295KB deleted). The codebase is now exclusively V5 headless-first architecture.

**Removed Architectures**:
- **V1 Monolithic Instruments**: `src/instrument/` (~120KB)
- **V2 Module System**: `src/modules/` (~30KB)
- **V2 Actor Messages**: `src/messages.rs` (~23KB)
- **V3 Instrument Manager**: `src/instrument_manager_v3.rs` (~29KB)
- **V4 Kameo Actors**: `src/actors/` (~73KB)
- **V4 Traits**: `src/traits/` (~3KB)
- **V1 Experiment Orchestration**: `src/experiment/` (~49KB)
- **GUI Components**: Removed for headless-first design

**Migration Required**:
- Replace `crate::instrument::*` with `crate::hardware::*`
- Replace V2 modules with Rhai scripts
- Replace `RunEngine` with script_runner CLI
- Replace actor messages with gRPC proto (Phase 3)

See [docs/architecture/V5_TRANSITION_COMPLETE.md](docs/architecture/V5_TRANSITION_COMPLETE.md) for complete migration guide.

### Added

- ✅ **V5 Capability-Based Hardware** (`src/hardware/capabilities.rs`)
  - Atomic traits: `Readable`, `Writable`, `Triggerable`, `Movable`, `ImageCapture`
  - 13 V5 drivers in `src/hardware/`
  - Composable hardware abstraction

- ✅ **Zero-Warning Builds** (commit 0429d0f1)
  - All compiler warnings resolved
  - Clean CI builds (commit de1d4a4f)

- ✅ **Feature Flag Normalization** (PR #107)
  - Consistent feature scoping across all V5 components
  - Removed legacy feature dependencies

- ✅ **Serial2-tokio Migration**
  - MaiTai driver migrated (commit ab38473f, bd-qiwv)
  - Modern async serial communication

### Fixed

- CI build failures (commit de1d4a4f)
- Compiler warnings cleanup (commit 0429d0f1)
- Feature flag inconsistencies (PR #107)
- V1 legacy data module compilation errors (commit 5ba543b9)

### Removed

- **BREAKING**: All V1-V4 architectures (~295KB)
- **BREAKING**: GUI components (headless-first)
- **BREAKING**: V2 actor pattern
- **BREAKING**: V4 kameo actors
- **BREAKING**: V1 experiment orchestration
- Commented-out module declarations in `src/lib.rs`
- Legacy dependency: `kameo` (optional, now removed)

### Architecture

**New V5 Design**:
1. 🎯 **Headless-First**: No GUI dependency, crash resilience
2. 📝 **Script-Driven**: Rhai + Python engines for experiment logic
3. 🌐 **Remote-First**: gRPC API for network control (Phase 3)
4. 📊 **High-Throughput**: Arrow batching + HDF5 storage
5. 🔒 **Type-Safe**: Pure Rust with async throughout

**Related Issues**:
- Closes: bd-9si6 (V4 cleanup)
- Closes: bd-qiwv (MaiTai migration)
- Closes: bd-kal8 (architectural reset)
- Related: bd-oq51 (V5 headless-first architecture)

## [0.2.0] - 2025-10-15

### Added - Major Architecture Enhancement

#### 🔄 Measurement Enum System
- **BREAKING CHANGE**: Introduced `Measurement` enum supporting structured data types
- Added `Measurement::Scalar(DataPoint)` - traditional scalar measurements
- Added `Measurement::Spectrum(SpectrumData)` - frequency domain data with typed bins  
- Added `Measurement::Image(ImageData)` - 2D image data for cameras
- Added `MeasurementProcessor` trait for type-safe data processing
- Added `SpectrumData`, `ImageData`, `FrequencyBin` structured types
- Added comprehensive helper methods: `timestamp()`, `channel()`, `unit()`, `metadata()`

#### 🚀 Performance Improvements  
- **96x reduction** in data objects for FFT processor (96 DataPoints → 1 Spectrum)
- **10x faster** data access (direct fields vs JSON parsing)
- **5x memory reduction** for spectral data
- Eliminated JSON serialization/deserialization overhead

#### 🛡️ Type Safety Enhancements
- **Compile-time guarantees** for frequency bin access
- **Pattern matching** for different measurement types  
- **Direct field access** instead of string-based JSON parsing
- **IDE support** with full autocomplete for structured data

### Changed

#### FFT Processor Transformation
- **Before**: Emits 96+ DataPoints with JSON metadata workarounds
- **After**: Emits single SpectrumData with typed Vec<FrequencyBin>
- Added dual implementation: maintains `DataProcessor` compatibility + new `MeasurementProcessor`
- Moved `FrequencyBin` from `fft.rs` to `core.rs` for reusability

#### API Extensions (Backward Compatible)
- `DataProcessor` trait unchanged - existing code works
- Added `MeasurementProcessor` trait alongside existing interfaces
- All core types implement `PartialEq` for testing and comparisons

### Fixed

#### Storage Writer Shutdown (bd-28)
- Implemented graceful shutdown with oneshot channel signaling
- Prevents data loss during storage writer termination
- Added 5-second timeout with abort fallback
- Storage writers now properly call `shutdown()` to flush buffers

#### GUI Hardware Status Display (bd-17)  
- Added color-coded instrument connection status indicators
- **Yellow "● Simulated"** - mock instruments in simulation mode
- **Green "● Hardware"** - connected to physical hardware
- **Red "● Failed"** - connection failed with error tooltip
- **Gray "● Stopped"** - instrument not running
- Replaced TODO comments with status-aware information display

### Documentation

#### Comprehensive Migration Guides
- [MEASUREMENT_ENUM_MIGRATION.md](MEASUREMENT_ENUM_MIGRATION.md) - Complete architectural overview
- [docs/measurement-processor-guide.md](docs/measurement-processor-guide.md) - Developer implementation guide  
- [docs/adr/001-measurement-enum-architecture.md](docs/adr/001-measurement-enum-architecture.md) - Architecture decision record
- Updated all code examples and documentation

#### Integration Tests
- Added `tests/measurement_enum_test.rs` demonstrating old vs new approaches
- FFT processor comparison showing quantified benefits
- Type-safety examples and migration patterns

### Technical Debt Reduction
- ✅ Eliminated JSON metadata workarounds in FFT processor
- ✅ Removed frequency data encoding in timestamp fields  
- ✅ Cleaned up data duplication (magnitude in both `value` and metadata)
- ✅ Established foundation for future complex measurement types

### Migration Path
- **Phase 1** (Current): Both `DataProcessor` and `MeasurementProcessor` coexist
- **Phase 2** (Future): Broadcast channels transition to `Measurement`
- **Phase 3** (Future): Storage backends support structured data types
- Helper utilities provided for smooth migration

### Future Enablement
This release creates the foundation for:
- PVCAM camera image processing (`Measurement::Image`)  
- Advanced spectral analysis with type-safe frequency access
- Multi-dimensional measurement types (waveforms, histograms, matrices)
- Zero-copy processing pipelines
- Columnar storage formats for efficient spectral data

---

## [0.1.0] - 2025-10-01

### Added
- Initial Rust DAQ implementation
- Core `DataPoint` structure for scalar measurements
- `DataProcessor` trait for data processing pipelines
- FFT processor with JSON metadata workarounds
- Mock, VISA, SCPI, PVCAM instrument implementations  
- eGUI-based graphical user interface
- CSV, HDF5, Arrow storage backends
- TOML-based configuration system
- Session management and metadata capture

### Features
- Real-time data acquisition and visualization
- Configurable processing pipelines
- Dockable GUI with multiple plot tabs
- Instrument control panels
- Data export in multiple formats

[0.2.0]: https://github.com/rust-daq/rust-daq/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rust-daq/rust-daq/releases/tag/v0.1.0
