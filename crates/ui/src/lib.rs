// Platform-agnostic async runtime abstraction (native tokio / WASM spawn_local)
pub mod runtime;

// These modules use native deps (clap, tokio runtime, process spawning, etc.)
// and are not available on WASM targets.
#[cfg(not(target_arch = "wasm32"))]
pub mod client;
#[cfg(not(target_arch = "wasm32"))]
pub mod connection;
#[cfg(not(target_arch = "wasm32"))]
pub mod daemon_launcher;
#[cfg(not(target_arch = "wasm32"))]
pub mod log_capture;
#[cfg(not(target_arch = "wasm32"))]
pub mod reconnect;

#[cfg(feature = "standalone")]
pub mod gui_log_layer;

#[cfg(feature = "standalone")]
pub mod connection_state_ext;
#[cfg(feature = "standalone")]
pub use connection_state_ext::ConnectionStateExt;

#[cfg(feature = "standalone")]
pub(crate) mod device_ext;

#[cfg(feature = "standalone")]
pub mod app;
#[cfg(feature = "standalone")]
pub mod export;
#[cfg(feature = "standalone")]
pub mod graph;
#[cfg(feature = "standalone")]
pub mod gui_config;
#[cfg(feature = "standalone")]
pub mod icons;
#[cfg(feature = "standalone")]
pub mod layout;
#[cfg(feature = "standalone")]
pub mod panels;
#[cfg(feature = "standalone")]
pub mod settings;
#[cfg(feature = "standalone")]
pub mod shortcuts;
#[cfg(feature = "standalone")]
pub mod theme;
#[cfg(feature = "standalone")]
pub mod widgets;
