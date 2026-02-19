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

### Infrastructure

| Guide | Description |
|-------|-------------|
| [Testing](how-to/testing.md) | Test runner, profiles, hardware tests, coverage |
| [Plugins](how-to/plugins.md) | Config-only, native Rust, and Rhai plugins |
| [SurrealDB Integration](how-to/surrealdb-integration.md) | Embedded database setup |

## Reference — Look up details

| Reference | Description |
|-----------|-------------|
| [gRPC API](reference/grpc-api.md) | Protocol and service definitions |
| [Device Metadata Contract](reference/device-metadata-contract.md) | Capability/command/UI metadata contract for advanced panels |
| [PVCAM SDK](reference/pvcam-sdk.md) | PVCAM API reference and error codes |
| [Dover Motion API](reference/dover-motion-api.md) | MotionSynergyAPI reference |
| [Feature Matrix](reference/feature-matrix.md) | Implementation status for all features |

## Explanation — Understand the system

| Document | Description |
|----------|-------------|
| [Architecture](explanation/architecture.md) | System overview, component diagrams, data flow |
| [Newcomer Guide](explanation/newcomer-guide.md) | Orientation for new contributors |
| [Plugin Schema](explanation/plugin-schema.md) | Plugin system design |
| [PVCAM Integration Map](explanation/pvcam-integration-map.md) | PVCAM driver integration points |
| [Rerun Visualization](explanation/rerun-visualization.md) | Rerun.io visualization debugging |

## Architecture Decision Records

See [ADR Index](adr/README.md) for all decisions with status and summaries.
