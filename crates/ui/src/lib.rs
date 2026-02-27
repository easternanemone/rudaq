// Platform-agnostic async runtime abstraction (native tokio / WASM spawn_local)
pub mod runtime;

// Cross-platform time types (std::time on native, web-time on WASM)
pub mod time;

// Serde-only schema types for the WASM web build.
// Included during native test runs (via `test` cfg) so the deserialization
// tests in web_schema.rs are exercised without needing a WASM target.
#[cfg(any(feature = "web", test))]
pub mod web_schema;

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

// gui_log_layer depends on tracing-subscriber (native-only)
#[cfg(not(target_arch = "wasm32"))]
pub mod gui_log_layer;

// connection_state_ext depends on client::reconnect::ConnectionState (native-only)
#[cfg(not(target_arch = "wasm32"))]
pub mod connection_state_ext;
#[cfg(not(target_arch = "wasm32"))]
pub use connection_state_ext::ConnectionStateExt;

#[cfg(any(feature = "standalone", feature = "web"))]
pub(crate) mod device_ext;

#[cfg(any(feature = "standalone", feature = "web"))]
pub mod app;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod export;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod graph;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod gui_config;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod icons;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod layout;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod panels;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod settings;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod shortcuts;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod theme;
#[cfg(any(feature = "standalone", feature = "web"))]
pub mod widgets;
