# ui [PRIMARY]

[PRIMARY] Main egui-based user interface crate for rust-daq.

## Overview

The `ui` crate provides the primary operator-facing interfaces for the headless daemon. It supports native desktop usage, browser/WASM deployment, and an optional Rerun-enabled workflow.

## Binaries

Current binaries from `Cargo.toml`:

- `rust-daq-gui` — native desktop GUI
- `rust-daq-web` — browser/WASM GUI (`--features web`)
- `daq-rerun` — native Rerun-enabled viewer (`--features rerun_viewer`)

## Features

| Feature | Purpose |
|---------|---------|
| `standalone` | Native egui desktop application (default) |
| `rerun_viewer` | Embedded/native Rerun viewer support |
| `pvcam` | Local/mock PVCAM integration in the UI crate |
| `pvcam_sdk` / `pvcam_hardware` | Real PVCAM SDK support |
| `storage_hdf5` | HDF5-backed run comparison/annotation |
| `web` | WASM/browser build |

## Quick Start

```bash
# Start a daemon in another terminal
cargo run --bin rust-daq-daemon -- daemon --hardware-config config/demo.toml

# Start the native GUI
cargo run -p ui --bin rust-daq-gui --features standalone
```

## Web GUI

The WASM/web build lives in the same crate:

```bash
cd crates/ui
trunk serve
```

See `docs/how-to/web-gui.md` for deployment and CORS details.

## PVCAM + Rerun Notes

Older documentation referred to UI features named `arrow` and `pvcam_arrow`. Those are not current `ui` crate features. For current PVCAM + Rerun workflows, use the actual feature flags from `Cargo.toml`, typically:

- `rerun_viewer`
- `pvcam` for local/mock PVCAM support
- `pvcam_sdk` when real PVCAM SDK support is needed

## Related Docs

- `docs/how-to/web-gui.md`
- `docs/explanation/rerun-visualization.md`
- `crates/client/README.md`
