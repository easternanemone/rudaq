# Summary

[Introduction](README.md)

---

# Getting Started

- [Demo Mode](tutorials/demo-mode.md)
- [Build and Run](how-to/build-and-run.md)
- [Windows Build](how-to/build-and-run-windows.md)
- [Newcomer Guide](explanation/newcomer-guide.md)

# Architecture

- [System Overview](explanation/architecture.md)
- [Plugin Schema](explanation/plugin-schema.md)
- [Rerun Visualization](explanation/rerun-visualization.md)

# Guides

- [Operations & Deployment](how-to/operations.md)
- [Hardware Setup](how-to/hardware-setup.md)
- [Platform Notes](how-to/platform-notes.md)
- [Testing](how-to/testing.md)
- [Fast Inner Loop](how-to/fast-inner-loop.md)
- [Scripting](how-to/scripting.md)
- [LIBS Scripting](how-to/libs-scripting.md)
- [EOM Power Sweep](how-to/eom-power-sweep.md)
- [Web GUI](how-to/web-gui.md)
- [Plugins](how-to/plugins.md)

# Hardware Drivers

- [Driver Overview](how-to/hardware-drivers.md)
- [Device Config (Schema v3)](how-to/device-config.md)
- [Write a Device Manifest (v4)](how-to/write-a-device-manifest.md)
- [Legacy SCPI Deprecation](how-to/legacy-scpi-deprecation.md)
- [PVCAM Setup](how-to/pvcam-setup.md)
  - [PVCAM Integration Map](explanation/pvcam-integration-map.md)
- [Andor SDK3](how-to/driver-andor-sdk3.md)
  - [iStar sCMOS](how-to/driver-istar-scmos.md)
  - [Kymera 328i](how-to/driver-kymera-328i.md)
  - [Crash Capture & Repro](how-to/andor-istar-crash-capture-and-repro.md)
- [Dover Motion](how-to/driver-dover-motion.md)
- [Spirit Laser](how-to/driver-spirit-laser.md)
- [Comedi Setup](how-to/comedi-setup.md)

# Storage & Data

- [Storage Formats](how-to/storage-formats.md)
- [Zarr Acquisition](how-to/zarr-acquisition.md)

# SQLite Control Plane

- [Current DB Backend & Historical SurrealDB Notes](how-to/surrealdb-integration.md)
- [Migration & Rollback](how-to/migration-rollback-toolkit.md)
- [Maitai Signoff](how-to/maitai-universal-db-signoff.md)
- [LEABS Signoff](how-to/leabs-universal-db-signoff.md)

# Echelle Spectroscopy

- [Extraction Architecture](explanation/echelle-extraction-architecture.md)
- [Spectrum Preview](how-to/echelle-spectrum-preview.md)
- [Calibration Development](how-to/echelle-calibration-development.md)
- [Live Calibration Session](how-to/echelle-live-calibration-session-leabs-dev.md)
- [Rollout & Troubleshooting](how-to/echelle-rollout-and-troubleshooting.md)
- [Validation Plan & HIL Checklist](how-to/echelle-validation-plan-and-hil-checklist.md)

# Reference

- [gRPC API](reference/grpc-api.md)
- [Feature Matrix](reference/feature-matrix.md)
- [Driver Capability Matrix](reference/driver-capability-matrix.md)
- [Device Metadata Contract](reference/device-metadata-contract.md)
- [Streaming Policy](reference/streaming-policy.md)
- [Hardware Inventory](reference/inventory.md)
- [Hardware Qualification Runners](reference/hardware-qualification-runner-plan.md)
- [UI Workflow Costs](reference/ui-workflow-costs.md)
- [PVCAM SDK](reference/pvcam-sdk.md)
- [Dover Motion API](reference/dover-motion-api.md)
- [Echelle Calibration Profile Schema](reference/echelle-calibration-profile-schema.md)
- [Echelle Spectrum Streaming Protocol](reference/echelle-spectrum-streaming-protocol-design.md)
- [Deprecation & Removal Plan](reference/deprecation-plan.md)

# Architecture Decision Records

- [ADR Index](adr/README.md)
- [001 — Capability Consolidation](adr/001-capability-consolidation.md)
- [002 — Connection Reliability](adr/002-connection-reliability.md)
- [003 — gRPC Validation Layer](adr/003-grpc-validation-layer.md)
- [004 — Panic Safety](adr/004-panic-safety.md)
- [005 — Pool Error Handling](adr/005-pool-error-handling.md)
- [006 — Pool Migration Rollback](adr/006-pool-migration-rollback.md)
- [007 — PVCAM 85-Frame Stall Fix](adr/007-pvcam-85-frame-stall-fix.md)
- [008 — PVCAM Continuous Acquisition](adr/008-pvcam-continuous-acquisition.md)
- [009 — PVCAM Driver Architecture](adr/009-pvcam-driver-architecture.md)
- [010 — PVCAM Pool Migration Results](adr/010-pvcam-pool-migration-results.md)
- [011 — PVCAM SDK Pattern Compliance](adr/011-pvcam-sdk-pattern-compliance.md)
- [012 — Mechelle Echelle MVP Location](adr/012-mechelle-echelle-mvp-extraction-location.md)
- [013 — Calibration Profile Ownership](adr/013-mechelle-calibration-profile-ownership-and-compatibility.md)
- [014 — Frame Streaming Buffer Reuse](adr/014-frame-streaming-buffer-reuse.md)
- [015 — Hybrid Persistence Architecture](adr/015-hybrid-persistence-architecture.md)
- [UI Slint Role](adr/ui-slint-role.md)
- [Runtime Driver Policy](adr/runtime-driver-policy.md)
- [PVCAM Performance Gap Analysis](adr/analysis-pvcam-performance-gap.md)
