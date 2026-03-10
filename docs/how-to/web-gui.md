# Web GUI (WASM Build)

The `ui` crate supports multiple deployment modes:
- **Native GUI** (`rust-daq-gui`) — standalone eframe application
- **WASM GUI** (`rust-daq-web`) — WebAssembly build for browser deployment
- **Rerun Viewer** — integrated live visualization panel

This guide covers the WASM build, which lets the control panel run in Chrome
while the hardware daemon runs natively on a lab machine.

## Prerequisites

```bash
# Install Trunk (WASM build + dev server)
cargo install trunk --locked

# Add the wasm32 target if not already present
rustup target add wasm32-unknown-unknown
```

## Development Build

```bash
cd crates/ui
trunk serve
# → http://127.0.0.1:8080
```

`trunk serve` watches for source changes and rebuilds automatically.

## Production Build

```bash
cd crates/ui
trunk build --release
# → dist/  (index.html + .wasm + .js)
```

Serve `dist/` from any static file host (nginx, GitHub Pages, S3, etc.).

## Connecting to the Daemon

1. Open the URL in Chrome.
2. Enter the daemon's address (e.g. `http://10.0.0.40:50051`) and click **Connect**.
3. The GUI lists all registered devices and renders config-driven panels from
   each device's `[ui.control_panel]` TOML section (delivered via gRPC metadata).

> **CORS**: Browser access depends on the daemon's `grpc.allowed_origins` setting in `config/config.v4.toml`.
> The current default config ships with `allowed_origins = ["*"]` for lab convenience, but production deployments should restrict this to explicit origins.
> If the web page is served from a different origin than the daemon, add that origin to `allowed_origins`.

## Architecture

```
Browser (WASM)                    Lab Machine (native)
──────────────────                ──────────────────────────
DaqWebApp (egui)                  rust-daq-daemon
  │                                 │
  │──── gRPC-web (HTTP/1.1) ────▶  tonic_web proxy
  │                                 │──▶ gRPC services
  │◀─── DeviceInfo + metadata ──────┘
  │
  └── web_schema::UiConfig
      (deserialised from DeviceMetadata.ui_schema_json)
```

### Key crate boundaries

| File | Purpose |
|------|---------|
| `src/web_main.rs` | WASM entry (`#[wasm_bindgen(start)]`), `DaqWebApp` impl |
| `src/web_schema.rs` | Serde-only mirror of `hardware::config::schema` UI types |
| `src/runtime.rs` | Platform runtime (`tokio::Runtime` on native, `spawn_local` on WASM) |
| `Trunk.toml` | Trunk build configuration |
| `index.html` | HTML shell with `<canvas id="daq_canvas">` |

### Transport

The `client` crate uses a `Transport` type alias:
- **Native**: `tonic::transport::Channel` (full HTTP/2)
- **WASM**: `tonic_web_wasm_client::Client` (HTTP/1.1 gRPC-web)

This is pinned to `tonic-web-wasm-client = "=0.5.0"` — the only release
compatible with tonic 0.10. Bump this when the workspace upgrades tonic.

## Cargo Feature Flags

The web binary is gated behind `--features web --no-default-features`:

```bash
# Manual cargo build (Trunk does this automatically)
cargo build -p ui \
  --target wasm32-unknown-unknown \
  --features web \
  --no-default-features \
  --bin rust-daq-web
```

The native GUI (`rust-daq-gui`) is unaffected — it continues to build with
`--features standalone` as before.
