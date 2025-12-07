# rust-daq Documentation

Welcome to the documentation for the `rust-daq` project. This documentation covers the V5 "Headless-First" architecture with reactive parameters, scriptable control, and remote GUI support.

## 🔍 Searching Documentation

This documentation is **indexed for semantic search** using CocoIndex. Instead of browsing files, you can search by natural language queries to quickly find relevant information.

**Quick Search:**
```bash
# From repository root
python scripts/search_hybrid.py --query "your question" --mode comprehensive
```

**Example Queries:**
- "How does V5 parameter reactive system work?"
- "What are the async hardware callback patterns?"
- "How to integrate Python client with gRPC?"
- "MaiTai laser communication protocol"

**Search Categories:**
- `architecture` - Design decisions, ADRs, V5 architecture
- `guides` - How-to guides, tutorials, development workflows
- `reference` - Hardware protocols, API references, testing strategies
- `instruments` - Device-specific protocols and findings
- `getting_started` - Setup and onboarding
- `tools` - Development tools and utilities

See [Hybrid Search Setup](./HYBRID_SEARCH_SETUP.md) for detailed usage and [CLAUDE.md](../CLAUDE.md#documentation-search--knowledge-base) for integration with AI assistants.

## Documentation Structure

### 🏗️ [Architecture](./architecture/)
High-level design documents explaining the core principles of the V5 "Headless-First" architecture.
- [System Architecture](./architecture/01_system_architecture.md): The master design document.
- [Configuration](./architecture/02_configuration.md): How configuration is handled (Figment, layering).
- [Hardware Communication](./architecture/03_hardware_communication.md): Protocols and patterns.
- [gRPC API](./architecture/04_grpc_api.md): The remote control interface.
- [V5 Architecture](./architecture/V5_ARCHITECTURE.md): Complete V5 architecture specification.
- [V5 Implementation Status](./architecture/V5_IMPLEMENTATION_STATUS.md): Current implementation status.
- [Reactive Parameters ADR](./architecture/ADR_005_REACTIVE_PARAMETERS.md): Design decisions for Parameter<T>.

### 📚 [Guides](./guides/)
Practical how-to guides for users and developers.
- **For Users**:
  - [CLI Guide](./guides/cli_guide.md): How to use the command-line interface.
  - [Scripting Guide](./guides/scripting/README.md): How to write Rhai scripts for experiments.
- **For Developers**:
  - [Driver Development](./guides/driver_development.md): Implementing new hardware drivers.
  - [Python Integration](./guides/python_integration.md): Using the Python engine.
  - [Agent Guide](./guides/agent_guide.md): Workflows for AI agents.
  - [Tools](./guides/tools/): Helper scripts and tools.

### 📖 [Reference](./reference/)
Technical reference material.
- [Instruments](./reference/instruments/): Manuals, findings, and protocol details for specific hardware.
  - [PVCAM Validation Checklist](./reference/instruments/PVCAM_VALIDATION_CHECKLIST.md): Prime BSI camera testing.
  - [PVCAM Hardware Validation](./reference/instruments/PVCAM_HARDWARE_VALIDATION.md): SDK installation and validation.
  - [PVCAM Operator Guide](./reference/instruments/PVCAM_OPERATOR_GUIDE.md): Operating the Prime BSI camera.
- [Hardware Inventory](./reference/hardware_inventory.md): List of supported and available hardware.
- [Hardware Testing Strategy](./reference/hardware_testing_strategy.md): Testing methodologies.
- [PVCAM SDK](./reference/pvcam-sdk/): Documentation for the PVCAM camera SDK.
