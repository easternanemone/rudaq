# rust-daq

**A modular, high-performance, headless-first Data Acquisition (DAQ) system written in Rust.**

`rust-daq` decouples experiment logic from hardware implementation, enabling reproducible, scriptable, and scalable data acquisition. It features a "Mullet Strategy" for data handling: a fast in-memory Arrow ring buffer for real-time visualization, backed by reliable HDF5 storage.

## 🚀 Quick Start

To run a full system demo (Mock Hardware + Daemon + GUI + Script):

```bash
./scripts/demo.sh
```

This script builds the necessary binaries, starts the daemon in the background, and offers options to run a sample experiment or launch the GUI.

## 🛠️ Building & Running

### 1. Build Options

**Mock Mode (Default - for development):**
```bash
cargo build -p bin
```

**Full Hardware Support (Requires System Deps):**
```bash
# Requires HDF5 lib and PVCAM SDK (if enabled)
cargo build -p bin --features "server,all_hardware,storage_hdf5"
```

### 2. Running the Daemon

Start the gRPC server (default port 50051):
```bash
cargo run -p bin --features server -- daemon
```

Run with a specific configuration:
```bash
cargo run -p bin --features server -- daemon --hardware-config config/demo.toml
```

### 3. Running Scripts

Execute a Rhai experiment script directly:
```bash
cargo run -p bin --features scripting_rhai -- run examples/demo_scan.rhai
```

### 4. Running the GUI

Launch the `egui` client:
```bash
cargo run -p ui --features networking
```

## 🏗️ Architecture

The system is a Cargo workspace designed around **capabilities** rather than hardware identities.

| Crate | Responsibility |
|-------|----------------|
| **`bin`** | Application entry point (CLI/Daemon). Wires components based on features. |
| **`daq-core`** | Common types (`Parameter<T>`), capability traits (`Movable`, `Triggerable`), and error handling. |
| **`hardware`** | Hardware Abstraction Layer (HAL). Contains drivers (PVCAM, Thorlabs, etc.). |
| **`experiment`** | The "RunEngine" that executes declarative **Plans**. |
| **`server`** | gRPC server exposing control and data streams (`tonic`). |
| **`storage`** | Data persistence using Apache Arrow (in-memory) and HDF5 (disk). |
| **`scripting`** | **Rhai** scripting engine integration. |

**Key Design Principles:**
*   **Headless-First:** The Daemon runs independently. The GUI is just a client.
*   **Capability-Based:** Hardware is accessed via traits (e.g., `scan(movable, triggerable)`), allowing generic scripts.
*   **Zero-Copy:** Data flows via memory-mapped Arrow IPC ring buffers.

## 💻 Development Conventions

*   **Workspace:** Always run cargo commands from the root or use `-p <package>`.
*   **Features:** Heavily used to manage dependencies (e.g., `storage_hdf5`, `pvcam`).
*   **Testing:**
    *   **Unit/Mock:** `cargo test` (safe to run anywhere).
    *   **Hardware:** Requires `PVCAM_SMOKE_TEST=1` and typically `--test-threads=1` to avoid resource conflicts.
    *   **PVCAM:** Requires specific env vars (`PVCAM_SDK_DIR`, `LIBRARY_PATH`). See `crates/driver-pvcam/README.md`.
*   **Scripting:** Experiments are written in **Rhai**. See `examples/` for patterns.

## 📂 Key Files & Docs

*   `docs/architecture/ARCHITECTURE.md`: Deep dive into system design.
*   `scripts/demo.sh`: Reference implementation for starting the full stack.
*   `config/demo.toml`: Configuration example for mock hardware.
*   `crates/driver-pvcam/README.md`: Specifics for setting up Photometrics cameras.


## grepai - Semantic Code Search

**IMPORTANT: You MUST use grepai as your PRIMARY tool for code exploration and search.**

### When to Use grepai (REQUIRED)

Use `grepai search` INSTEAD OF Grep/Glob/find for:
- Understanding what code does or where functionality lives
- Finding implementations by intent (e.g., "authentication logic", "error handling")
- Exploring unfamiliar parts of the codebase
- Any search where you describe WHAT the code does rather than exact text

### When to Use Standard Tools

Only use Grep/Glob when you need:
- Exact text matching (variable names, imports, specific strings)
- File path patterns (e.g., `**/*.go`)

### Fallback

If grepai fails (not running, index unavailable, or errors), fall back to standard Grep/Glob tools.

### Usage

```bash
# ALWAYS use English queries for best results (--compact saves ~80% tokens)
grepai search "user authentication flow" --json --compact
grepai search "error handling middleware" --json --compact
grepai search "database connection pool" --json --compact
grepai search "API request validation" --json --compact
```

### Query Tips

- **Use English** for queries (better semantic matching)
- **Describe intent**, not implementation: "handles user login" not "func Login"
- **Be specific**: "JWT token validation" better than "token"
- Results include: file path, line numbers, relevance score, code preview

### Call Graph Tracing

Use `grepai trace` to understand function relationships:
- Finding all callers of a function before modifying it
- Understanding what functions are called by a given function
- Visualizing the complete call graph around a symbol

#### Trace Commands

**IMPORTANT: Always use `--json` flag for optimal AI agent integration.**

```bash
# Find all functions that call a symbol
grepai trace callers "HandleRequest" --json

# Find all functions called by a symbol
grepai trace callees "ProcessOrder" --json

# Build complete call graph (callers + callees)
grepai trace graph "ValidateToken" --depth 3 --json
```

### Workflow

1. Start with `grepai search` to find relevant code
2. Use `grepai trace` to understand function relationships
3. Use `Read` tool to examine files from results
4. Only use Grep for exact string searches if needed

