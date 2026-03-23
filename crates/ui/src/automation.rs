//! WASM GUI Automation API (`window.daqGui`)
//!
//! Provides programmatic control and state reading for browser automation tools
//! (e.g., claude-in-chrome MCP). The egui canvas renders to a single `<canvas>`
//! with zero DOM nodes, making coordinate-based clicks fragile. This module
//! exposes a JS API via `#[wasm_bindgen]` using a command queue + state snapshot
//! pattern:
//!
//! - JavaScript pushes commands (`connect`, `selectCamera`, `startStream`, …)
//! - `DaqApp::update()` drains the queue each frame and executes them
//! - `DaqApp::update()` writes a state snapshot each frame
//! - JavaScript reads the snapshot via `getStatus()`
//!
//! WASM is single-threaded → `Rc<RefCell<T>>` is safe, no `Send`/`Sync` needed.

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde::Serialize;
    use wasm_bindgen::prelude::*;

    // ── Command enum ─────────────────────────────────────────────

    /// Commands pushed by JavaScript, drained by `DaqApp::update()`.
    #[derive(Debug)]
    pub enum AutomationCommand {
        /// Connect to a daemon at the given URL.
        Connect(String),
        /// Disconnect from the current daemon.
        Disconnect,
        /// Select a camera by device ID (starts stream + loads params).
        SelectCamera(String),
        /// Start streaming from the currently selected camera.
        StartStream,
        /// Stop the active frame stream.
        StopStream,
        /// Set the spectrum view mode ("2D", "1D", "Split").
        SetViewMode(String),
        /// Set a camera parameter by name.
        SetParameter { name: String, value: String },
        /// Load + activate an echelle calibration profile (daemon-side path).
        ActivateProfile(String),
        /// Select an echelle order by index for the spectrum plot.
        SelectOrder(usize),
        /// Refresh the available cameras list from the daemon.
        RefreshCameras,
    }

    // ── State snapshot ───────────────────────────────────────────

    /// Snapshot of GUI state, serialized to JS via `serde-wasm-bindgen`.
    #[derive(Debug, Clone, Serialize, Default, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct AutomationState {
        pub connected: bool,
        pub streaming: bool,
        pub camera: Option<String>,
        pub frame_count: u64,
        pub fps: f32,
        pub width: u32,
        pub height: u32,
        pub view_mode: String,
        pub available_cameras: Vec<String>,
        pub parameters: Vec<ParameterInfo>,
        pub echelle_profile_loaded: bool,
        pub echelle_orders_count: usize,
        pub echelle_selected_order: usize,
        pub echelle_error: Option<String>,
        pub colormap: String,
    }

    /// Per-parameter info exposed to the JS automation API.
    #[derive(Debug, Clone, Serialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct ParameterInfo {
        pub name: String,
        pub value: String,
        #[serde(rename = "type")]
        pub param_type: String,
        pub read_only: bool,
    }

    // ── Bridge handles ───────────────────────────────────────────

    /// Shared command queue (JS pushes, `DaqApp` drains).
    pub type CommandQueue = Rc<RefCell<Vec<AutomationCommand>>>;

    /// Shared state holder (`DaqApp` writes, JS reads).
    pub type StateHolder = Rc<RefCell<AutomationState>>;

    thread_local! {
        static COMMAND_QUEUE: CommandQueue = Rc::new(RefCell::new(Vec::new()));
        static STATE_HOLDER: StateHolder = Rc::new(RefCell::new(AutomationState::default()));
        /// Stored egui Context for triggering repaints from JS API calls.
        /// Set on the first `DaqApp::update()` frame.
        static EGUI_CTX: RefCell<Option<eframe::egui::Context>> = const { RefCell::new(None) };
    }

    /// Get cloned `Rc` handles for the command queue and state holder.
    ///
    /// Called once by `DaqApp::new_wasm()` to store handles in app state,
    /// and once by `DaqGuiApi::new()` for the JS-side object. Both callers
    /// get handles to the *same* underlying data (single WASM thread).
    pub fn get_bridge_handles() -> (CommandQueue, StateHolder) {
        COMMAND_QUEUE.with(|q| STATE_HOLDER.with(|s| (q.clone(), s.clone())))
    }

    /// Store the egui Context so JS API calls can trigger repaints.
    ///
    /// Called from `DaqApp::update()` each frame (idempotent — just overwrites
    /// with a clone of the same `Arc`-backed context).
    pub fn set_egui_context(ctx: eframe::egui::Context) {
        EGUI_CTX.with(|cell| *cell.borrow_mut() = Some(ctx));
    }

    /// Request an egui repaint so that queued commands are processed promptly.
    ///
    /// Without this, eframe in WASM reactive mode only repaints on user input,
    /// leaving automation commands dormant until the next mouse/keyboard event.
    fn request_repaint() {
        EGUI_CTX.with(|cell| {
            if let Some(ctx) = cell.borrow().as_ref() {
                ctx.request_repaint();
            }
        });
    }

    // ── JS API class ─────────────────────────────────────────────

    /// The JS-visible API object installed at `window.daqGui`.
    ///
    /// All control methods are fire-and-forget (push to queue, executed next frame).
    /// State is read via `getStatus()` which returns the snapshot from the previous frame.
    #[wasm_bindgen(js_name = "DaqGuiApi")]
    pub struct DaqGuiApi {
        commands: CommandQueue,
        state: StateHolder,
    }

    impl Default for DaqGuiApi {
        fn default() -> Self {
            Self::new()
        }
    }

    #[wasm_bindgen(js_class = "DaqGuiApi")]
    impl DaqGuiApi {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Self {
            let (commands, state) = get_bridge_handles();
            Self { commands, state }
        }

        /// Push a command and wake the eframe render loop to process it.
        fn push_cmd(&self, cmd: AutomationCommand) {
            self.commands.borrow_mut().push(cmd);
            request_repaint();
        }

        /// Connect to a daemon URL (e.g. `"http://100.117.5.12:50051"`).
        pub fn connect(&self, url: &str) {
            self.push_cmd(AutomationCommand::Connect(url.to_string()));
        }

        /// Disconnect from the current daemon.
        pub fn disconnect(&self) {
            self.push_cmd(AutomationCommand::Disconnect);
        }

        /// Select a camera device by ID (e.g. `"istar_camera"`).
        #[wasm_bindgen(js_name = "selectCamera")]
        pub fn select_camera(&self, device_id: &str) {
            self.push_cmd(AutomationCommand::SelectCamera(device_id.to_string()));
        }

        /// Start streaming frames from the selected camera.
        #[wasm_bindgen(js_name = "startStream")]
        pub fn start_stream(&self) {
            self.push_cmd(AutomationCommand::StartStream);
        }

        /// Stop the active frame stream.
        #[wasm_bindgen(js_name = "stopStream")]
        pub fn stop_stream(&self) {
            self.push_cmd(AutomationCommand::StopStream);
        }

        /// Set view mode: `"2D"`, `"1D"`, or `"Split"`.
        #[wasm_bindgen(js_name = "setViewMode")]
        pub fn set_view_mode(&self, mode: &str) {
            self.push_cmd(AutomationCommand::SetViewMode(mode.to_string()));
        }

        /// Set a camera parameter by name and string value.
        #[wasm_bindgen(js_name = "setParameter")]
        pub fn set_parameter(&self, name: &str, value: &str) {
            self.push_cmd(AutomationCommand::SetParameter {
                name: name.to_string(),
                value: value.to_string(),
            });
        }

        /// Load and activate an echelle calibration profile from a daemon-side path.
        #[wasm_bindgen(js_name = "activateProfile")]
        pub fn activate_profile(&self, path: &str) {
            self.push_cmd(AutomationCommand::ActivateProfile(path.to_string()));
        }

        /// Select an echelle order by index for the spectrum plot.
        #[wasm_bindgen(js_name = "selectOrder")]
        pub fn select_order(&self, index: usize) {
            self.push_cmd(AutomationCommand::SelectOrder(index));
        }

        /// Refresh the list of available cameras from the daemon.
        #[wasm_bindgen(js_name = "refreshCameras")]
        pub fn refresh_cameras(&self) {
            self.push_cmd(AutomationCommand::RefreshCameras);
        }

        /// Get the current GUI state as a JS object.
        ///
        /// Returns an object with fields: `connected`, `streaming`, `camera`,
        /// `frameCount`, `fps`, `width`, `height`, `viewMode`, `availableCameras`,
        /// `parameters`, `echelleProfileLoaded`, `echelleOrdersCount`,
        /// `echelleSelectedOrder`, `echelleError`, `colormap`.
        #[wasm_bindgen(js_name = "getStatus")]
        pub fn get_status(&self) -> JsValue {
            let state = self.state.borrow();
            serde_wasm_bindgen::to_value(&*state).unwrap_or(JsValue::NULL)
        }
    }

    /// Install the automation API on `window.daqGui`.
    ///
    /// Must be called before `eframe::WebRunner::start()` so that automation
    /// scripts can issue commands as soon as the app initializes.
    pub fn install_automation_api() {
        let api = DaqGuiApi::new();
        let js_api: JsValue = api.into();
        let window = web_sys::window().expect("no global window");
        js_sys::Reflect::set(&window, &JsValue::from_str("daqGui"), &js_api)
            .expect("failed to set window.daqGui");
        tracing::info!("Automation API installed at window.daqGui");
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_impl::*;
