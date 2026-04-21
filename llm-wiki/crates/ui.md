# crate: `ui`

<!--
last-ingested: 2026-04-19
sources:
  - crates/ui/Cargo.toml
  - crates/ui/src/panels/image_viewer/
  - docs/explanation/rerun-visualization.md
  - docs/reference/ui-workflow-costs.md
  - docs/how-to/wasm-dom-interop.md
see-also:
  - ./client.md
  - ./ui-graph.md
  - ../workflows/build-test-lint.md
-->

**Role:** Primary operator UI. `egui` + `egui_dock`. Produces:

- `rust-daq-gui` — native desktop (via `eframe`).
- `rust-daq-web` — browser WASM (via `trunk`).

**Connectivity:** gRPC on native, gRPC-web on WASM.

**Key modules:**

- `widgets/device_controls/generic_panel.rs` — `GenericDevicePanel`; capability-based auto-composition. Adding a new driver automatically gets a working panel (except cameras, which use `ImageViewerPanel`).
- `panels/image_viewer/` — decomposed module for frame streaming (`mod.rs`, `processing.rs`, `colormap.rs`, `types.rs`, plus echelle extension files `echelle_extraction.rs`, `echelle_profile_cache.rs`, `echelle_sidecar.rs`).

**Build notes:**

- **Excluded from the default workspace clippy gate** (needs WASM target separately).
- WASM check: `cargo check -p ui --lib --target wasm32-unknown-unknown --no-default-features --features web`.
- DOM interop details in `docs/how-to/wasm-dom-interop.md`.

**Deprecated (sunset v1.0):** `CodePreviewPanel::ui()` method — removed.
