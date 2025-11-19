# rust-daq

`rust-daq` is a high-performance, modular data acquisition (DAQ) application written in Rust, designed for scientific and industrial applications.

## Project Status: Undergoing V4 Refactoring

**Warning:** This project is currently undergoing a major architectural refactoring to resolve critical design flaws. The `main` branch may be in a broken state as obsolete code is actively being removed and replaced.

The previous V1, V2, and V3 architectures have been deprecated in favor of a single, unified **V4 architecture**. The primary goal is to create a robust, maintainable, and scalable system.

For a detailed analysis of the issues that prompted this refactor, please see [ARCHITECTURAL_FLAW_ANALYSIS.md](./docs/architecture/ARCHITECTURAL_FLAW_ANALYSIS.md).

## V4 Architecture Overview

The new V4 architecture is being built on the following principles and technologies:

*   **Actor-Based Concurrency:** Using the **[Kameo](https://github.com/jprochazk/kameo)** framework, where each instrument is an isolated, stateful actor to ensure robustness and prevent deadlocks.
*   **High-Performance Data Handling:** Using **[Apache Arrow](https://arrow.apache.org/)** (`arrow-rs`) for in-memory data representation.
*   **Hierarchical Data Storage:** Using **HDF5** (`hdf5-rust`) for structured, scientific data storage, aligning with common industry practice.
*   **Modern Tooling:** Adopting best-in-class libraries for logging (`tracing`), configuration (`figment`), and plotting (`egui-plot`).

For a full list of chosen libraries, see [ADDITIONAL_LIBRARY_RESEARCH.md](./docs/architecture/ADDITIONAL_LIBRARY_RESEARCH.md).

The detailed V4 refactoring plan is being tracked in the `beads` issue tracker, under the main epic **`bd-xvpw`**.

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
