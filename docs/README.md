# rust-daq Documentation

Organized using the [Diataxis](https://diataxis.fr/) framework.

## Tutorials — Learn by doing

| Tutorial | Description |
|----------|-------------|
| [Demo Mode](tutorials/demo-mode.md) | Try rust-daq without hardware |

## How-To Guides — Solve a specific problem

### Build & Deploy

| Guide | Description |
|-------|-------------|
| [Build and Run](how-to/build-and-run.md) | Build verification and setup |
| [Windows Build](how-to/build-and-run-windows.md) | Windows-specific build instructions |
| [Hardware Setup (Maitai)](how-to/hardware-setup.md) | Real hardware configuration |
| [Operations](how-to/operations.md) | Daemon startup, deployment, monitoring, alerting |
| [Maitai Universal+DB Signoff](how-to/maitai-universal-db-signoff.md) | Hardware validation runbook for SQLite DB mode |
| [Legacy SCPI Deprecation](how-to/legacy-scpi-deprecation.md) | Migration and rollback policy for legacy native SCPI/TCP paths |
| [Platform Notes](how-to/platform-notes.md) | OS-specific considerations |

### Hardware Drivers

| Guide | Description |
|-------|-------------|
| [Hardware Drivers](how-to/hardware-drivers.md) | Driver implementation patterns |
| [Device Config (Schema v3)](how-to/device-config.md) | TOML-based declarative drivers |
| [Andor SDK3](how-to/driver-andor-sdk3.md) | Andor iStar / Shamrock setup |
| [Dover Motion](how-to/driver-dover-motion.md) | Dover SmartStage driver |
| [iStar sCMOS](how-to/driver-istar-scmos.md) | iStar sCMOS camera |
| [Kymera 328i](how-to/driver-kymera-328i.md) | Kymera spectrograph |
| [Spirit Laser](how-to/driver-spirit-laser.md) | Spectra Physics Spirit laser |
| [PVCAM Setup](how-to/pvcam-setup.md) | Photometrics camera configuration |
| [Comedi Setup](how-to/comedi-setup.md) | Linux Comedi DAQ board |

### Data & Scripting

| Guide | Description |
|-------|-------------|
| [Scripting](how-to/scripting.md) | Rhai scripts for experiment automation |
| [Storage Formats](how-to/storage-formats.md) | HDF5, Arrow, Parquet, TIFF, Zarr |
| [Zarr Acquisition](how-to/zarr-acquisition.md) | Zarr V3 storage |
| [EOM Power Sweep](how-to/eom-power-sweep.md) | EOM power sweep workflow |

### Echelle Spectroscopy

| Guide | Description |
|-------|-------------|
| [Echelle Spectrum Preview (MVP)](how-to/echelle-spectrum-preview.md) | Load a calibration profile and view local echelle extraction previews |
| [Live Echelle Calibration Session](how-to/echelle-live-calibration-session-leabs-dev.md) | Bench runbook for Mechelle+iSTAR+HG-2 live calibration and GUI workflow |
| [Echelle Calibration Development](how-to/echelle-calibration-development.md) | Developer workflow for creating profiles and maintaining golden datasets |
| [Echelle Rollout & Troubleshooting](how-to/echelle-rollout-and-troubleshooting.md) | MVP rollout stages, runtime knobs, and failure troubleshooting |
| [Echelle Validation Plan & HIL Checklist](how-to/echelle-validation-plan-and-hil-checklist.md) | Dataset matrix definition and lab hardware-in-loop validation checklist |

### Infrastructure

| Guide | Description |
|-------|-------------|
| [Testing](how-to/testing.md) | Test runner, profiles, hardware tests, coverage |
| [Plugins](how-to/plugins.md) | Config-only, native Rust, and Rhai plugins |
| [SQLite Control Plane](how-to/surrealdb-integration.md) | Current DB backend note and historical SurrealDB migration context |
| [Web GUI](how-to/web-gui.md) | WASM build, deployment, and architecture |
| [Migration/Rollback Toolkit](how-to/migration-rollback-toolkit.md) | Backup, restore, and incident rollback procedures |
| [LEABS Universal+DB Signoff](how-to/leabs-universal-db-signoff.md) | LEABS hardware validation runbook for SQLite DB mode |
| [LIBS Scripting](how-to/libs-scripting.md) | Rhai API for LIBS experiments |
| [Fast Inner Loop](how-to/fast-inner-loop.md) | Development iteration tips |
| [Andor iSTAR Crash Capture](how-to/andor-istar-crash-capture-and-repro.md) | Crash capture and streaming repro harness |

## Reference — Look up details

| Reference | Description |
|-----------|-------------|
| [gRPC API](reference/grpc-api.md) | Protocol and service definitions |
| [Device Metadata Contract](reference/device-metadata-contract.md) | Capability/command/UI metadata contract for advanced panels |
| [PVCAM SDK](reference/pvcam-sdk.md) | PVCAM API reference and error codes |
| [Dover Motion API](reference/dover-motion-api.md) | MotionSynergyAPI reference |
| [Feature Matrix](reference/feature-matrix.md) | Implementation status for all features |
| [Driver Capability Matrix](reference/driver-capability-matrix.md) | Per-driver capability support |
| [Streaming Policy](reference/streaming-policy.md) | Frame streaming backpressure and rate policy |
| [Hardware Inventory](reference/inventory.md) | Physical hardware inventory across lab machines |
| [Hardware Qualification Runner](reference/hardware-qualification-runner-plan.md) | Self-hosted CI runners for hardware tests |
| [Echelle Calibration Profile Schema](reference/echelle-calibration-profile-schema.md) | Versioned profile schema for Mechelle/echelle extraction |
| [Echelle Spectrum Streaming Protocol](reference/echelle-spectrum-streaming-protocol-design.md) | gRPC vector-spectrum payload streaming design |
| [UI Workflow Costs](reference/ui-workflow-costs.md) | egui/Slint/Rerun maintenance cost analysis |

## Explanation — Understand the system

| Document | Description |
|----------|-------------|
| [Architecture](explanation/architecture.md) | System overview, persistence tiers, data flow |
| [Newcomer Guide](explanation/newcomer-guide.md) | Orientation for new contributors |
| [Plugin Schema](explanation/plugin-schema.md) | Plugin system design (legacy v1/v2 reference) |
| [PVCAM Integration Map](explanation/pvcam-integration-map.md) | PVCAM driver integration points |
| [Rerun Visualization](explanation/rerun-visualization.md) | Rerun.io visualization debugging |
| [Echelle Extraction Architecture](explanation/echelle-extraction-architecture.md) | MVP local extractor design and planned evolution |

## Architecture Decision Records

See [ADR Index](adr/README.md) for all decisions with status and summaries.

## Archive

Historical documents (completed plans, one-off handoffs, superseded analyses) are preserved in [`docs/archive/`](archive/) for reference. These are not maintained.
