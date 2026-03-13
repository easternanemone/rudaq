# UI Workflow Costs (bd-pman.6.1)

This document summarizes **practical maintainer/operator costs** for the current UI workflows in `rust-daq`.
It is intentionally decision-focused (coverage, operational constraints, maintenance weight), not a synthetic benchmark report.

## Scope and data sources

- `crates/ui` [PRIMARY] (egui native + WASM + optional Rerun integration)
- `crates/ui-slint` [EXPERIMENTAL] (evaluation crate)
- Existing docs/code comments for Rerun and web behavior
- Current repository artifact sizes in `crates/ui/dist`

> Notes:
> - Counts below are from the current tree snapshot.
> - WASM bundle size is from existing `crates/ui/dist` output (not a fresh rebuild in this task).

## 1) UI surface inventory

### A) Native egui (`crates/ui`, `standalone`)

- Binary: `rust-daq-gui`
- Primary operator UI for desktop/lab use.
- Code footprint:
  - `crates/ui`: **108** `.rs` files total
  - `crates/ui/src`: **106** `.rs` files, **60,053** LOC
  - `crates/ui/tests`: **2** `.rs` files, **1,049** LOC
  - Panel files under `src/panels`: **41** `.rs`
  - Widget files under `src/widgets`: **27** `.rs`
- Cargo complexity (`crates/ui/Cargo.toml`):
  - Top-level deps: **25**
  - Target-specific deps: native **18**, wasm **10**, unix **1**
  - Features: **8**
  - Binaries: **3**

### B) WASM egui (`crates/ui`, `web` feature)

- Binary: `rust-daq-web` (wasm32 + Trunk)
- Same `DaqApp` architecture as native with browser-specific connection flow.
- Build/deploy shape:
  - `cd crates/ui && trunk serve` or `trunk build --release`
  - PWA assets present (`manifest.json`, `sw.js`)
- Existing dist artifact sizes (`crates/ui/dist`):
  - WASM: `rust-daq-web-..._bg.wasm` = **8,104,004 bytes** (~7.7 MiB)
  - JS glue: `rust-daq-web-....js` = **89,109 bytes** (~87 KiB)

### C) Rerun workflow [AUXILIARY] (optional `rerun_viewer`)

- Binary: `daq-rerun`
- Feature-gated in `crates/ui` (no separate crate for the main workflow).
- Uses `rerun = 0.27.3` (with `native_viewer`, `sdk`, `server` features).
- Current workflow focus is camera visualization (PVCAM-linked preview path + Rerun viewer integration).

### D) Slint evaluation [EXPERIMENTAL] (`crates/ui-slint`)

- Crate exists and builds as native + wasm evaluation surface.
- Binaries: `ui-slint`, `ui-slint-bench-tokio`
- Code footprint:
  - `crates/ui-slint`: **7** `.rs` files total
  - `crates/ui-slint/src`: **6** `.rs` files, **1,184** LOC
- Cargo complexity (`crates/ui-slint/Cargo.toml`):
  - Top-level deps: **1** (`slint`)
  - Target-specific deps: native **2**, wasm **6**
  - Build-deps: **1**
  - Features: **1** (`web`)

## 2) Feature coverage by surface

| Capability area | Native egui (`standalone`) | WASM egui (`web`) | Rerun (`rerun_viewer`) | `ui-slint` eval |
|---|---|---|---|---|
| Full dock/panel app shell | Yes | Yes | Partial (separate `daq-rerun` app shell) | Prototype only |
| Instrument manager | Yes | Yes | Simplified panel in `main_rerun.rs` | Prototype controls |
| Image viewer / frame display | Yes | Yes | Yes (Rerun-oriented path) | Prototype camera panel |
| Signal plotting | Yes | Yes | Simplified panel in `main_rerun.rs` + Rerun views | Prototype plot panel |
| Experiment designer | Yes (native-only panel) | **No** (WASM stub placeholder) | Not full parity | No |
| Comedi and broader device panels | Yes | Intended cross-platform where gRPC-safe | Not full parity | No |
| PVCAM + Rerun live workflow | Optional via features | Not target path | Primary reason to enable | No |
| PWA/browser tablet operation | No | Yes | No | Yes (eval path) |

