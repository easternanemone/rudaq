# rust-daq

`rust-daq` is a high-performance, headless-first data acquisition (DAQ) system written in Rust, designed for scientific and industrial applications.

## Architecture - V5 Headless-First Design

**Status**: ✅ V5 architecture fully implemented (as of 2025-11-20)

**rust-daq v5.0** implements a modern, script-driven DAQ system with complete separation of core daemon from UI:

- 🎯 **Capability-based hardware**: Atomic traits (`Readable`, `Movable`, `Triggerable`) for composable instruments
- 📝 **Script-driven**: Rhai and Python engines for flexible experiment logic without recompilation
- 🌐 **Remote-first**: gRPC API for headless operation and network control
- 📊 **High-throughput**: Arrow batching + HDF5 storage for scientific data
- 🔒 **Type-safe**: Pure Rust with async throughout, zero-copy data access
- 🛡️ **Crash resilient**: UI crashes don't stop experiments

**All legacy V1-V4 architectures have been removed** as of November 2025. See [V5_TRANSITION_COMPLETE.md](./docs/architecture/V5_TRANSITION_COMPLETE.md) for migration details.

### V5 Core Principles

1. **Headless-First**: Core daemon runs independently of any UI
2. **Capability Composition**: Hardware implements only capabilities it supports
3. **Script Extensibility**: Scientists write experiment logic in Rhai/Python
4. **Network Transparency**: Remote control via gRPC from any platform
5. **Zero-Copy Data**: Memory-mapped Arrow buffers accessible from Python

## Quick Start (Headless-First Architecture)

The system now supports a scriptable, headless-first architecture using Rhai scripts:

```bash
# Run a simple stage scan
cargo run -- run examples/simple_scan.rhai

# Run a triggered acquisition workflow
cargo run -- run examples/triggered_acquisition.rhai

# Start daemon for remote control (Phase 3 - not yet implemented)
cargo run -- daemon --port 50051

# Show help
cargo run -- --help
```

## Example Scripts

### Simple Scan (`examples/simple_scan.rhai`)
```rhai
// Simple stage scan experiment
print("Starting scan...");

for i in 0..10 {
    let pos = i * 1.0;
    stage.move_abs(pos);
    print(`Moved to ${pos}mm`);
    sleep(0.1);
}

print("Scan complete!");
```

### Triggered Acquisition (`examples/triggered_acquisition.rhai`)
```rhai
// Camera triggered acquisition
print("Setting up acquisition...");

camera.arm();
print("Camera armed");

for i in 0..5 {
    let pos = i * 2.0;
    stage.move_abs(pos);
    stage.wait_settled();
    camera.trigger();
    print(`Frame ${i+1} captured at ${pos}mm`);
}

print("Acquisition complete!");
```

## Getting Started

To get started with the development, please see the [Getting Started Guide](./docs/getting_started/rust-daq-getting-started.md). Note that some of this documentation may be outdated until the V4 refactor is complete.
