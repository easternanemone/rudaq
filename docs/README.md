# rust-daq Documentation

Welcome to the documentation for the `rust-daq` project. This documentation is organized to help you understand the architecture, use the system, and extend its capabilities.

## Documentation Structure

### 🏗️ [Architecture](./architecture/)
High-level design documents explaining the core principles of the V5 "Headless-First" architecture.
- [System Architecture](./architecture/01_system_architecture.md): The master design document.
- [Configuration](./architecture/02_configuration.md): How configuration is handled (Figment, layering).
- [Hardware Communication](./architecture/03_hardware_communication.md): Protocols and patterns.
- [gRPC API](./architecture/04_grpc_api.md): The remote control interface.

### 📚 [Guides](./guides/)
Practical how-to guides for users and developers.
- **For Users**:
  - [CLI Guide](./guides/cli_guide.md): How to use the command-line interface.
  - [Client Examples](./guides/client_examples.md): Usage examples for the CLI and clients.
  - [egui GUI Quickstart](./guides/egui_gui_quickstart.md): How to run and extend the native egui desktop GUI.
  - [Scripting Guide](./guides/scripting/README.md): How to write Rhai scripts for experiments.
- **For Developers**:
  - [Driver Development](./guides/driver_development.md): Implementing new hardware drivers.
  - [Python Integration](./guides/python_integration.md): Using the Python engine.
  - [Agent Guide](./guides/agent_guide.md): Workflows for AI agents.
  - [Tools](./guides/tools/): Helper scripts and tools.

### 📖 [Reference](./reference/)
Technical reference material.
- [Instruments](./reference/instruments/): Manuals, findings, and protocol details for specific hardware (MaiTai, Elliptec, etc.).
- [Hardware Inventory](./reference/hardware_inventory.md): List of supported and available hardware.
- [Hardware Testing Strategy](./reference/hardware_testing_strategy.md): Testing methodologies.
- [PVCAM SDK](./reference/pvcam-sdk/): Documentation for the PVCAM camera SDK.
