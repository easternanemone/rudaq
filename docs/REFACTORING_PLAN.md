# Refactoring Plan: Migrating to Generic Drivers

**Author**: Pickle Rick
**Date**: 2026-01-25

## Overview
We have introduced a `GenericSerialDriver` that can replace bespoke Rust drivers for simple serial instruments. This document outlines the plan to migrate existing hardcoded drivers to this new system.

## Phase 1: Hybrid Mode ✅ Complete
*   **Goal**: Ensure `GenericSerialDriver` works alongside existing drivers.
*   **Status**: Done. `driver-generic` is in the workspace.
*   **Action**: Use `driver-generic` for *new* simple serial devices (e.g., temperature controllers, simple pumps).

## Phase 2: Driver Decoupling ✅ Complete
*   **Goal**: Extract drivers from `crates/hardware/src/drivers/` into standalone crates.
*   **Status**: Done. All major drivers now have standalone crates:
    - `crates/driver-thorlabs/` — ELL14 rotation mounts
    - `crates/driver-newport/` — ESP300 motion controller + 1830-C power meter
    - `crates/driver-spectra-physics/` — MaiTai laser
    - `crates/driver-mock/` — Mock devices for testing
    - `crates/driver-pvcam/` — PVCAM cameras (was already standalone)
    - `crates/driver-comedi/` — Linux Comedi DAQ
    - `crates/driver-andor-sdk3/` — Andor iStar camera (new)
    - `crates/driver-dover-motion/` — Dover Motion SmartStage (new)
    - `crates/driver-red-pitaya/` — Red Pitaya FPGA
*   **Note**: Legacy code in `crates/hardware/src/drivers/` still exists alongside the new standalone crates and has not yet been deleted.

## Phase 3: Migrate Thorlabs ELL14 to Generic (Future)
*   **Target**: `crates/driver-thorlabs` → `driver-generic` config
*   **Analysis**: The ELL14 protocol is simple ASCII ("0ma00000000"). It maps perfectly to `InstrumentConfig` + Regex.
*   **Steps**:
    1.  Create `config/devices/thorlabs_ell14.toml` using the schema.
    2.  Test `GenericSerialDriver` with this config against a real or mock ELL14.
    3.  Verify `Movable` trait behavior matches the hardcoded driver.
    4.  Remove `crates/driver-thorlabs` from workspace.

## Phase 4: Migrate Newport ESP300 to Generic (Future)
*   **Target**: `crates/driver-newport` → `driver-generic` config
*   **Analysis**: ESP300 is more complex but still ASCII. It might require the `scripting` feature for complex initialization sequences.
*   **Steps**:
    1.  Draft `config/devices/newport_esp300.toml`.
    2.  Identify any logic that cannot be expressed in TOML (e.g., complex homing routines).
    3.  Implement these as Rhai scripts in the config.
    4.  Test and deprecate the Rust crate.

## Phase 5: Cleanup `hardware` (Future)
*   **Target**: `crates/hardware`
*   **Analysis**: This crate still contains legacy driver code in `src/drivers/` alongside the HAL traits and `DeviceRegistry`. Once legacy drivers are confirmed unused, they can be removed.
*   **Steps**:
    1.  Verify no code paths still instantiate legacy drivers from `hardware/src/drivers/`.
    2.  Remove legacy driver code from `hardware`.
    3.  Simplify the `DriverFactory` trait if possible.

## Validation Strategy
*   Use `driver-mock` to simulate device responses for regression testing during migration.
