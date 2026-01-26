# Refactoring Plan: Migrating to Generic Drivers

**Author**: Pickle Rick
**Date**: 2026-01-25

## Overview
We have introduced a `GenericSerialDriver` that can replace bespoke Rust drivers for simple serial instruments. This document outlines the plan to migrate existing hardcoded drivers to this new system.

## Phase 1: Hybrid Mode (Current State)
*   **Goal**: Ensure `GenericSerialDriver` works alongside existing drivers.
*   **Status**: Done. `daq-driver-generic` is in the workspace.
*   **Action**: Use `daq-driver-generic` for *new* simple serial devices (e.g., temperature controllers, simple pumps).

## Phase 2: Migrate Thorlabs ELL14
*   **Target**: `crates/daq-driver-thorlabs`
*   **Analysis**: The ELL14 protocol is simple ASCII ("0ma00000000"). It maps perfectly to `InstrumentConfig` + Regex.
*   **Steps**:
    1.  Create `config/devices/thorlabs_ell14.toml` using the schema.
    2.  Test `GenericSerialDriver` with this config against a real or mock ELL14.
    3.  Verify `Movable` trait behavior matches the hardcoded driver.
    4.  Remove `crates/daq-driver-thorlabs` from workspace.

## Phase 3: Migrate Newport ESP300
*   **Target**: `crates/daq-driver-newport`
*   **Analysis**: ESP300 is more complex but still ASCII. It might require the `scripting` feature for complex initialization sequences.
*   **Steps**:
    1.  Draft `config/devices/newport_esp300.toml`.
    2.  Identify any logic that cannot be expressed in TOML (e.g., complex homing routines).
    3.  Implement these as Rhai scripts in the config.
    4.  Test and deprecate the Rust crate.

## Phase 4: Cleanup `daq-hardware`
*   **Target**: `crates/daq-hardware`
*   **Analysis**: This crate contains a lot of legacy traits and factory logic.
*   **Steps**:
    1.  Once most drivers are generic, simplify the `DriverFactory` trait.
    2.  Remove unused "helper" code that was only used by legacy drivers.

## Validation Strategy
*   Use `daq-driver-mock` to simulate device responses for regression testing during migration.
