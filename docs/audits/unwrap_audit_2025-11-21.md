# .unwrap() Audit Report - 2025-11-21

**Issue**: bd-7g94
**Goal**: Replace production .unwrap() calls with proper error handling

## Summary

Total files with .unwrap(): 50
- Tests/Examples: 10 (acceptable)
- Production code: 40 (need review)

## Priority Classification

### CRITICAL (Hardware Drivers - Zero Tolerance)
Files in data path or hardware control that can cause system panics:

1. `src/hardware/pvcam.rs` - Camera driver
2. `src/hardware/ell14.rs` - Thorlabs rotation mount
3. `src/hardware/newport_1830c.rs` - Power meter
4. `src/hardware/mock.rs` - Mock hardware (test support)
5. `src/instrument/pvcam.rs` - Legacy camera (V1/V2)
6. `src/instrument/esp300.rs` - Newport motion controller
7. `src/instrument/elliptec.rs` - Elliptec driver
8. `src/instrument/newport_1830c.rs` - Legacy power meter

### CRITICAL (Data Plane)
Data acquisition and storage pipeline:

9. `src/data/ring_buffer.rs` - Memory-mapped ring buffer
10. `src/data/hdf5_writer.rs` - Background HDF5 writer
11. `src/data/fft.rs` - Real-time FFT processing
12. `src/data/iir_filter.rs` - IIR filter implementation
13. `src/data/storage.rs` - Storage backend

### CRITICAL (Network Layer)
gRPC server and remote control:

14. `src/grpc/server.rs` - gRPC daemon

### HIGH (Core Infrastructure)
Scripting, configuration, and experiment control:

15. `src/scripting/bindings.rs` - Rhai script bindings
16. `src/config_v4.rs` - V4 configuration system
17. `src/config.rs` - Legacy configuration
18. `src/config/versioning.rs` - Config versioning
19. `src/experiment/state.rs` - Experiment state machine
20. `src/instrument_manager_v3.rs` - V3 instrument manager
21. `src/session.rs` - Session management
22. `src/tracing_v4.rs` - Logging infrastructure

### MEDIUM (Modules & Capabilities)
Module system and capability traits:

23. `src/modules/camera.rs` - Camera module
24. `src/modules/power_meter.rs` - Power meter module
25. `src/modules/meta_instruments.rs` - Meta instruments
26. `src/modules/mod.rs` - Module orchestration
27. `src/hardware/capabilities.rs` - Capability trait impls
28. `src/instrument/capabilities.rs` - Legacy capabilities
29. `src/instrument/config.rs` - Instrument config

### MEDIUM (Actors - Deprecated)
V4 actor system (being removed, but still in use):

30. `src/actors/instrument_manager.rs` - Actor manager
31. `src/actors/data_publisher.rs` - Data publisher actor
32. `src/actors/newport_1830c.rs` - Power meter actor

### LOW (Utilities & Support)
Helper code and mocks:

33. `src/log_capture.rs` - Log capture (Mutex locks acceptable)
34. `src/parameter.rs` - Parameter types
35. `src/measurement/mod.rs` - Measurement types
36. `src/instrument/mock.rs` - Mock instrument
37. `src/instrument/mock_v3.rs` - V3 mock
38. `src/instrument/scpi.rs` - SCPI protocol helpers

### ACCEPTABLE (Tests & Examples)
These are fine to keep .unwrap():

- `tests/scripting_hardware.rs`
- `tests/scripting_standalone.rs`
- `tests/scripting_safety.rs`
- `tests/grpc_server_test.rs`
- `tests/mock_hardware.rs`
- `examples/v4_newport_hardware_test.rs`
- `examples/v4_newport_demo.rs`
- `examples/scripting_hardware_demo.rs`
- `examples/ring_buffer_demo.rs`
- `docs/timeout_test_cases.rs`

### OUT OF SCOPE
Build scripts (build-time only, panics acceptable):

- `pvcam-sys/build.rs`

## Detailed File Analysis

Analysis in progress...

## Fixes Applied

None yet.

## Remaining Work

1. Audit each CRITICAL file
2. Replace unwraps with proper error handling
3. Test after each file change
4. Document justified unwraps

## Notes

- Mutex lock unwraps are generally acceptable (lock poisoning is rare)
- Config loading at startup can use .expect() with clear messages
- Mock implementations can keep unwraps if used only in tests
