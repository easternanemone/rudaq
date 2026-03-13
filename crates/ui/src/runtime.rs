//! Platform-agnostic async runtime abstraction.
//!
//! On native: wraps `tokio::runtime::Runtime` (multi-threaded, full features).
//! On WASM: delegates to `wasm_bindgen_futures::spawn_local` (browser event loop).
//!
//! All GUI panels use `Runtime` from this module instead of importing tokio directly,
//! enabling the same panel code to work on both native desktop and browser targets.

// Re-export the platform-specific Runtime type so panels can use
// `use crate::runtime::Runtime;` regardless of target.

#[cfg(not(target_arch = "wasm32"))]
pub use tokio::runtime::Runtime;
#[cfg(not(target_arch = "wasm32"))]
pub use tokio::task::JoinHandle;

#[cfg(target_arch = "wasm32")]
mod wasm_runtime {
    /// Minimal async runtime for WASM that mirrors the `tokio::runtime::Runtime`
    /// API surface used by GUI panels (specifically: `spawn()`).
    ///
    /// On WASM, there is no multi-threaded runtime. All futures run on the
    /// browser's single-threaded event loop via `wasm_bindgen_futures::spawn_local`.
    pub struct Runtime;

    impl Runtime {
        /// Spawn a future onto the browser event loop.
        ///
        /// Unlike tokio's `spawn`, this does NOT require `Send` because WASM
        /// is single-threaded. The `tonic_web_wasm_client::Client` is `!Send`,
        /// so this relaxed bound is essential.
        ///
        /// Returns a dummy `JoinHandle` for API compatibility — the handle
        /// cannot be awaited (the future runs detached on the browser event loop).
        pub fn spawn<F, T>(&self, future: F) -> JoinHandle<T>
        where
            F: std::future::Future<Output = T> + 'static,
            T: 'static,
        {
            wasm_bindgen_futures::spawn_local(async move {
                // Run the future; discard the return value.
                // GUI code communicates results via channels, not return values.
                drop(future.await);
            });
            JoinHandle(std::marker::PhantomData)
        }
    }

    /// Dummy JoinHandle for API compatibility with tokio.
    ///
    /// On native, `runtime.spawn()` returns `tokio::task::JoinHandle<T>`.
    /// GUI code typically ignores the handle (fire-and-forget pattern),
    /// but some panels store it as an `Option<JoinHandle<()>>` to track
    /// whether a background task is running.
    ///
    /// **WASM limitation**: The return value `T` is dropped inside `spawn()`
    /// and cannot be retrieved. The type parameter exists solely for API
    /// compatibility with native code — do not attempt to `.await` this handle.
    /// GUI code should communicate results via channels, not return values.
    pub struct JoinHandle<T>(pub(crate) std::marker::PhantomData<T>);

    impl<T> JoinHandle<T> {
        /// No-op abort on WASM — spawned futures run detached on the browser
        /// event loop and cannot be cancelled.
        pub fn abort(&self) {}
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_runtime::{JoinHandle, Runtime};

// Cross-platform async sleep.
// On native: delegates to tokio::time::sleep (requires tokio time driver).
// On WASM: uses setTimeout via js_sys::Promise (browser event loop).

#[cfg(not(target_arch = "wasm32"))]
pub use tokio::time::sleep;

/// Async sleep for WASM — uses `setTimeout` under the hood.
#[cfg(target_arch = "wasm32")]
pub async fn sleep(duration: std::time::Duration) {
    let ms = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .expect("no global window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .expect("setTimeout failed");
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
