# rust-daq

`rust-daq` is a high-performance, modular data acquisition (DAQ) application written in Rust. It is designed for scientific and industrial applications that require real-time data acquisition, processing, and visualization.

## V2 Architecture

The `rust-daq` application has been recently migrated to a modern, actor-based V2 architecture. This new architecture provides several key benefits over the legacy V1 design, including:

*   **Improved Performance**: The actor model eliminates the lock contention and performance bottlenecks of the previous `Arc<Mutex<>>`-based design.
*   **Enhanced Scalability**: The V2 architecture is designed to be highly scalable, allowing for the easy addition of new instruments, data processors, and other components.
*   **Rich Data Type Support**: The V2 architecture natively supports the `Measurement` enum, which allows for the handling of rich data types like images and spectra.
*   **Clear Separation of Concerns**: The V2 architecture enforces a clear separation of concerns between the core application logic, the instrument drivers, and the GUI.

For a more detailed overview of the V2 architecture, please see the [rust-daq Application Architecture Guide](rust-daq-app-architecture.md).

## Getting Started

To get started with `rust-daq`, please see the [Getting Started Guide](rust-daq-getting-started.md).
