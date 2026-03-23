//! Ghost DOM overlay for AI agent discoverability (bd-dmk8).
//!
//! Creates hidden HTML elements positioned over the egui canvas that mirror
//! application state. Browser automation tools (Playwright, claude-in-chrome)
//! can discover and query these elements via standard selectors:
//!
//! ```js
//! page.getByRole('button', { name: 'Connect' })
//! page.locator('[data-widget-id="camera_istar_camera"]')
//! page.locator('#ghost-dom-root [data-widget-type]').all()
//! ```
//!
//! This complements the `window.daqGui` JS API (which requires pre-knowledge
//! of command names) by making the UI self-discoverable.
//!
//! ## Design
//!
//! - **State-driven**: Syncs from [`AutomationState`], not from individual widget calls
//! - **Dirty-flag writes**: Only touches the DOM when values actually change
//! - **Invisible overlay**: `opacity: 0; pointer-events: none` — users see nothing,
//!   but automation tools can query via `getByRole`, `getByLabel`, `data-*` attributes
//! - **PixiJS-inspired**: Same pattern as PixiJS `AccessibilityManager` (production-proven)

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use std::collections::HashMap;

    use wasm_bindgen::JsCast;

    use crate::automation::AutomationState;

    /// Widget type constants for `data-widget-type` attributes.
    /// Single source of truth — a typo here is a compile-time constant,
    /// not a silent selector mismatch at runtime.
    mod widget_type {
        pub const STATUS: &str = "status";
        pub const BUTTON: &str = "button";
        pub const DATA: &str = "data";
        pub const CAMERA_LIST: &str = "camera-list";
        pub const CAMERA_OPTION: &str = "camera-option";
        pub const VIEW_MODE: &str = "view-mode";
        pub const VIEW_MODE_OPTION: &str = "view-mode-option";
        pub const PARAMETER: &str = "parameter";
        pub const ECHELLE: &str = "echelle";
        pub const ECHELLE_ORDER: &str = "echelle-order";
    }

    /// Node ID prefixes for namespaced ghost elements.
    const PREFIX_CAMERA: &str = "camera_";
    const PREFIX_PARAM: &str = "param_";
    const PREFIX_VIEW_MODE: &str = "view_mode_";

    /// Cached state for dirty-flag comparison.
    ///
    /// On the first frame, `initialized` is `false` — forcing a full sync
    /// even when all values match their defaults (e.g., `connected: false`).
    /// Without this, baseline ghost elements would never be created.
    #[derive(Default)]
    struct CachedState {
        initialized: bool,
        connected: bool,
        streaming: bool,
        camera: Option<String>,
        frame_count: u64,
        fps_bucket: u32,
        view_mode: String,
        available_cameras: Vec<String>,
        param_values: Vec<(String, String)>,
        echelle_profile_loaded: bool,
        echelle_orders_count: usize,
        echelle_selected_order: usize,
        echelle_error: Option<String>,
    }

    /// Hidden DOM overlay that mirrors egui state for browser automation discovery.
    pub struct GhostDom {
        root: web_sys::HtmlElement,
        live_region: web_sys::Element,
        nodes: HashMap<String, web_sys::Element>,
        cached: CachedState,
    }

    impl GhostDom {
        /// Create the Ghost DOM overlay as a sibling of the canvas element.
        ///
        /// The overlay is absolutely positioned over the canvas with `pointer-events: none`
        /// and `opacity: 0`, making it invisible to users but discoverable by automation.
        pub fn new() -> Result<Self, wasm_bindgen::JsValue> {
            let document = web_sys::window()
                .expect("no window")
                .document()
                .expect("no document");

            let root: web_sys::HtmlElement = document.create_element("div")?.dyn_into()?;
            root.set_id("ghost-dom-root");
            root.set_attribute("role", "complementary")?;
            root.set_attribute("aria-label", "DAQ GUI Automation Overlay")?;

            let style = root.style();
            style.set_property("position", "fixed")?;
            style.set_property("top", "0")?;
            style.set_property("left", "0")?;
            style.set_property("width", "100%")?;
            style.set_property("height", "100%")?;
            style.set_property("pointer-events", "none")?;
            style.set_property("overflow", "hidden")?;
            style.set_property("z-index", "1000")?;
            style.set_property("opacity", "0")?;

            document.body().expect("no body").append_child(&root)?;

            let live_region = document.create_element("div")?;
            live_region.set_id("ghost-live-region");
            live_region.set_attribute("role", "log")?;
            live_region.set_attribute("aria-live", "polite")?;
            live_region.set_attribute("aria-atomic", "false")?;
            live_region.set_attribute("aria-label", "DAQ status events")?;
            root.append_child(&live_region)?;

            tracing::info!("Ghost DOM overlay installed at #ghost-dom-root");

            Ok(Self {
                root,
                live_region,
                nodes: HashMap::new(),
                cached: CachedState::default(),
            })
        }

        /// Sync the Ghost DOM to match the current automation state.
        ///
        /// Uses dirty-flag comparisons: only touches the DOM when values change.
        /// Designed to be called every frame with negligible overhead when state is stable.
        pub fn sync(&mut self, state: &AutomationState) {
            let first = !self.cached.initialized;
            if first {
                self.cached.initialized = true;
            }

            // Connection status
            if first || self.cached.connected != state.connected {
                self.cached.connected = state.connected;
                let status_text = if state.connected {
                    "Connected"
                } else {
                    "Disconnected"
                };
                self.upsert_status("connection_status", "Connection Status", status_text);
                self.update_button("btn_connect", "Connect", !state.connected);
                self.update_button("btn_disconnect", "Disconnect", state.connected);
                if !first {
                    self.announce(status_text);
                }
            }

            // Streaming status
            if first || self.cached.streaming != state.streaming {
                self.cached.streaming = state.streaming;
                let status_text = if state.streaming {
                    "Streaming"
                } else {
                    "Stopped"
                };
                self.upsert_status("streaming_status", "Streaming Status", status_text);
                self.update_button("btn_start_stream", "Start Stream", !state.streaming);
                self.update_button("btn_stop_stream", "Stop Stream", state.streaming);
            }

            // Selected camera — also triggers camera list aria-selected update
            let camera_changed = first || self.cached.camera != state.camera;
            if camera_changed {
                self.cached.camera.clone_from(&state.camera);
                let display = state.camera.as_deref().unwrap_or("None");
                self.upsert_status("selected_camera", "Selected Camera", display);
            }

            // Available cameras list (also resync on selection change for aria-selected)
            if first || self.cached.available_cameras != state.available_cameras || camera_changed {
                self.cached
                    .available_cameras
                    .clone_from(&state.available_cameras);
                self.sync_camera_list(&state.available_cameras);
            }

            // Frame counter — only update every 10 frames to avoid thrashing
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let fps_bucket = (state.fps.max(0.0) * 10.0) as u32;
            if first
                || self.cached.frame_count / 10 != state.frame_count / 10
                || self.cached.fps_bucket != fps_bucket
            {
                self.cached.frame_count = state.frame_count;
                self.cached.fps_bucket = fps_bucket;
                self.upsert_data(
                    "frame_stats",
                    "Frame Statistics",
                    &[
                        ("data-frame-count", &state.frame_count.to_string()),
                        ("data-fps", &format!("{:.1}", state.fps)),
                        ("data-width", &state.width.to_string()),
                        ("data-height", &state.height.to_string()),
                    ],
                );
            }

            // View mode
            if first || self.cached.view_mode != state.view_mode {
                self.cached.view_mode.clone_from(&state.view_mode);
                self.sync_view_mode(&state.view_mode);
            }

            // Parameters — zero-allocation comparison via zip
            let params_changed = first
                || self.cached.param_values.len() != state.parameters.len()
                || self
                    .cached
                    .param_values
                    .iter()
                    .zip(state.parameters.iter())
                    .any(|((cached_name, cached_value), p)| {
                        cached_name != &p.name || cached_value != &p.value
                    });
            if params_changed {
                self.cached.param_values.clear();
                self.cached.param_values.extend(
                    state
                        .parameters
                        .iter()
                        .map(|p| (p.name.clone(), p.value.clone())),
                );
                self.sync_parameters(&state.parameters);
            }

            // Echelle state
            if first
                || self.cached.echelle_profile_loaded != state.echelle_profile_loaded
                || self.cached.echelle_orders_count != state.echelle_orders_count
                || self.cached.echelle_selected_order != state.echelle_selected_order
                || self.cached.echelle_error != state.echelle_error
            {
                self.cached.echelle_profile_loaded = state.echelle_profile_loaded;
                self.cached.echelle_orders_count = state.echelle_orders_count;
                self.cached.echelle_selected_order = state.echelle_selected_order;
                self.cached.echelle_error.clone_from(&state.echelle_error);
                self.sync_echelle(state);
            }
        }

        // ── Private helpers ────────────────────────────────────────────

        /// Get or create an element by widget ID.
        fn get_or_create(&mut self, id: &str, tag: &str) -> web_sys::Element {
            if let Some(el) = self.nodes.get(id) {
                return el.clone();
            }
            let document = web_sys::window().unwrap().document().unwrap();
            let el = document.create_element(tag).unwrap();
            el.set_attribute("data-widget-id", id).unwrap();
            if let Ok(html_el) = el.clone().dyn_into::<web_sys::HtmlElement>() {
                let style = html_el.style();
                let _ = style.set_property("position", "absolute");
                let _ = style.set_property("pointer-events", "none");
            }
            self.root.append_child(&el).unwrap();
            self.nodes.insert(id.to_string(), el.clone());
            el
        }

        /// Remove a ghost element by widget ID.
        fn remove_node(&mut self, id: &str) {
            if let Some(el) = self.nodes.remove(id) {
                let _ = self.root.remove_child(&el);
            }
        }

        /// Remove ghost nodes whose IDs start with `prefix` but don't match
        /// any suffix in `valid_suffixes`. Uses zero-allocation slice comparison
        /// instead of `format!()` per-key.
        fn remove_stale_nodes(&mut self, prefix: &str, valid_suffixes: &[impl AsRef<str>]) {
            let stale_keys: Vec<String> = self
                .nodes
                .keys()
                .filter(|k| {
                    k.starts_with(prefix)
                        && !valid_suffixes.iter().any(|s| {
                            k.len() == prefix.len() + s.as_ref().len()
                                && k[prefix.len()..] == *s.as_ref()
                        })
                })
                .cloned()
                .collect();
            for key in stale_keys {
                self.remove_node(&key);
            }
        }

        /// Create/update a status indicator (`role="status"`).
        fn upsert_status(&mut self, id: &str, label: &str, value: &str) {
            let el = self.get_or_create(id, "div");
            let _ = el.set_attribute("role", "status");
            let _ = el.set_attribute("aria-label", label);
            let _ = el.set_attribute("data-widget-type", widget_type::STATUS);
            let _ = el.set_attribute("data-value", value);
            el.set_text_content(Some(&format!("{label}: {value}")));
        }

        /// Create/update a button element.
        fn update_button(&mut self, id: &str, label: &str, enabled: bool) {
            let el = self.get_or_create(id, "button");
            let _ = el.set_attribute("aria-label", label);
            let _ = el.set_attribute("data-widget-type", widget_type::BUTTON);
            let _ = el.set_attribute("tabindex", "-1");
            if enabled {
                let _ = el.remove_attribute("aria-disabled");
                let _ = el.set_attribute("data-widget-state", "enabled");
            } else {
                let _ = el.set_attribute("aria-disabled", "true");
                let _ = el.set_attribute("data-widget-state", "disabled");
            }
            el.set_text_content(Some(label));
        }

        /// Create/update an element with multiple data attributes.
        fn upsert_data(&mut self, id: &str, label: &str, attrs: &[(&str, &str)]) {
            let el = self.get_or_create(id, "div");
            let _ = el.set_attribute("aria-label", label);
            let _ = el.set_attribute("data-widget-type", widget_type::DATA);
            for (key, value) in attrs {
                let _ = el.set_attribute(key, value);
            }
        }

        /// Sync the available cameras as a listbox with option elements.
        fn sync_camera_list(&mut self, cameras: &[String]) {
            self.remove_stale_nodes(PREFIX_CAMERA, cameras);

            let list = self.get_or_create("camera_list", "div");
            let _ = list.set_attribute("role", "listbox");
            let _ = list.set_attribute("aria-label", "Available Cameras");
            let _ = list.set_attribute("data-widget-type", widget_type::CAMERA_LIST);
            let _ = list.set_attribute("data-count", &cameras.len().to_string());

            for camera_id in cameras {
                let node_id = format!("{PREFIX_CAMERA}{camera_id}");
                let el = self.get_or_create(&node_id, "div");
                let _ = el.set_attribute("role", "option");
                let _ = el.set_attribute("aria-label", camera_id);
                let _ = el.set_attribute("data-widget-type", widget_type::CAMERA_OPTION);
                let _ = el.set_attribute("data-device-id", camera_id);

                let is_selected = self.cached.camera.as_deref() == Some(camera_id.as_str());
                let _ =
                    el.set_attribute("aria-selected", if is_selected { "true" } else { "false" });
                el.set_text_content(Some(camera_id));
            }
        }

        /// Sync view mode radio group.
        fn sync_view_mode(&mut self, current: &str) {
            let group = self.get_or_create("view_mode_group", "div");
            let _ = group.set_attribute("role", "radiogroup");
            let _ = group.set_attribute("aria-label", "View Mode");
            let _ = group.set_attribute("data-widget-type", widget_type::VIEW_MODE);
            let _ = group.set_attribute("data-value", current);

            for mode in &["2D", "1D", "Split"] {
                let id = format!("{PREFIX_VIEW_MODE}{mode}");
                let el = self.get_or_create(&id, "div");
                let _ = el.set_attribute("role", "radio");
                let _ = el.set_attribute("aria-label", &format!("View Mode: {mode}"));
                let _ = el.set_attribute("data-widget-type", widget_type::VIEW_MODE_OPTION);
                let _ = el.set_attribute("data-value", mode);
                let is_checked = *mode == current;
                let _ = el.set_attribute("aria-checked", if is_checked { "true" } else { "false" });
                el.set_text_content(Some(mode));
            }
        }

        /// Sync camera parameters as labeled data elements.
        fn sync_parameters(&mut self, params: &[crate::automation::ParameterInfo]) {
            let param_names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
            self.remove_stale_nodes(PREFIX_PARAM, &param_names);

            for param in params {
                let id = format!("{PREFIX_PARAM}{}", param.name);
                let el = self.get_or_create(&id, "div");

                let role = match param.param_type.as_str() {
                    "Int" | "Float" | "Enumerated" => "slider",
                    _ => "textbox",
                };
                let _ = el.set_attribute("role", role);
                let _ = el.set_attribute("aria-label", &param.name);
                let _ = el.set_attribute("data-widget-type", widget_type::PARAMETER);
                let _ = el.set_attribute("data-param-name", &param.name);
                let _ = el.set_attribute("data-value", &param.value);
                let _ = el.set_attribute("data-param-type", &param.param_type);
                let _ = el.set_attribute(
                    "aria-readonly",
                    if param.read_only { "true" } else { "false" },
                );
                if role == "slider" {
                    let _ = el.set_attribute("aria-valuenow", &param.value);
                    let _ = el.set_attribute("aria-valuetext", &param.value);
                }
                el.set_text_content(Some(&format!("{}: {}", param.name, param.value)));
            }
        }

        /// Sync echelle spectroscopy state.
        fn sync_echelle(&mut self, state: &AutomationState) {
            let el = self.get_or_create("echelle_state", "div");
            let _ = el.set_attribute("aria-label", "Echelle Spectrometer");
            let _ = el.set_attribute("data-widget-type", widget_type::ECHELLE);
            let _ = el.set_attribute(
                "data-profile-loaded",
                &state.echelle_profile_loaded.to_string(),
            );
            let _ = el.set_attribute("data-orders-count", &state.echelle_orders_count.to_string());
            let _ = el.set_attribute(
                "data-selected-order",
                &state.echelle_selected_order.to_string(),
            );
            if let Some(ref err) = state.echelle_error {
                let _ = el.set_attribute("data-error", err);
            } else {
                let _ = el.remove_attribute("data-error");
            }

            if state.echelle_profile_loaded && state.echelle_orders_count > 0 {
                let selector = self.get_or_create("echelle_order_selector", "div");
                let _ = selector.set_attribute("role", "spinbutton");
                let _ = selector.set_attribute("aria-label", "Echelle Order");
                let _ = selector.set_attribute("data-widget-type", widget_type::ECHELLE_ORDER);
                let _ = selector
                    .set_attribute("aria-valuenow", &state.echelle_selected_order.to_string());
                let _ = selector.set_attribute("aria-valuemin", "0");
                let _ = selector.set_attribute(
                    "aria-valuemax",
                    &state.echelle_orders_count.saturating_sub(1).to_string(),
                );
            } else {
                self.remove_node("echelle_order_selector");
            }
        }

        /// Post a message to the aria-live region for automation event listeners.
        fn announce(&self, message: &str) {
            // Clear then set to ensure mutation observers fire
            self.live_region.set_text_content(Some(""));
            self.live_region.set_text_content(Some(message));
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_impl::GhostDom;