### Practical exclusivity notes

- **Native-only:** `ExperimentDesignerPanel` is conditionally compiled out on wasm and replaced with a stub message.
- **Rerun-specific:** `daq-rerun` is a dedicated binary with simplified DAQ control panels and embedded Rerun viewer logic.
- **Slint:** currently an evaluation/prototype surface, not a replacement for the full operator panel set.

## 3) Maintenance cost indicators

### Relative implementation weight

- **egui (`crates/ui`) is the main maintenance center**:
  - ~60k LOC in `src`
  - high panel/widget count
  - multi-target feature gating (native + wasm + rerun + pvcam + storage)
- **ui-slint is comparatively lightweight**:
  - ~1.2k LOC in `src`
  - much smaller dependency graph
  - currently scoped to evaluation/demo patterns

### Dependency and build complexity signals

- `crates/ui` carries more integration burden:
  - eframe/egui ecosystem
  - gRPC + target-specific transport behavior
  - optional Rerun stack
  - native-only integrations and optional hardware/storage features
- `crates/ui-slint` has lower direct complexity but currently lower product coverage.

## 4) Known performance characteristics

- Native egui image streaming path includes FPS tracking (`max_fps` defaults to **30** in image viewer panel state).
- Historical Slint-vs-egui evaluation baseline (from prior project context):
  - egui: ~**163 FPS**
  - Slint: ~**560 FPS**
- Existing WASM artifact size indicates a non-trivial browser payload:
  - ~7.7 MiB wasm + ~87 KiB JS glue (current dist snapshot)

## 5) Operational constraints

### Native egui

- Best operator coverage and panel completeness.
- Desktop/runtime environment required.
- Strongest support for advanced/native-only workflows (notably experiment design).

### WASM egui

- Browser deployment and tablet-friendly path (including iPad usage in project context).
- Requires wasm build pipeline (`trunk`) and gRPC-web-compatible endpoint setup.
- Some native workflows are intentionally unavailable in browser (e.g., experiment designer).

### Rerun workflow

- Most useful for specialized camera/live-visualization pipelines (PVCAM-oriented integration).
- Additional cognitive and build surface: separate binary + Rerun runtime behavior.
- Not currently a full parity replacement for the standard operator app panel set.

### ui-slint eval

- Lower code/dependency weight and strong historical FPS upside.
- Still evaluation-grade in this repo: limited workflow coverage and no full panel parity.

## 6) Summary table (cost/benefit for maintainership decisions)

| Surface | Benefit | Cost | Best fit now |
|---|---|---|---|
| Native egui | Fullest operator feature coverage; established workflows | Highest maintenance footprint (~60k LOC, broad feature matrix) | Primary day-to-day operator UI |
| WASM egui | Remote/browser/tablet accessibility; shares most app architecture | Web build/deploy complexity; partial feature gaps (native-only panels unavailable) | Field/lab remote control and lightweight access |
| Rerun (`daq-rerun`) | Strong specialized live-visualization workflow (PVCAM/Rerun) | Separate binary + integration overhead; partial panel parity | Camera-centric diagnostics/visualization |
| `ui-slint` eval | Smaller codebase; favorable historical FPS signal | Evaluation scope only; missing full operator parity | R&D / future UI direction evaluation |

## Maintainer takeaway

Today, the project effectively carries **one primary production UI surface** (`crates/ui`, with native and WASM targets), plus one auxiliary specialized workflow (`daq-rerun`) and one evaluation surface (`ui-slint`).

- If the goal is broad operator capability and lowest migration risk: keep native egui as the anchor.
- If the goal is practical remote operation: maintain WASM as a constrained but high-value companion surface.
- If the goal is specialized camera visualization: retain Rerun as a targeted workflow, not a universal UI replacement.
- If considering UI stack migration: `ui-slint` shows potential performance upside but currently lacks coverage parity, so migration cost remains substantial.
