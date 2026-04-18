# WASM DOM Interop

The WASM build uses `web-sys` + `wasm-bindgen` for browser API access.

**Currently enabled features** (in `crates/ui/Cargo.toml`): `Window`, `Document`, `HtmlCanvasElement`.

## Adding web-sys Features

Add to `crates/ui/Cargo.toml` under `[target.'cfg(target_arch = "wasm32")'.dependencies]`:

| Feature | API | Use Case |
|---------|-----|----------|
| `Location` | `window().location().search()`, `.set_href()` | Read URL params, redirects |
| `Storage` | `window().local_storage()`, `.get_item()`, `.set_item()` | Persist settings across page loads |
| `UrlSearchParams` | `UrlSearchParams::new_with_str(&search).get(name)` | Parse URL query params |
| `HtmlElement` | `element.dyn_into::<HtmlElement>().set_inner_text()` | Modify DOM outside canvas |

## Verified Patterns

```rust
// Read URL query param (requires: Location, UrlSearchParams features)
#[cfg(target_arch = "wasm32")]
pub fn get_url_param(name: &str) -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params.get(name)
}

// localStorage get/set (requires: Storage feature)
#[cfg(target_arch = "wasm32")]
pub fn local_storage_get(key: &str) -> Option<String> {
    let storage = web_sys::window()?.local_storage().ok()??;
    storage.get_item(key).ok()?
}

// Update browser tab title (works with existing features, no changes needed)
#[cfg(target_arch = "wasm32")]
pub fn set_page_title(title: &str) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        doc.set_title(title);
    }
}
```

All patterns verified to compile for `wasm32-unknown-unknown` (tested March 2026).

## Constraints

- All `web-sys` calls are main-thread only (WASM is single-threaded in browser)
- Use `#[cfg(target_arch = "wasm32")]` guards — these APIs don't exist on native builds
- Use `wasm_bindgen::JsCast` for `.dyn_into::<T>()` downcasts (`Element` → `HtmlElement`)
- Keep the `web-sys` feature list minimal — each feature increases WASM binary size

## Practical Applications for rust-daq

- **URL-based daemon selection**: `?daemon=http://100.117.5.12:50051` — bookmarkable daemon URLs (fixes reconnect bug bd-0zu5)
- **Settings persistence**: Save last daemon URL, panel layout, calibration display preferences to `localStorage`
- **Tab title**: Show "DAQ Panel — Connected (maitai)" or "DAQ Panel — DISCONNECTED" in browser tab

## Build

```bash
# Manual WASM build (trunk required)
cd crates/ui && trunk build --release
python3 -m http.server 8080   # Serve dist/

# Via deploy scripts (trunk auto-installed if missing)
bash scripts/deploy/deploy-leabs.sh --wasm-gui
```
