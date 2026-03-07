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
| [Operations](how-to/operations.md) | Daemon startup, deployment, monitoring |
| [Maitai Universal+DB Signoff](how-to/maitai-universal-db-signoff.md) | Hardware validation runbook for hybrid-db mode |
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
| [Storage Formats](how-to/storage-formats.md) | HDF5, Arrow, CSV, NetCDF |
| [Zarr Acquisition](how-to/zarr-acquisition.md) | Zarr V3 storage |
| [EOM Power Sweep](how-to/eom-power-sweep.md) | EOM power sweep workflow |
| [Echelle Spectrum Preview (MVP)](how-to/echelle-spectrum-preview.md) | Load a calibration profile and view local echelle extraction previews |
| [Live Echelle Calibration Session (leabs-dev)](how-to/echelle-live-calibration-session-leabs-dev.md) | Bench runbook for Mechelle+iSTAR+HG-2 live calibration and GUI workflow |
| [Andor iSTAR Crash Capture & Repro](how-to/andor-istar-crash-capture-and-repro.md) | Wrapped-daemon crash capture and streaming repro harness for `leabs-dev` |
| [Echelle Calibration Development](how-to/echelle-calibration-development.md) | Developer workflow for creating profiles and maintaining golden datasets |
| [Echelle Rollout & Troubleshooting](how-to/echelle-rollout-and-troubleshooting.md) | MVP rollout stages, runtime knobs, and failure troubleshooting |
| [Echelle Validation Plan & HIL Checklist](how-to/echelle-validation-plan-and-hil-checklist.md) | Dataset matrix definition and lab hardware-in-loop validation checklist |

### Infrastructure

| Guide | Description |
|-------|-------------|
| [Testing](how-to/testing.md) | Test runner, profiles, hardware tests, coverage |
| [Plugins](how-to/plugins.md) | Config-only, native Rust, and Rhai plugins |
| [SurrealDB Integration](how-to/surrealdb-integration.md) | Embedded database setup |
| [Web GUI](how-to/web-gui.md) | WASM build, deployment, and architecture |
| [Migration/Rollback Toolkit](how-to/migration-rollback-toolkit.md) | Backup, restore, and incident rollback procedures |
| [LEABS Universal+DB Signoff](how-to/leabs-universal-db-signoff.md) | LEABS hardware validation runbook for hybrid-db mode |
| [LIBS Scripting](how-to/libs-scripting.md) | Rhai API for LIBS experiments |

## Reference — Look up details

| Reference | Description |
|-----------|-------------|
| [gRPC API](reference/grpc-api.md) | Protocol and service definitions |
| [Device Metadata Contract](reference/device-metadata-contract.md) | Capability/command/UI metadata contract for advanced panels |
| [PVCAM SDK](reference/pvcam-sdk.md) | PVCAM API reference and error codes |
| [Dover Motion API](reference/dover-motion-api.md) | MotionSynergyAPI reference |
| [Feature Matrix](reference/feature-matrix.md) | Implementation status for all features |
| [Echelle Calibration Profile Schema](reference/echelle-calibration-profile-schema.md) | Versioned profile schema for Mechelle/echelle extraction calibration |
| [Echelle Sidecar API Contract](reference/echelle-sidecar-api-contract.md) | Design for Python sidecar extraction contract, packaging, and licensing guidance |
| [Echelle Spectrum Streaming Protocol Design](reference/echelle-spectrum-streaming-protocol-design.md) | Design for gRPC vector-spectrum payload streaming and metadata |

## Explanation — Understand the system

| Document | Description |
|----------|-------------|
| [Architecture](explanation/architecture.md) | System overview, component diagrams, data flow |
| [Newcomer Guide](explanation/newcomer-guide.md) | Orientation for new contributors |
| [Plugin Schema](explanation/plugin-schema.md) | Plugin system design |
| [PVCAM Integration Map](explanation/pvcam-integration-map.md) | PVCAM driver integration points |
| [Rerun Visualization](explanation/rerun-visualization.md) | Rerun.io visualization debugging |
| [Echelle Extraction Architecture](explanation/echelle-extraction-architecture.md) | MVP local extractor design and planned sidecar/protocol evolution |

## Architecture Decision Records

See [ADR Index](adr/README.md) for all decisions with status and summaries.

## Architecture Policies

| Policy | Description |
|--------|-------------|
| [Runtime Driver Policy](architecture/runtime-driver-policy.md) | Universal vs native boundaries and SurrealDB role by runtime mode |

## Plans — Project Roadmaps

| Plan | Description |
|------|-------------|
| [Mechelle Echelle Spectrum Workstream A](plans/mechelle-echelle-spectrum-workstream-a.md) | Echelle extraction epic: scope, assumptions, MVP outcomes |
| [Test Suite Overhaul](plans/test-suite-overhaul.md) | Test reorganization for hybrid-db runtime mode |
